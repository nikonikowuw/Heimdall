//! 六步安全沙箱自检器与平台拓扑感知体系
//! 支持物理子进程隔离执行自检，坚决防范段错误（SIGSEGV）带崩主进程。

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::c_abi::loader::{check_c_status, LoadedLib, RawAlgoLibrary};
use crate::c_abi::types::*;
use crate::error::InferError;
use crate::package::ALGO_MANIFEST_FILENAME;

/// 算法沙箱物理隔离子进程私有 CLI 自测标志参数
pub const VERIFY_ALGO_ARG: &str = "__verify-algo";

/// 获取当前编译运行环境的标准精炼平台代号
pub fn current_platform_id() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos-arm64"
    } else if cfg!(all(target_os = "linux", feature = "backend-rknn")) {
        "linux-rknn"
    } else if cfg!(all(target_os = "linux", feature = "backend-ascend")) {
        "linux-ascend"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-x64"
    } else {
        "macos-arm64" // 开发机默认兜底
    }
}

/// 归一化平台代号，兼容历史冗长别名
pub fn normalize_platform_id(id: &str) -> &str {
    match id {
        "macos-arm64" | "macos-arm64-coreml" | "darwin-arm64" => "macos-arm64",
        "linux-rknn" | "linux-arm64-rknn" | "rknn" => "linux-rknn",
        "linux-ascend" | "linux-arm64-ascend" | "ascend" => "linux-ascend",
        "linux-x64" | "generic-x86_64-cpu" | "linux-x86_64" => "linux-x64",
        other => other,
    }
}

/// 算法包 manifest.json 元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlgoManifest {
    pub manifest_version: u32,
    pub algorithm_id: String,
    pub version: String,
    pub name: String,
    pub description: Option<String>,
    pub algorithm_type: String,
    pub alarm_type_id: String,
    pub platform_id: String,
    #[serde(default)]
    pub min_adapter_version: Option<String>,
}

impl AlgoManifest {
    /// 校验 Manifest 各字段的合法性，严格防御路径穿越与畸形数据注入
    pub fn validate(&self) -> Result<(), InferError> {
        let step = "2.解析 Manifest 与平台匹配";
        if self.manifest_version != 1 {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!("不支持的 manifest_version: {}", self.manifest_version),
            });
        }

        Self::validate_algorithm_id(&self.algorithm_id, step)?;
        Self::validate_version(&self.version, step)?;
        Self::validate_name(&self.name, step)?;
        Self::validate_identifier("algorithm_type", &self.algorithm_type, 31, step)?;
        Self::validate_identifier("alarm_type_id", &self.alarm_type_id, 63, step)?;
        Self::validate_platform_id(&self.platform_id, step)?;

        Ok(())
    }

    /// 校验 algorithm_id
    pub fn validate_algorithm_id(id: &str, step: &str) -> Result<(), InferError> {
        Self::validate_identifier("algorithm_id", id, 63, step)
    }

    /// 校验 version
    pub fn validate_version(ver: &str, step: &str) -> Result<(), InferError> {
        if ver.is_empty() {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: "Manifest 中 'version' 不能为空".to_string(),
            });
        }
        if ver.len() > 31 {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!(
                    "Manifest 中 'version' 长度超过 C ABI 上限 (最大 31 字符, 实际 {} 字符)",
                    ver.len()
                ),
            });
        }
        // 严格防路径穿越，禁止包含 ..、/、\、以及单纯由点构成的路径组件
        if ver.contains("..") || ver.contains('/') || ver.contains('\\') || ver == "." {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!("Manifest 中 'version' [{ver}] 包含路径穿越组件"),
            });
        }
        // 仅允许标准 SemVer 字符集: ASCII 字母、数字、点、短横线、下划线、加号
        if !ver
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_' || b == b'+')
        {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!("Manifest 中 'version' [{ver}] 包含非法字符"),
            });
        }
        Ok(())
    }

    /// 校验 name 字段
    pub fn validate_name(name: &str, step: &str) -> Result<(), InferError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: "Manifest 中 'name' 字段不能为空".to_string(),
            });
        }
        if trimmed.contains('\0') {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: "Manifest 中 'name' 字段包含非法空字符 (Null Byte)".to_string(),
            });
        }
        if trimmed.len() > 128 {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!(
                    "Manifest 中 'name' 字段过长 (最大 128 字符, 实际 {} 字符)",
                    trimmed.len()
                ),
            });
        }
        Ok(())
    }

    /// 校验普通标识符 (algorithm_type / alarm_type_id)
    pub fn validate_identifier(
        field: &str,
        val: &str,
        max_len: usize,
        step: &str,
    ) -> Result<(), InferError> {
        if val.is_empty() {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!("Manifest 中 '{field}' 不能为空"),
            });
        }
        if val.len() > max_len {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!(
                    "Manifest 中 '{field}' 长度超过 C ABI 上限 (最大 {max_len} 字符, 实际 {} 字符)",
                    val.len()
                ),
            });
        }
        if !val
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!("Manifest 中 '{field}' [{val}] 包含非法字符或路径穿越组件"),
            });
        }
        Ok(())
    }

    /// 校验 platform_id
    pub fn validate_platform_id(id: &str, step: &str) -> Result<(), InferError> {
        if id.is_empty() {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: "Manifest 中 'platform_id' 不能为空".to_string(),
            });
        }
        if id.len() > 64 {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!(
                    "Manifest 中 'platform_id' 长度过长 (最大 64 字符, 实际 {} 字符)",
                    id.len()
                ),
            });
        }
        if id.contains("..") || id.contains('/') || id.contains('\\') || id.contains('\0') {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!("Manifest 中 'platform_id' [{id}] 包含路径穿越组件"),
            });
        }
        Ok(())
    }
}

/// 算法包沙箱自检结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfTestReport {
    pub status: String,
    pub algorithm_id: String,
    pub version: String,
    pub detections_count: usize,
    pub duration_ms: u64,
}

/// 六步沙箱检查的可观测状态。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SandboxStepStatus {
    Running,
    Passed,
    Failed,
}

/// 六步沙箱检查进度事件。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxProgressEvent {
    pub step: usize,
    pub status: SandboxStepStatus,
}

impl SandboxProgressEvent {
    pub const fn running(step: usize) -> Self {
        Self {
            step,
            status: SandboxStepStatus::Running,
        }
    }

    pub const fn passed(step: usize) -> Self {
        Self {
            step,
            status: SandboxStepStatus::Passed,
        }
    }
}

/// 沙箱子进程 stdout 上的行协议。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "camelCase")]
pub enum SandboxChildMessage {
    Progress(SandboxProgressEvent),
    Report(SelfTestReport),
}

const MAX_SANDBOX_STDOUT_LINE_BYTES: usize = 64 * 1024;
const MAX_SANDBOX_STDERR_BYTES: usize = 64 * 1024;
const SANDBOX_OUTPUT_DRAIN_TIMEOUT_MS: u64 = 250;

type SandboxReportSlot = Arc<Mutex<Option<SelfTestReport>>>;

#[derive(Debug, Default)]
struct CappedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

impl CappedOutput {
    fn append(&mut self, chunk: &[u8]) {
        let remaining = MAX_SANDBOX_STDERR_BYTES.saturating_sub(self.bytes.len());
        let captured = chunk.len().min(remaining);
        self.bytes.extend_from_slice(&chunk[..captured]);
        self.truncated |= captured < chunk.len();
    }

    fn snapshot(&self) -> String {
        let mut message = String::from_utf8_lossy(&self.bytes).into_owned();
        if self.truncated {
            message.push_str(" [stderr truncated at 64 KiB]");
        }
        message
    }
}

fn try_send_sandbox_protocol_line(
    tx: &SyncSender<SandboxProgressEvent>,
    report_slot: &SandboxReportSlot,
    line: &[u8],
) -> bool {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let Ok(message) = serde_json::from_slice::<SandboxChildMessage>(line) else {
        return true;
    };

    match message {
        SandboxChildMessage::Report(report) => {
            if let Ok(mut slot) = report_slot.lock() {
                *slot = Some(report);
            }
            true
        }
        SandboxChildMessage::Progress(progress) => match tx.try_send(progress) {
            Ok(()) | Err(TrySendError::Full(_)) => true,
            Err(TrySendError::Disconnected(_)) => false,
        },
    }
}

fn forward_sandbox_line(
    tx: &SyncSender<SandboxProgressEvent>,
    report_slot: &SandboxReportSlot,
    line: &[u8],
    truncated: bool,
) -> bool {
    !truncated && try_send_sandbox_protocol_line(tx, report_slot, line)
}

/// 只把合法协议行投递给父线程。
///
/// 算法库与底层 SDK 可能向 stdout 输出普通日志，因此不能把每一行原文放入
/// 有界通道：普通日志应被丢弃，进度消息使用非阻塞投递，避免 reader 反向
/// 阻塞子进程退出。最终报告放入单槽位，不能因进度洪泛而被丢弃。单行也必须
/// 有上限，防止无换行的日志占满内存。
fn spawn_sandbox_stdout_reader(
    stdout: impl Read + Send + 'static,
    tx: SyncSender<SandboxProgressEvent>,
    report_slot: SandboxReportSlot,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::with_capacity(256);
        let mut truncated = false;

        while let Ok(buffer) = reader.fill_buf() {
            if buffer.is_empty() {
                if !line.is_empty() && !forward_sandbox_line(&tx, &report_slot, &line, truncated) {
                    return;
                }
                break;
            }

            let mut consumed = 0;
            for &byte in buffer {
                consumed += 1;
                if byte == b'\n' {
                    if !forward_sandbox_line(&tx, &report_slot, &line, truncated) {
                        return;
                    }
                    line.clear();
                    truncated = false;
                } else if !truncated {
                    if line.len() < MAX_SANDBOX_STDOUT_LINE_BYTES {
                        line.push(byte);
                    } else {
                        truncated = true;
                    }
                }
            }
            reader.consume(consumed);
        }
    })
}

fn spawn_sandbox_stderr_reader(
    stderr: impl Read + Send + 'static,
    output: Arc<Mutex<CappedOutput>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut chunk = [0u8; 4096];
        while let Ok(read) = reader.read(&mut chunk) {
            if read == 0 {
                break;
            }
            if let Ok(mut captured) = output.lock() {
                captured.append(&chunk[..read]);
            }
        }
    })
}

/// 算法包安全沙箱自检器
#[derive(Debug, Default)]
pub struct AlgoSandbox;

impl AlgoSandbox {
    /// 对指定算法包目录执行六步沙箱安全校验
    ///
    /// `use_subprocess`: 是否启用物理子进程隔离自检（生产和上传时必须为 true）
    pub fn validate_package(
        package_dir: &Path,
        use_subprocess: bool,
    ) -> Result<AlgoManifest, InferError> {
        Self::validate_package_with_progress(package_dir, use_subprocess, |_| {})
    }

    /// 执行六步沙箱校验并在真实检查边界发出进度事件。
    pub fn validate_package_with_progress<F>(
        package_dir: &Path,
        use_subprocess: bool,
        mut on_progress: F,
    ) -> Result<AlgoManifest, InferError>
    where
        F: FnMut(SandboxProgressEvent),
    {
        let step = "1.路径防穿透与结构检查";
        on_progress(SandboxProgressEvent::running(1));
        if package_dir.as_os_str().is_empty() {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: "算法包路径不能为空".to_string(),
            });
        }

        if package_dir.to_string_lossy().contains('\0') {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: "算法包路径包含非法空字符 (Null Byte)".to_string(),
            });
        }

        if !package_dir.exists() || !package_dir.is_dir() {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!("算法包目录不存在: {:?}", package_dir),
            });
        }

        let canonical_dir =
            package_dir
                .canonicalize()
                .map_err(|e| InferError::SandboxValidation {
                    step: step.to_string(),
                    reason: format!("规范化算法包路径失败: {e}"),
                })?;

        // 检查核心必备文件并严格防御符号链接逃逸出算法包目录
        let check_contained_entry = |name: &str, is_dir: bool| -> Result<PathBuf, InferError> {
            let p = canonical_dir.join(name);
            let is_valid = if is_dir { p.is_dir() } else { p.is_file() };
            if !is_valid {
                let reason = match name {
                    "lib" => "缺少 lib/ 动态库目录".to_string(),
                    ALGO_MANIFEST_FILENAME => format!("缺少 {ALGO_MANIFEST_FILENAME} 文件"),
                    "testimage.jpg" => "缺少内置自测图 testimage.jpg".to_string(),
                    other => format!("缺少 {other} 文件"),
                };
                return Err(InferError::SandboxValidation {
                    step: step.to_string(),
                    reason,
                });
            }

            let canonical_p = p
                .canonicalize()
                .map_err(|e| InferError::SandboxValidation {
                    step: step.to_string(),
                    reason: format!("规范化 {name} 物理路径失败: {e}"),
                })?;

            if !canonical_p.starts_with(&canonical_dir) {
                return Err(InferError::SandboxValidation {
                    step: step.to_string(),
                    reason: format!(
                        "检测到符号链接路径逃逸: {name} 指向算法包目录外部 ({:?})",
                        canonical_p
                    ),
                });
            }

            Ok(canonical_p)
        };

        let manifest_path = check_contained_entry(ALGO_MANIFEST_FILENAME, false)?;
        let _lib_dir = check_contained_entry("lib", true)?;
        let _testimage_path = check_contained_entry("testimage.jpg", false)?;
        on_progress(SandboxProgressEvent::passed(1));

        let step = "2.解析 Manifest 与平台匹配";
        on_progress(SandboxProgressEvent::running(2));
        let manifest_str =
            std::fs::read_to_string(&manifest_path).map_err(|e| InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!("读取 {ALGO_MANIFEST_FILENAME} 失败: {e}"),
            })?;

        let manifest: AlgoManifest =
            serde_json::from_str(&manifest_str).map_err(|e| InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!("{ALGO_MANIFEST_FILENAME} 格式非法: {e}"),
            })?;

        // 强校验 Manifest 各元数据字段合法性（严防路径穿越与畸形字段注入）
        manifest.validate()?;

        let cur_platform = current_platform_id();
        let target_platform = normalize_platform_id(&manifest.platform_id);
        if target_platform != cur_platform {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!(
                    "平台架构不匹配: 本机环境为 [{cur_platform}], 算法包声明为 [{}] (归一化为 [{target_platform}])",
                    manifest.platform_id
                ),
            });
        }

        on_progress(SandboxProgressEvent::passed(2));
        let step = "3.Config Schema 格式校验";
        on_progress(SandboxProgressEvent::running(3));
        let schema_path = canonical_dir.join("config.schema.json");
        if schema_path.exists() {
            let canonical_schema =
                schema_path
                    .canonicalize()
                    .map_err(|e| InferError::SandboxValidation {
                        step: step.to_string(),
                        reason: format!("规范化 config.schema.json 失败: {e}"),
                    })?;
            if !canonical_schema.starts_with(&canonical_dir) {
                return Err(InferError::SandboxValidation {
                    step: step.to_string(),
                    reason: format!(
                        "检测到符号链接路径逃逸: config.schema.json 指向算法包目录外部 ({:?})",
                        canonical_schema
                    ),
                });
            }

            let schema_str = std::fs::read_to_string(&canonical_schema).map_err(|e| {
                InferError::SandboxValidation {
                    step: step.to_string(),
                    reason: format!("读取 config.schema.json 失败: {e}"),
                }
            })?;
            serde_json::from_str::<serde_json::Value>(&schema_str).map_err(|e| {
                InferError::SandboxValidation {
                    step: step.to_string(),
                    reason: format!("config.schema.json 不是合法的 JSON: {e}"),
                }
            })?;
        }

        on_progress(SandboxProgressEvent::passed(3));
        if use_subprocess {
            Self::run_subprocess_self_test_with_progress(&canonical_dir, &mut on_progress)?;
        } else {
            let entry_lib_path = find_entry_library(&canonical_dir, &manifest.algorithm_id)?;
            Self::run_in_process_self_test_with_progress(
                &canonical_dir,
                &entry_lib_path,
                &manifest,
                &mut on_progress,
            )?;
        }

        tracing::info!(
            algorithm_id = %manifest.algorithm_id,
            version = %manifest.version,
            "算法包六步沙箱安全校验 100% 通过"
        );

        Ok(manifest)
    }

    /// 通过独立子进程运行自测试（物理故障域隔离，防 SIGSEGV / 内存越界）
    fn run_subprocess_self_test_with_progress<F>(
        package_dir: &Path,
        mut on_progress: F,
    ) -> Result<(), InferError>
    where
        F: FnMut(SandboxProgressEvent),
    {
        on_progress(SandboxProgressEvent::running(4));
        let current_exe = std::env::var("HEIMDALL_BIN")
            .map(PathBuf::from)
            .or_else(|_| std::env::current_exe())
            .map_err(|e| InferError::SandboxValidation {
                step: "4.派生隔离子进程".to_string(),
                reason: format!("获取当前可执行文件路径失败: {e}"),
            })?;

        let mut child = std::process::Command::new(&current_exe)
            .arg(VERIFY_ALGO_ARG)
            .arg(package_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| InferError::SandboxValidation {
                step: "4.派生隔离子进程".to_string(),
                reason: format!("启动自测子进程失败: {e}"),
            })?;

        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(InferError::SandboxValidation {
                    step: "4.派生隔离子进程".to_string(),
                    reason: "自测子进程 stdout 管道不可用".to_string(),
                });
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(InferError::SandboxValidation {
                    step: "4.派生隔离子进程".to_string(),
                    reason: "自测子进程 stderr 管道不可用".to_string(),
                });
            }
        };

        let (stdout_tx, stdout_rx) = std::sync::mpsc::sync_channel::<SandboxProgressEvent>(16);
        let stderr_output = Arc::new(Mutex::new(CappedOutput::default()));
        let report_slot: SandboxReportSlot = Arc::new(Mutex::new(None));
        let stdout_reader = spawn_sandbox_stdout_reader(stdout, stdout_tx, report_slot.clone());
        let stderr_reader = spawn_sandbox_stderr_reader(stderr, stderr_output.clone());

        // 进程创建成功且 stdout/stderr 已被独立线程接管，第四步的隔离与监控设施就绪。
        on_progress(SandboxProgressEvent::passed(4));

        let mut reported_failed_step: Option<usize> = None;
        let mut consume_progress = |progress: SandboxProgressEvent| {
            if progress.status == SandboxStepStatus::Failed {
                reported_failed_step = Some(progress.step);
            }
            on_progress(progress);
        };

        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(10);

        loop {
            while let Ok(progress) = stdout_rx.try_recv() {
                consume_progress(progress);
            }

            match child.try_wait() {
                Ok(Some(status)) => {
                    // reader 线程使用非阻塞投递，不能因日志洪泛反向阻塞父线程。
                    // 子进程退出后只给管道一个有限排空窗口；若算法 fork 出持有管道的
                    // 后代，也不能让本次上传永久等待 reader 结束。
                    let drain_deadline = std::time::Instant::now()
                        + Duration::from_millis(SANDBOX_OUTPUT_DRAIN_TIMEOUT_MS);
                    while (!stdout_reader.is_finished() || !stderr_reader.is_finished())
                        && std::time::Instant::now() < drain_deadline
                    {
                        while let Ok(progress) = stdout_rx.try_recv() {
                            consume_progress(progress);
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    while let Ok(progress) = stdout_rx.try_recv() {
                        consume_progress(progress);
                    }

                    let stderr_msg = stderr_output
                        .lock()
                        .map(|captured| captured.snapshot())
                        .unwrap_or_default();
                    let report = report_slot.lock().ok().and_then(|mut slot| slot.take());

                    if status.success() {
                        let report = report.ok_or_else(|| InferError::SandboxValidation {
                            step: "6.真实前向推理自测".to_string(),
                            reason: "沙箱子进程未返回有效自测报告".to_string(),
                        })?;
                        tracing::info!(
                            algorithm_id = %report.algorithm_id,
                            version = %report.version,
                            detections = report.detections_count,
                            duration_ms = report.duration_ms,
                            "沙箱子进程真实前向自检成功完成"
                        );
                        return Ok(());
                    }

                    let failed_step_number = reported_failed_step.unwrap_or_else(|| {
                        if stderr_msg.contains("5.")
                            || stderr_msg.contains("定位动态库")
                            || stderr_msg.contains("查找动态库")
                            || stderr_msg.contains("C ABI")
                            || stderr_msg.contains("instance_create")
                            || stderr_msg.contains("instance_process")
                        {
                            5
                        } else {
                            6
                        }
                    });
                    let failed_step = match failed_step_number {
                        1 => "1.路径防穿透与结构检查",
                        2 => "2.解析 Manifest 与平台匹配",
                        3 => "3.Config Schema 格式校验",
                        4 => "4.派生隔离子进程",
                        5 => "5.算法库 C ABI 导出符号核对",
                        _ => "6.真实前向推理自测",
                    };
                    return Err(InferError::SandboxValidation {
                        step: failed_step.to_string(),
                        reason: format!(
                            "沙箱子进程异常退出 (状态码: {status:?}, stderr: {stderr_msg})"
                        ),
                    });
                }
                Ok(None) => {
                    if start.elapsed() > timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(InferError::SandboxValidation {
                            step: "6.真实前向推理自测".to_string(),
                            reason: "算法包自测超时 (超过 10 秒)，已被沙箱强杀".to_string(),
                        });
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(InferError::SandboxValidation {
                        step: "4.监控隔离子进程".to_string(),
                        reason: format!("等待子进程出错: {e}"),
                    });
                }
            }
        }
    }

    /// 进程内执行自测（供子进程入口或测试用例调用）
    pub fn run_in_process_self_test(
        package_dir: &Path,
        entry_lib_path: &Path,
        manifest: &AlgoManifest,
    ) -> Result<SelfTestReport, InferError> {
        Self::run_in_process_self_test_with_progress(package_dir, entry_lib_path, manifest, |_| {})
    }

    /// 进程内执行自测，并在 ABI 握手与真实前向推理边界发出事件。
    pub fn run_in_process_self_test_with_progress<F>(
        package_dir: &Path,
        entry_lib_path: &Path,
        manifest: &AlgoManifest,
        mut on_progress: F,
    ) -> Result<SelfTestReport, InferError>
    where
        F: FnMut(SandboxProgressEvent),
    {
        let start_time = std::time::Instant::now();
        on_progress(SandboxProgressEvent::running(5));

        // 1. 加载动态库
        let loaded_lib = Arc::new(LoadedLib::load(entry_lib_path)?);

        // 2. 打开算法库会话
        let raw_lib = RawAlgoLibrary::open(loaded_lib.clone(), package_dir, &manifest.platform_id)?;

        if raw_lib.meta().algorithm_id != manifest.algorithm_id {
            return Err(InferError::SandboxValidation {
                step: "5.核对算法库元数据".to_string(),
                reason: format!(
                    "动态库导出的 algorithm_id [{}] 与 manifest [{}] 不一致",
                    raw_lib.meta().algorithm_id,
                    manifest.algorithm_id
                ),
            });
        }

        let abi = loaded_lib.abi();
        let create_fn = abi.instance_create.ok_or_else(|| InferError::InvalidAbi {
            reason: "instance_create 为空".to_string(),
        })?;
        let process_fn = abi.instance_process.ok_or_else(|| InferError::InvalidAbi {
            reason: "instance_process 为空".to_string(),
        })?;
        on_progress(SandboxProgressEvent::passed(5));
        on_progress(SandboxProgressEvent::running(6));

        // 3. 准备测试图像并转换为硬件加速平台帧
        let testimage_path = package_dir.join("testimage.jpg");
        let img = image::open(&testimage_path).map_err(|e| InferError::SandboxValidation {
            step: "6.准备测试图片".to_string(),
            reason: format!("解码 testimage.jpg 失败: {e}"),
        })?;

        let rgb = img.to_rgb8();
        let (width, height) = rgb.dimensions();
        let rgb_raw = rgb.into_raw();

        #[cfg(target_os = "macos")]
        let pixel_buffer = crate::c_abi::cvpixelbuffer::NativePixelBuffer::from_rgb_to_nv12(
            &rgb_raw, width, height,
        )?;

        #[cfg(not(target_os = "macos"))]
        let mut host_nv12 = rgb_to_nv12_bytes(&rgb_raw, width as usize, height as usize);

        // 4. 创建测试模式实例并执行前向推理
        let inst_id = std::ffi::CString::new("sandbox_self_test").unwrap_or_default();
        let run_id = std::ffi::CString::new("run_0").unwrap_or_default();

        let collected_results = std::sync::Mutex::new(Vec::<String>::new());
        let results_ptr =
            &collected_results as *const std::sync::Mutex<Vec<String>> as *mut std::ffi::c_void;

        let inst_args = AvAlgoInstanceArgs {
            size: std::mem::size_of::<AvAlgoInstanceArgs>() as u32,
            api_version: AV_ALGO_API_VERSION,
            mode: AV_INSTANCE_INSTALL_SELF_TEST,
            reserved0: 0,
            instance_id: inst_id.as_ptr(),
            instance_run_id: run_id.as_ptr(),
            config_json: std::ptr::null(),
            config_json_len: 0,
            reserved1: 0,
            frame_ops: std::ptr::null(),
            image_ops: std::ptr::null(),
            on_result: Some(crate::c_abi::loader::algo_result_collector),
            result_user: results_ptr,
            rules: std::ptr::null(),
            rule_count: 0,
        };

        let mut raw_inst: AvAlgoInstance = std::ptr::null_mut();

        // SAFETY: inst_args 栈有效，raw_inst 指向有效指针
        let create_code = unsafe { create_fn(raw_lib.raw(), &inst_args, &mut raw_inst) };
        if create_code != AV_OK {
            // SAFETY: abi 有效；instance-level 错误使用部分返回的句柄（若有）提取详情。
            let error = unsafe { check_c_status(create_code, abi, raw_inst) };
            if !raw_inst.is_null() {
                if let Some(destroy_fn) = abi.instance_destroy {
                    // SAFETY: raw_inst 由当前 instance_create 返回，错误路径立即销毁且仅执行一次。
                    unsafe { destroy_fn(raw_inst) };
                }
            }
            return Err(error);
        }
        if raw_inst.is_null() {
            return Err(InferError::InvalidAbi {
                reason: "instance_create 成功但返回了空实例句柄".to_string(),
            });
        }

        // RAII 保护 instance 释放
        struct InstanceGuard<'a> {
            inst: AvAlgoInstance,
            abi: &'a AvAlgoAbi,
        }
        impl<'a> Drop for InstanceGuard<'a> {
            fn drop(&mut self) {
                if !self.inst.is_null() {
                    if let Some(destroy_fn) = self.abi.instance_destroy {
                        // SAFETY: 仅执行一次释放
                        unsafe { destroy_fn(self.inst) };
                    }
                }
            }
        }
        let _guard = InstanceGuard {
            inst: raw_inst,
            abi,
        };

        // 构造 FrameDesc
        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);

        #[cfg(target_os = "macos")]
        let (y_stride, uv_stride) = pixel_buffer.strides();
        #[cfg(not(target_os = "macos"))]
        let (y_stride, uv_stride) = ((width & !1) as i32, (width & !1) as i32);

        let mut frame =
            AvFrameDesc::default_nv12(width & !1, height & !1, y_stride, uv_stride, now_ns);

        #[cfg(target_os = "macos")]
        {
            frame.opaque = pixel_buffer.as_raw();
            frame.frame_token = pixel_buffer.as_raw();
            frame.opaque_kind = AV_OPAQUE_CVPIXELBUFFER;
        }
        #[cfg(not(target_os = "macos"))]
        {
            frame.opaque = host_nv12.as_mut_ptr() as *mut std::ffi::c_void;
        }

        // SAFETY: frame 栈内存有效
        let process_code = unsafe { process_fn(raw_inst, &frame) };
        if process_code != AV_OK {
            // SAFETY: 调用方保证 abi 与 raw_inst 内存有效
            return Err(unsafe { check_c_status(process_code, abi, raw_inst) });
        }

        on_progress(SandboxProgressEvent::passed(6));

        // 验证回调结果
        let detections_count = collected_results.lock().map(|r| r.len()).unwrap_or(0);

        Ok(SelfTestReport {
            status: "ok".to_string(),
            algorithm_id: manifest.algorithm_id.clone(),
            version: manifest.version.clone(),
            detections_count,
            duration_ms: start_time.elapsed().as_millis() as u64,
        })
    }
}

/// 查找算法包动态库文件（严格杜绝路径穿越与符号链接逃逸）
pub fn find_entry_library(package_dir: &Path, algorithm_id: &str) -> Result<PathBuf, InferError> {
    // 1. 防路径穿越强校验：algorithm_id 必须为合法的非空安全标识符
    AlgoManifest::validate_algorithm_id(algorithm_id, "定位动态库")?;

    let canonical_pkg = package_dir
        .canonicalize()
        .map_err(|e| InferError::SandboxValidation {
            step: "定位动态库".to_string(),
            reason: format!("规范化算法包路径失败: {e}"),
        })?;

    let lib_dir = canonical_pkg.join("lib");
    if !lib_dir.is_dir() {
        return Err(InferError::SandboxValidation {
            step: "定位动态库".to_string(),
            reason: format!("动态库目录不存在: {:?}", lib_dir),
        });
    }

    let canonical_lib_dir = lib_dir
        .canonicalize()
        .map_err(|e| InferError::SandboxValidation {
            step: "定位动态库".to_string(),
            reason: format!("规范化 lib/ 目录路径失败: {e}"),
        })?;

    if !canonical_lib_dir.starts_with(&canonical_pkg) {
        return Err(InferError::SandboxValidation {
            step: "定位动态库".to_string(),
            reason: format!(
                "检测到 lib/ 目录符号链接逃逸出算法包: {:?}",
                canonical_lib_dir
            ),
        });
    }

    let (ext, alt_ext) = if cfg!(target_os = "macos") {
        ("dylib", "so")
    } else {
        ("so", "dylib")
    };

    // 优先尝试宿主原生命名的动态库，再尝试异构目标平台的命名
    let mut candidate_file = [ext, alt_ext].into_iter().find_map(|candidate_ext| {
        let candidate = canonical_lib_dir.join(format!("lib{algorithm_id}.{candidate_ext}"));
        candidate.is_file().then_some(candidate)
    });

    // 扫描 lib/ 目录下的匹配文件（优先宿主原生扩展名，再回退异构扩展名）
    if candidate_file.is_none() {
        if let Ok(entries) = std::fs::read_dir(&canonical_lib_dir) {
            let mut fallback = None;
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(extension) = path.extension() else {
                    continue;
                };
                if extension == ext {
                    candidate_file = Some(path);
                    break;
                }
                if extension == alt_ext && fallback.is_none() {
                    fallback = Some(path);
                }
            }
            candidate_file = candidate_file.or(fallback);
        }
    }

    let found_path = candidate_file.ok_or_else(|| InferError::SandboxValidation {
        step: "定位动态库".to_string(),
        reason: format!(
            "在 {:?} 中未找到动态库文件 (*.{} / *.{})",
            canonical_lib_dir, ext, alt_ext
        ),
    })?;

    // 严防动态库物理路径符号链接逃逸出算法包目录
    let canonical_found = found_path
        .canonicalize()
        .map_err(|e| InferError::SandboxValidation {
            step: "定位动态库".to_string(),
            reason: format!("规范化动态库路径失败: {e}"),
        })?;

    if !canonical_found.starts_with(&canonical_pkg) {
        return Err(InferError::SandboxValidation {
            step: "定位动态库".to_string(),
            reason: format!(
                "动态库物理路径逃逸出算法包目录 (检测到符号链接逃逸): {:?}",
                canonical_found
            ),
        });
    }

    Ok(canonical_found)
}

/// 非 macOS 平台开发/回退自测使用的软件 RGB24 转 NV12 转换器
#[cfg(not(target_os = "macos"))]
fn rgb_to_nv12_bytes(rgb: &[u8], width: usize, height: usize) -> Vec<u8> {
    let w = width & !1;
    let h = height & !1;
    let y_size = w * h;
    let uv_size = (w * h) / 2;
    let mut nv12 = vec![0u8; y_size + uv_size];

    let clamp_byte = |v: f64| -> u8 { v.round().clamp(0.0, 255.0) as u8 };

    // 1. Y 平面
    for y in 0..h {
        for x in 0..w {
            let idx = (y * width + x) * 3;
            let r = rgb[idx] as f64;
            let g = rgb[idx + 1] as f64;
            let b = rgb[idx + 2] as f64;
            nv12[y * w + x] = clamp_byte(16.0 + (65.481 * r + 128.553 * g + 24.966 * b) / 255.0);
        }
    }

    // 2. UV 平面 (NV12: U0, V0, U1, V1...)
    let uv_offset = y_size;
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            let mut cb_sum = 0.0;
            let mut cr_sum = 0.0;
            for dy in 0..2 {
                for dx in 0..2 {
                    let idx = ((y + dy) * width + (x + dx)) * 3;
                    let r = rgb[idx] as f64;
                    let g = rgb[idx + 1] as f64;
                    let b = rgb[idx + 2] as f64;
                    cb_sum += 128.0 + (-37.797 * r - 74.203 * g + 112.0 * b) / 255.0;
                    cr_sum += 128.0 + (112.0 * r - 93.786 * g - 18.214 * b) / 255.0;
                }
            }
            let cb = clamp_byte(cb_sum / 4.0);
            let cr = clamp_byte(cr_sum / 4.0);
            let uv_idx = uv_offset + (y / 2) * w + x;
            nv12[uv_idx] = cb;
            nv12[uv_idx + 1] = cr;
        }
    }

    nv12
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdout_reader_discards_plugin_logs_without_blocking_protocol_delivery() {
        let mut output = String::new();
        for _ in 0..128 {
            output.push_str("plugin log\n");
        }
        let message = SandboxChildMessage::Progress(SandboxProgressEvent::passed(5));
        output.push_str(&serde_json::to_string(&message).expect("沙箱协议消息序列化失败"));
        output.push('\n');

        let report = SandboxChildMessage::Report(SelfTestReport {
            status: "ok".to_string(),
            algorithm_id: "flood-test".to_string(),
            version: "1.0.0".to_string(),
            detections_count: 1,
            duration_ms: 1,
        });
        output.push_str(&serde_json::to_string(&report).expect("沙箱报告序列化失败"));
        output.push('\n');

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let report_slot: SandboxReportSlot = Arc::new(Mutex::new(None));
        let reader = spawn_sandbox_stdout_reader(
            std::io::Cursor::new(output.into_bytes()),
            tx,
            report_slot.clone(),
        );
        assert!(reader.join().is_ok());
        assert!(matches!(
            rx.try_recv(),
            Ok(SandboxProgressEvent {
                step: 5,
                status: SandboxStepStatus::Passed,
            })
        ));
        assert!(matches!(
            report_slot.lock().ok().and_then(|mut slot| slot.take()),
            Some(SelfTestReport {
                detections_count: 1,
                ..
            })
        ));
    }

    #[test]
    fn stderr_capture_is_bounded() {
        let mut output = CappedOutput::default();
        output.append(&vec![b'x'; MAX_SANDBOX_STDERR_BYTES + 1]);

        assert_eq!(output.bytes.len(), MAX_SANDBOX_STDERR_BYTES);
        assert!(output.truncated);
        assert!(output.snapshot().ends_with("[stderr truncated at 64 KiB]"));
    }
}
