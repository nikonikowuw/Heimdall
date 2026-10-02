//! 硬件回退策略经**真实 C ABI 虚表**的端到端验证（A2 / A3 / A4 / A5 / A7）
//!
//! 本文件是自检硬门的集成证据：它不直接调用 SDK 内部函数，而是展开 `export_algo!`
//! 虚表后走 `library_open` → `instance_create` → `instance_process` 的真实调用链，
//! 与宿主 `AlgoSandbox::run_in_process_self_test` 在自检第 5、6 步所做的调用一致。
//!
//! # 为什么开发机（无 `librknnrt`）就是有效前提
//!
//! 本用例的失败分支要求"物理上无硬件推理运行时"。开发机（macOS / x86 Linux）
//! 天然满足该前提，因此这里**不需要** `#[ignore]`，失败分支会被真实走到。
//! 唯一例外：若本机恰好安装了 `librknnrt.so`，前提不成立——用例会显式跳过并说明原因，
//! 而不是伪装通过。

use std::ffi::{c_char, c_int, CStr, CString};
use std::path::Path;

use algo_sdk::prelude::*;

/// 测试用算法包规格（模型文件由用例在临时目录内构造）
struct GateSpec;

impl YoloSpec for GateSpec {
    const INPUT_DIM: (u32, u32) = (640, 640);
    const NUM_CLASSES: usize = 2;
    const LABELS: &'static [&'static str] = &["Hardhat", "NO-Hardhat"];
    const MODEL_PATH: &'static str = "model/model.rknn";
}

type GatePlugin = GenericYoloDetector<GateSpec>;

export_algo!(
    GatePlugin,
    algo_id: "fallback_gate_harness",
    version: "1.0.0",
    algo_type: "object_detection",
    alarm_type_id: "safety_violation"
);

/// 一次 `library_open` 的结果
struct Opened {
    abi: &'static AvAlgoAbi,
    lib: AvAlgoLibrary,
}

impl Opened {
    /// 经真实 ABI 虚表打开算法库
    fn new(platform_id: &str, package_root: &Path) -> Self {
        // SAFETY: 调取本测试二进制展开的 ABI 虚表指针
        let abi = unsafe { &*av_algo_get_abi(AV_ALGO_API_VERSION) };
        assert_eq!(
            std::mem::size_of::<AvAlgoAbi>(),
            abi.size as usize,
            "虚表尺寸必须与 ABI 声明一致"
        );

        let root = CString::new(package_root.to_str().expect("包路径应为 UTF-8")).expect("cstring");
        let platform = CString::new(platform_id).expect("cstring");
        let args = AvAlgoLibraryArgs {
            size: std::mem::size_of::<AvAlgoLibraryArgs>() as u32,
            api_version: AV_ALGO_API_VERSION,
            package_root: root.as_ptr(),
            platform_id: platform.as_ptr(),
            platform_tag: 0,
            log: None,
            log_user: std::ptr::null_mut(),
        };

        let mut lib: AvAlgoLibrary = std::ptr::null_mut();
        // SAFETY: 参数与出参在调用期有效
        let status = unsafe { (abi.library_open.expect("library_open"))(&args, &mut lib) };
        assert_eq!(status, AV_OK, "library_open 必须成功: {status}");
        assert!(!lib.is_null());
        Self { abi, lib }
    }

    /// 以指定实例模式创建实例，返回 `(状态码, 实例句柄)`
    fn create_instance(&self, mode: u32, config_json: &str) -> (c_int, AvAlgoInstance) {
        let inst_id = CString::new("gate_inst").expect("cstring");
        let run_id = CString::new("gate_run").expect("cstring");
        let config = CString::new(config_json).expect("cstring");
        let args = AvAlgoInstanceArgs {
            size: std::mem::size_of::<AvAlgoInstanceArgs>() as u32,
            api_version: AV_ALGO_API_VERSION,
            mode,
            reserved0: 0,
            instance_id: inst_id.as_ptr(),
            instance_run_id: run_id.as_ptr(),
            config_json: config.as_ptr(),
            config_json_len: config.as_bytes().len() as u32,
            reserved1: 0,
            frame_ops: std::ptr::null(),
            image_ops: std::ptr::null(),
            on_result: None,
            result_user: std::ptr::null_mut(),
            rules: std::ptr::null(),
            rule_count: 0,
        };

        let mut inst: AvAlgoInstance = std::ptr::null_mut();
        // SAFETY: 参数栈有效，出参指针有效
        let status =
            unsafe { (self.abi.instance_create.expect("create"))(self.lib, &args, &mut inst) };
        (status, inst)
    }

    /// 读取线程局部 `last_error` 详情（与宿主 `check_c_status` 同一读取路径）
    ///
    /// 缓冲必须与宿主**逐字节一致**（`crates/infer/src/c_abi/loader.rs` 的
    /// `[0 as c_char; 512]`）：若这里开得更大，本文件的定位信息断言就会在
    /// 宿主实际看不到后半段的情况下依然通过，失去判别力。
    fn last_error(&self) -> String {
        /// 宿主 `check_c_status` 的栈缓冲尺寸
        const HOST_ERROR_BUFFER: usize = 512;

        let mut buf = [0 as c_char; HOST_ERROR_BUFFER];
        // SAFETY: 缓冲区为本栈内存且容量真实
        let status = unsafe {
            (self.abi.last_error.expect("last_error"))(
                std::ptr::null_mut(),
                buf.as_mut_ptr(),
                buf.len() as u32,
            )
        };
        if status != AV_OK {
            return String::new();
        }
        // SAFETY: 桥接层保证以 NUL 结尾
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }
}

impl Drop for Opened {
    fn drop(&mut self) {
        if !self.lib.is_null() {
            // SAFETY: lib 由本结构持有的 library_open 返回，且只在此处关闭一次
            unsafe { (self.abi.library_close.expect("close"))(self.lib) };
            self.lib = std::ptr::null_mut();
        }
    }
}

/// 构造含 `model/model.rknn` 的临时算法包目录
///
/// 模型文件必须真实存在，否则 `init` 会在模型路径校验阶段就失败，
/// 使本用例因为错误的理由通过。
fn temp_package(env_extra: Option<&str>) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("algo_fallback_gate_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("model")).expect("create model dir");
    std::fs::write(dir.join("model/model.rknn"), b"mock rknn bytes").expect("write model");
    if let Some(env) = env_extra {
        std::fs::write(dir.join(".env"), env).expect("write .env");
    }
    dir
}

/// 本机是否物理安装了 RKNN 运行时（安装了则本文件的"无硬件"前提不成立）
///
/// 候选路径与 `RknnRuntime::load` 保持一致。这里只做存在性探测而不真正 `dlopen`：
/// 探测发生在用例体内，不应把运行时载入测试进程。
fn host_has_rknn_runtime(pkg_root: &Path) -> bool {
    [
        pkg_root.join("lib/librknnrt.so"),
        pkg_root.join("lib64/librknnrt.so"),
        std::path::PathBuf::from("librknnrt.so"),
        std::path::PathBuf::from("/usr/lib/librknnrt.so"),
        std::path::PathBuf::from("/usr/lib64/librknnrt.so"),
        std::path::PathBuf::from("/usr/local/lib/librknnrt.so"),
    ]
    .iter()
    .any(|path| path.is_file())
}

/// A7：安装自检模式 + 无硬件 → `instance_create` 返回 `AV_ERR_MODEL_LOAD_FAILED`
///
/// 这是自检硬门的集成证据：宿主 `AlgoSandbox` 第 5 步拿到非 `AV_OK` 即判定失败，
/// 因此无需宿主新增任何校验代码。
#[test]
fn self_test_mode_without_hardware_fails_instance_create() {
    let pkg = temp_package(None);
    let opened = Opened::new("linux-rknn", &pkg);
    if host_has_rknn_runtime(&pkg) {
        eprintln!("本机已安装 librknnrt，用例前提（无硬件运行时）不成立，跳过");
        let _ = std::fs::remove_dir_all(&pkg);
        return;
    }

    let (status, inst) = opened.create_instance(AV_INSTANCE_INSTALL_SELF_TEST, "{}");

    assert_eq!(
        status,
        AV_ERR_MODEL_LOAD_FAILED,
        "自检模式下模拟降级不得成功创建实例（实际状态码 {status}，last_error={}）",
        opened.last_error()
    );
    assert!(
        inst.is_null(),
        "失败路径必须不返回任何实例句柄（不得返回可用的模拟会话）"
    );

    // 失败信息必须可操作，运维能据此定位
    let detail = opened.last_error();
    for needle in ["librknnrt", "package_root", "ALLOW_CPU_FALLBACK"] {
        assert!(
            detail.contains(needle),
            "last_error 必须包含可定位信息 `{needle}`，实际: {detail}"
        );
    }

    let _ = std::fs::remove_dir_all(&pkg);
}

/// A2 + A4：常规实例在硬件平台上同样受策略约束（不只在自检阶段）
///
/// 仅对自检加门会留下死角：生产常规实例仍会静默降级，与"算法包声明了本平台"矛盾。
#[test]
fn normal_mode_on_hardware_platform_also_requires_hardware() {
    let pkg = temp_package(None);
    let opened = Opened::new("linux-rknn", &pkg);
    if host_has_rknn_runtime(&pkg) {
        eprintln!("本机已安装 librknnrt，用例前提（无硬件运行时）不成立，跳过");
        let _ = std::fs::remove_dir_all(&pkg);
        return;
    }

    let (status, inst) = opened.create_instance(AV_INSTANCE_NORMAL, "{}");
    assert_eq!(
        status,
        AV_ERR_MODEL_LOAD_FAILED,
        "linux-rknn 平台上的常规实例不得静默降级（last_error={}）",
        opened.last_error()
    );
    assert!(inst.is_null());

    let _ = std::fs::remove_dir_all(&pkg);
}

/// A5：非硬件平台（开发机）默认策略仍为 `Allow`，回退会话可用且可推理
#[test]
fn non_hardware_platform_keeps_simulation_available() {
    let pkg = temp_package(None);
    let opened = Opened::new("test-platform", &pkg);

    let (status, inst) = opened.create_instance(AV_INSTANCE_NORMAL, "{}");
    assert_eq!(
        status,
        AV_OK,
        "非硬件平台必须保留无驱动回退能力（last_error={}）",
        opened.last_error()
    );
    assert!(!inst.is_null());

    // 回退会话必须能真正完成一次后处理闭环
    let frame = MockFrameBuilder::new()
        .dimensions(1920, 1080)
        .to_nv12(16)
        .build();
    // SAFETY: 实例有效，帧描述符由脚手架构造并满足 ABI 约束
    let process_status =
        unsafe { (opened.abi.instance_process.expect("process"))(inst, frame.raw_desc()) };
    assert_eq!(process_status, AV_OK, "回退会话必须能完成推理闭环");

    // SAFETY: 实例由本测试创建且未销毁
    unsafe { (opened.abi.instance_destroy.expect("destroy"))(inst) };

    let _ = std::fs::remove_dir_all(&pkg);
}

/// A5 + 运维逃生口：`.env` 显式 `ALLOW_CPU_FALLBACK=1` 可放开硬件平台上的模拟回退
///
/// 该断言保证 `RequireHardware` 错误信息中的操作指引不是空头承诺。
#[test]
fn explicit_env_override_restores_simulation_on_hardware_platform() {
    let pkg = temp_package(Some("ALLOW_CPU_FALLBACK=1\n"));
    let opened = Opened::new("linux-rknn", &pkg);

    let (status, inst) = opened.create_instance(AV_INSTANCE_NORMAL, "{}");
    assert_eq!(
        status,
        AV_OK,
        "显式 ALLOW_CPU_FALLBACK=1 必须放开模拟回退（last_error={}）",
        opened.last_error()
    );
    assert!(!inst.is_null());

    // SAFETY: 实例由本测试创建且未销毁
    unsafe { (opened.abi.instance_destroy.expect("destroy"))(inst) };
    let _ = std::fs::remove_dir_all(&pkg);
}

/// A4：自检硬门不可被 `.env` 覆盖（模拟会话不得冒充自检通过）
#[test]
fn self_test_gate_cannot_be_overridden_by_env() {
    let pkg = temp_package(Some("ALLOW_CPU_FALLBACK=1\n"));
    let opened = Opened::new("linux-rknn", &pkg);
    if host_has_rknn_runtime(&pkg) {
        eprintln!("本机已安装 librknnrt，用例前提（无硬件运行时）不成立，跳过");
        let _ = std::fs::remove_dir_all(&pkg);
        return;
    }

    let (status, inst) = opened.create_instance(AV_INSTANCE_INSTALL_SELF_TEST, "{}");
    assert_eq!(
        status,
        AV_ERR_MODEL_LOAD_FAILED,
        "安装自检必须始终要求真实硬件（last_error={}）",
        opened.last_error()
    );
    assert!(inst.is_null());

    let _ = std::fs::remove_dir_all(&pkg);
}
