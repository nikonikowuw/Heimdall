//! 七步安全沙箱自检器与平台拓扑感知体系
//! 支持物理子进程隔离执行自检，坚决防范段错误（SIGSEGV）带崩主进程。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::c_abi::loader::{check_c_status, LoadedLib, RawAlgoLibrary};
use crate::c_abi::types::*;
use crate::error::InferError;

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

/// 算法包自检结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfTestReport {
    pub status: String,
    pub algorithm_id: String,
    pub version: String,
    pub detections_count: usize,
    pub duration_ms: u64,
}

/// 算法包安全沙箱自检器
#[derive(Debug, Default)]
pub struct AlgoSandbox;

impl AlgoSandbox {
    /// 对指定算法包目录执行七步沙箱安全校验
    ///
    /// `use_subprocess`: 是否启用物理子进程隔离自检（生产和上传时必须为 true）
    pub fn validate_package(
        package_dir: &Path,
        use_subprocess: bool,
    ) -> Result<AlgoManifest, InferError> {
        let step = "1.路径防穿透与结构检查";
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

        // 检查核心必备文件
        let manifest_path = canonical_dir.join("manifest.json");
        if !manifest_path.is_file() {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: "缺少 manifest.json 文件".to_string(),
            });
        }

        let lib_dir = canonical_dir.join("lib");
        if !lib_dir.is_dir() {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: "缺少 lib/ 动态库目录".to_string(),
            });
        }

        let testimage_path = canonical_dir.join("testimage.jpg");
        if !testimage_path.is_file() {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: "缺少内置自测图 testimage.jpg".to_string(),
            });
        }

        let step = "2.SHA256 完整性与安全指纹校验";
        let checksum_file = canonical_dir.join("checksum.sha256");
        if checksum_file.is_file() {
            verify_checksum_file(&checksum_file, &canonical_dir).map_err(|e| {
                InferError::SandboxValidation {
                    step: step.to_string(),
                    reason: format!("SHA256 校验失败: {e}"),
                }
            })?;
        } else if let Ok(hash) = compute_file_sha256(&manifest_path) {
            tracing::debug!(
                manifest_sha256 = %hash,
                "算法包未附带 checksum.sha256，已登记 manifest 安全指纹"
            );
        }

        let step = "3.解析 Manifest 与平台匹配";
        let manifest_str =
            std::fs::read_to_string(&manifest_path).map_err(|e| InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!("读取 manifest.json 失败: {e}"),
            })?;

        let manifest: AlgoManifest =
            serde_json::from_str(&manifest_str).map_err(|e| InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!("manifest.json 格式非法: {e}"),
            })?;

        if manifest.manifest_version != 1 {
            return Err(InferError::SandboxValidation {
                step: step.to_string(),
                reason: format!("不支持的 manifest_version: {}", manifest.manifest_version),
            });
        }

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

        let step = "4.Config Schema 格式校验";
        let schema_path = canonical_dir.join("config.schema.json");
        if schema_path.exists() {
            let schema_str = std::fs::read_to_string(&schema_path).map_err(|e| {
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

        let entry_lib_path = find_entry_library(&canonical_dir, &manifest.algorithm_id)?;

        if use_subprocess {
            Self::run_subprocess_self_test(&canonical_dir)?;
        } else {
            Self::run_in_process_self_test(&canonical_dir, &entry_lib_path, &manifest)?;
        }

        tracing::info!(
            algorithm_id = %manifest.algorithm_id,
            version = %manifest.version,
            "算法包七步沙箱安全校验 100% 通过"
        );

        Ok(manifest)
    }

    /// 通过独立子进程运行自测试（物理故障域隔离，防 SIGSEGV / 内存越界）
    fn run_subprocess_self_test(package_dir: &Path) -> Result<(), InferError> {
        use std::io::Read;

        let current_exe = std::env::var("ARGUS_BIN")
            .map(PathBuf::from)
            .or_else(|_| std::env::current_exe())
            .map_err(|e| InferError::SandboxValidation {
                step: "5.派生隔离子进程".to_string(),
                reason: format!("获取当前可执行文件路径失败: {e}"),
            })?;

        let mut child = std::process::Command::new(&current_exe)
            .arg("__verify-algo")
            .arg(package_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| InferError::SandboxValidation {
                step: "5.派生隔离子进程".to_string(),
                reason: format!("启动自测子进程失败: {e}"),
            })?;

        // 设置 10 秒超时看门狗
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(10);

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if status.success() {
                        let mut stdout_msg = String::new();
                        if let Some(mut out_pipe) = child.stdout.take() {
                            let _ = out_pipe.read_to_string(&mut stdout_msg);
                        }
                        if let Ok(report) = serde_json::from_str::<SelfTestReport>(&stdout_msg) {
                            tracing::info!(
                                algorithm_id = %report.algorithm_id,
                                version = %report.version,
                                detections = report.detections_count,
                                duration_ms = report.duration_ms,
                                "沙箱子进程真实前向自检成功完成"
                            );
                        }
                        return Ok(());
                    } else {
                        // 进程崩溃或异常退出（例如收到 SIGSEGV）
                        let mut stderr_msg = String::new();
                        if let Some(mut err_pipe) = child.stderr.take() {
                            let _ = err_pipe.read_to_string(&mut stderr_msg);
                        }
                        return Err(InferError::SandboxValidation {
                            step: "7.真实前向推理自测".to_string(),
                            reason: format!(
                                "沙箱子进程异常退出 (状态码: {status:?}, stderr: {stderr_msg})"
                            ),
                        });
                    }
                }
                Ok(None) => {
                    if start.elapsed() > timeout {
                        let _ = child.kill();
                        return Err(InferError::SandboxValidation {
                            step: "7.真实前向推理自测".to_string(),
                            reason: "算法包自测超时 (超过 10 秒)，已被沙箱强杀".to_string(),
                        });
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    return Err(InferError::SandboxValidation {
                        step: "5.监控隔离子进程".to_string(),
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
        let start_time = std::time::Instant::now();

        // 1. 加载动态库
        let loaded_lib = Arc::new(LoadedLib::load(entry_lib_path)?);

        // 2. 打开算法库会话
        let raw_lib = RawAlgoLibrary::open(loaded_lib.clone(), package_dir, &manifest.platform_id)?;

        if raw_lib.meta().algorithm_id != manifest.algorithm_id {
            return Err(InferError::SandboxValidation {
                step: "6.核对算法库元数据".to_string(),
                reason: format!(
                    "动态库导出的 algorithm_id [{}] 与 manifest [{}] 不一致",
                    raw_lib.meta().algorithm_id,
                    manifest.algorithm_id
                ),
            });
        }

        // 3. 准备测试图像并转换为硬件加速平台帧
        let testimage_path = package_dir.join("testimage.jpg");
        let img = image::open(&testimage_path).map_err(|e| InferError::SandboxValidation {
            step: "7.准备测试图片".to_string(),
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
        let abi = loaded_lib.abi();
        let create_fn = abi.instance_create.ok_or_else(|| InferError::InvalidAbi {
            reason: "instance_create 为空".to_string(),
        })?;

        // SAFETY: inst_args 栈有效，raw_inst 指向有效指针
        let create_code = unsafe { create_fn(raw_lib.raw(), &inst_args, &mut raw_inst) };
        if create_code != AV_OK || raw_inst.is_null() {
            // SAFETY: 调用方保证 abi 与 raw_inst 内存有效
            return Err(unsafe { check_c_status(create_code, abi, std::ptr::null_mut()) });
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
            frame.memory_type = AV_MEM_PLATFORM_SURFACE;
            frame.layout = AV_LAYOUT_PLATFORM_NATIVE;
        }
        #[cfg(not(target_os = "macos"))]
        {
            frame.opaque = host_nv12.as_mut_ptr() as *mut std::ffi::c_void;
        }

        let process_fn = abi.instance_process.ok_or_else(|| InferError::InvalidAbi {
            reason: "instance_process 为空".to_string(),
        })?;

        // SAFETY: frame 栈内存有效
        let process_code = unsafe { process_fn(raw_inst, &frame) };
        if process_code != AV_OK {
            // SAFETY: 调用方保证 abi 与 raw_inst 内存有效
            return Err(unsafe { check_c_status(process_code, abi, raw_inst) });
        }

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

/// 查找算法包动态库文件
pub fn find_entry_library(package_dir: &Path, algorithm_id: &str) -> Result<PathBuf, InferError> {
    let lib_dir = package_dir.join("lib");
    let ext = if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };

    // 优先尝试标准命名的动态库: lib{algorithm_id}.{ext}
    let standard_name = format!("lib{algorithm_id}.{ext}");
    let candidate = lib_dir.join(&standard_name);
    if candidate.is_file() {
        return Ok(candidate);
    }

    // 扫描 lib/ 目录下的第一个匹配后缀的动态库
    if let Ok(entries) = std::fs::read_dir(&lib_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(e) = path.extension() {
                if e == ext {
                    return Ok(path);
                }
            }
        }
    }

    Err(InferError::SandboxValidation {
        step: "定位动态库".to_string(),
        reason: format!("在 {:?} 中未找到动态库文件 (*.{})", lib_dir, ext),
    })
}

/// 计算指定文件的 SHA256 十六进制摘要
pub fn compute_file_sha256(path: &Path) -> Result<String, std::io::Error> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// 校验算法包内置的 checksum.sha256 文件
fn verify_checksum_file(checksum_file: &Path, base_dir: &Path) -> Result<(), String> {
    let content = std::fs::read_to_string(checksum_file).map_err(|e| e.to_string())?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let expected_hash = parts[0];
        let file_rel = parts[1].trim_start_matches('*');
        let target_file = base_dir.join(file_rel);
        if !target_file.is_file() {
            return Err(format!("校验列表中的文件不存在: {file_rel}"));
        }
        let actual_hash = compute_file_sha256(&target_file).map_err(|e| e.to_string())?;
        if !expected_hash.eq_ignore_ascii_case(&actual_hash) {
            return Err(format!(
                "文件 {file_rel} 校验和不匹配: 期望 {expected_hash}, 实际 {actual_hash}"
            ));
        }
    }
    Ok(())
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
