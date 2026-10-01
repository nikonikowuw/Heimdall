//! 插件生命周期与 export_algo! 宏集成验证测试

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use algo_sdk::prelude::*;
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
struct MockConfig {
    #[serde(default)]
    threshold: f32,
}

struct MockDetector {
    _threshold: f32,
    rule_count: usize,
    panic_on_process: bool,
}

impl AlgoPlugin for MockDetector {
    type Config = MockConfig;

    fn init(ctx: &InitContext<'_>, config: Self::Config) -> Result<Self, AlgoError> {
        if ctx.package_root.as_os_str().is_empty() {
            return Err(AlgoError::ConfigParse {
                reason: "package_root 为空".to_string(),
            });
        }

        let panic_on_process = ctx.instance_id == "panic_inst";

        Ok(Self {
            _threshold: config.threshold,
            rule_count: 0,
            panic_on_process,
        })
    }

    fn process(
        &mut self,
        frame: SafeFrame<'_>,
        emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        if self.panic_on_process {
            panic!("故意触发的算法 Panic 异常！");
        }

        let boxes = [NormBox::new(0.1, 0.2, 0.3, 0.4, 0.95, 0).with_label("vehicle")];

        // 验证帧属性
        assert!(frame.width() > 0);
        assert!(frame.height() > 0);

        emitter.emit_detections(&boxes)?;
        Ok(())
    }

    fn flush(&mut self, _emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
        Ok(())
    }

    fn set_rules(&mut self, rules: &[AvRule]) -> Result<(), AlgoError> {
        self.rule_count = rules.len();
        Ok(())
    }
}

// 展开 C ABI 宏
export_algo!(
    MockDetector,
    algo_id: "mock_detector",
    version: "1.0.0",
    algo_type: "object_detection",
    alarm_type_id: "intrusion"
);

static RESULT_COUNT: AtomicUsize = AtomicUsize::new(0);

/// 串行化所有触及 `library_open` / `library_close` 的用例。
///
/// 日志桥接的登记槽位与活跃标记是**进程级全局状态**，而 `library_close` 会停用
/// 桥接；若用例并行执行，一个用例的 `library_close` 会在另一个用例断言前
/// 把桥接关掉，导致间歇性失败。
static LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());

/// 宿主日志回调收到的调用：(level, 消息, msg[len] 是否为 NUL)。
static LOG_CALLS: Mutex<Vec<(c_int, String, bool)>> = Mutex::new(Vec::new());

unsafe extern "C" fn test_logger(_user: *mut c_void, level: c_int, msg: *const c_char, len: u32) {
    if msg.is_null() {
        return;
    }
    // SAFETY: 桥接层保证 msg 指向本调用期有效、以 0 结尾的缓冲。
    let text = unsafe { CStr::from_ptr(msg) }
        .to_string_lossy()
        .into_owned();
    // 宿主 `default_c_logger` 忽略 len 改用 `CStr::from_ptr`，因此第 len 字节必须是
    // 终止符；这里记录下来回到用例主体断言，而不在回调内 assert（回调被
    // catch_unwind 包裹，内部 panic 会被静默吞掉，无法作为失败信号）。
    // SAFETY: 桥接层保证缓冲至少有 len + 1 字节。
    let nul_at_len = unsafe { *msg.add(len as usize) } == 0;
    LOG_CALLS
        .lock()
        .expect("log calls lock")
        .push((level, text, nul_at_len));
}

unsafe extern "C" fn test_on_result(result: *const AvAlgoResult, _user: *mut c_void) {
    if !result.is_null() {
        // SAFETY: 测试中宿主保证传入有效的非空 result 指针
        let res = unsafe { &*result };
        if res.kind == AV_RESULT_ALARM {
            RESULT_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[test]
fn test_plugin_full_lifecycle() {
    let _guard = LIFECYCLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // 1. 获取 C ABI 虚表
    // SAFETY: 测试调取本模块导出的 ABI 虚表指针
    let abi_ptr = unsafe { av_algo_get_abi(AV_ALGO_API_VERSION) };
    assert!(!abi_ptr.is_null(), "av_algo_get_abi 符号应有效");
    // SAFETY: abi_ptr 非空
    let abi = unsafe { &*abi_ptr };

    // 2. library_open
    let pkg_root = CString::new("/tmp/mock_pkg").expect("cstring");
    let platform = CString::new("rk3588").expect("cstring");
    let open_args = AvAlgoLibraryArgs {
        size: std::mem::size_of::<AvAlgoLibraryArgs>() as u32,
        api_version: AV_ALGO_API_VERSION,
        package_root: pkg_root.as_ptr(),
        platform_id: platform.as_ptr(),
        platform_tag: 0,
        log: None,
        log_user: std::ptr::null_mut(),
    };

    let mut lib: AvAlgoLibrary = std::ptr::null_mut();
    // SAFETY: 传入合法的 open_args
    let status = unsafe { (abi.library_open.expect("open"))(&open_args, &mut lib) };
    assert_eq!(status, AV_OK);
    assert!(!lib.is_null());

    // 3. library_query
    let mut info = AvAlgoLibraryInfo {
        size: std::mem::size_of::<AvAlgoLibraryInfo>() as u32,
        api_version: AV_ALGO_API_VERSION,
        algorithm_id: [0; 64],
        version: [0; 32],
        algorithm_type: [0; 32],
        alarm_type_id: [0; 64],
    };
    // SAFETY: info 指针合法
    let query_status = unsafe { (abi.library_query.expect("query"))(lib, &mut info) };
    assert_eq!(query_status, AV_OK);
    // SAFETY: 字符串由宏保证以 null 结尾
    let id_str = unsafe { CStr::from_ptr(info.algorithm_id.as_ptr()) }
        .to_str()
        .expect("utf8");
    assert_eq!(id_str, "mock_detector");

    // 4. instance_create
    let inst_id = CString::new("inst_001").expect("cstring");
    let run_id = CString::new("run_001").expect("cstring");
    let config_json = CString::new(r#"{"threshold": 0.5}"#).expect("cstring");

    let inst_args = AvAlgoInstanceArgs {
        size: std::mem::size_of::<AvAlgoInstanceArgs>() as u32,
        api_version: AV_ALGO_API_VERSION,
        mode: AV_INSTANCE_NORMAL,
        reserved0: 0,
        instance_id: inst_id.as_ptr(),
        instance_run_id: run_id.as_ptr(),
        config_json: config_json.as_ptr(),
        config_json_len: config_json.as_bytes().len() as u32,
        reserved1: 0,
        frame_ops: std::ptr::null(),
        image_ops: std::ptr::null(),
        on_result: Some(test_on_result),
        result_user: std::ptr::null_mut(),
        rules: std::ptr::null(),
        rule_count: 0,
    };

    let mut inst: AvAlgoInstance = std::ptr::null_mut();
    // SAFETY: 传入合法的 inst_args
    let create_status =
        unsafe { (abi.instance_create.expect("create"))(lib, &inst_args, &mut inst) };
    assert_eq!(create_status, AV_OK);
    assert!(!inst.is_null());

    // 5. instance_process
    let mock_frame = MockFrameBuilder::new()
        .dimensions(1920, 1080)
        .to_nv12(16)
        .build();

    RESULT_COUNT.store(0, Ordering::SeqCst);
    // SAFETY: 实例与帧描述符有效
    let process_status =
        unsafe { (abi.instance_process.expect("process"))(inst, mock_frame.raw_desc()) };
    assert_eq!(process_status, AV_OK);
    assert_eq!(RESULT_COUNT.load(Ordering::SeqCst), 1);

    // 6. instance_flush
    // SAFETY: inst 实例句柄有效
    let flush_status = unsafe { (abi.instance_flush.expect("flush"))(inst) };
    assert_eq!(flush_status, AV_OK);

    // 7. instance_destroy
    // SAFETY: inst 实例句柄有效
    let destroy_status = unsafe { (abi.instance_destroy.expect("destroy"))(inst) };
    assert_eq!(destroy_status, AV_OK);

    // 8. library_close
    // SAFETY: lib 句柄有效
    let close_status = unsafe { (abi.library_close.expect("close"))(lib) };
    assert_eq!(close_status, AV_OK);
}

#[test]
fn test_plugin_panic_isolation() {
    let _guard = LIFECYCLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // SAFETY: 调取 ABI 虚表指针
    let abi_ptr = unsafe { av_algo_get_abi(AV_ALGO_API_VERSION) };
    // SAFETY: abi_ptr 非空
    let abi = unsafe { &*abi_ptr };

    let pkg_root = CString::new("/tmp/mock_pkg").expect("cstring");
    let platform = CString::new("test").expect("cstring");
    let open_args = AvAlgoLibraryArgs {
        size: std::mem::size_of::<AvAlgoLibraryArgs>() as u32,
        api_version: AV_ALGO_API_VERSION,
        package_root: pkg_root.as_ptr(),
        platform_id: platform.as_ptr(),
        platform_tag: 0,
        log: None,
        log_user: std::ptr::null_mut(),
    };

    let mut lib: AvAlgoLibrary = std::ptr::null_mut();
    // SAFETY: 传入合法的 open_args
    unsafe { (abi.library_open.expect("open"))(&open_args, &mut lib) };

    // 创建触发 panic 的实例
    let inst_id = CString::new("panic_inst").expect("cstring");
    let run_id = CString::new("run_panic").expect("cstring");
    let inst_args = AvAlgoInstanceArgs {
        size: std::mem::size_of::<AvAlgoInstanceArgs>() as u32,
        api_version: AV_ALGO_API_VERSION,
        mode: AV_INSTANCE_NORMAL,
        reserved0: 0,
        instance_id: inst_id.as_ptr(),
        instance_run_id: run_id.as_ptr(),
        config_json: std::ptr::null(),
        config_json_len: 0,
        reserved1: 0,
        frame_ops: std::ptr::null(),
        image_ops: std::ptr::null(),
        on_result: None,
        result_user: std::ptr::null_mut(),
        rules: std::ptr::null(),
        rule_count: 0,
    };

    let mut inst: AvAlgoInstance = std::ptr::null_mut();
    // SAFETY: 传入合法的 inst_args
    unsafe { (abi.instance_create.expect("create"))(lib, &inst_args, &mut inst) };

    let mock_frame = MockFrameBuilder::new().build();

    // 执行 process，内部触发 panic
    // SAFETY: inst 与 mock_frame 有效
    let status = unsafe { (abi.instance_process.expect("process"))(inst, mock_frame.raw_desc()) };

    // 核心断言：Panic 必须被 catch_unwind 拦截并返回 AV_ERR_INTERNAL (-99)，宿主进程不崩溃
    assert_eq!(status, AV_ERR_INTERNAL);

    // 验证 last_error 捕获到信息
    let mut err_buf = [0 as c_char; 256];
    // SAFETY: 传入合法 256 字节缓冲区
    let err_status =
        unsafe { (abi.last_error.expect("last_error"))(inst, err_buf.as_mut_ptr(), 256) };
    assert_eq!(err_status, AV_OK);
    // SAFETY: err_buf 由 copy_last_error 保证以 null 结尾
    let err_msg = unsafe { CStr::from_ptr(err_buf.as_ptr()) }
        .to_str()
        .expect("utf8");
    assert!(err_msg.contains("Panic"), "错误信息应当记录 Panic");

    // SAFETY: 销毁测试资源
    unsafe {
        (abi.instance_destroy.expect("destroy"))(inst);
        (abi.library_close.expect("close"))(lib);
    }
}

/// B1 / B2 / B8：宿主在 `library_open` 提供的日志回调必须真正收到插件内事件。
///
/// 这是端到端用例：经真实 ABI 虚表触发 `library_open`，而不是直接调用桥接内部函数。
#[test]
fn test_library_open_bridges_host_log_callback() {
    let _guard = LIFECYCLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    LOG_CALLS.lock().expect("log calls lock").clear();

    // SAFETY: 调取 ABI 虚表指针
    let abi_ptr = unsafe { av_algo_get_abi(AV_ALGO_API_VERSION) };
    assert!(!abi_ptr.is_null(), "av_algo_get_abi 符号应有效");
    // SAFETY: abi_ptr 非空
    let abi = unsafe { &*abi_ptr };

    let pkg_root = CString::new("/tmp/mock_pkg").expect("cstring");
    let platform = CString::new("test-platform").expect("cstring");
    let open_args = AvAlgoLibraryArgs {
        size: std::mem::size_of::<AvAlgoLibraryArgs>() as u32,
        api_version: AV_ALGO_API_VERSION,
        package_root: pkg_root.as_ptr(),
        platform_id: platform.as_ptr(),
        platform_tag: 0,
        log: Some(test_logger),
        log_user: std::ptr::null_mut(),
    };

    let mut lib: AvAlgoLibrary = std::ptr::null_mut();
    // SAFETY: 传入合法的 open_args
    let status = unsafe { (abi.library_open.expect("open"))(&open_args, &mut lib) };
    assert_eq!(status, AV_OK);
    assert!(!lib.is_null());

    // library_open 之后、library_close 之前的事件必须到达宿主回调。
    tracing::warn!(backend = "rknn", "回退会话已启用（测试）");

    let calls = LOG_CALLS.lock().expect("log calls lock").clone();
    assert_eq!(calls.len(), 1, "宿主日志回调必须被真实调用一次");
    let (level, text, nul_at_len) = &calls[0];
    assert_eq!(
        *level, 3,
        "warn 必须映射为 level 3（与宿主 default_c_logger 约定一致）"
    );
    assert!(
        text.contains("回退会话已启用"),
        "事件内容必须到达宿主回调: {text}"
    );
    assert!(
        text.contains("backend"),
        "结构化字段必须随消息一并转发: {text}"
    );
    assert!(
        *nul_at_len,
        "msg 必须以 NUL 结尾（宿主用 CStr::from_ptr 读取）"
    );

    // B8：library_close 之后不得再回调宿主。
    // SAFETY: lib 句柄有效
    let close_status = unsafe { (abi.library_close.expect("close"))(lib) };
    assert_eq!(close_status, AV_OK);

    tracing::warn!("库关闭后的日志不应到达宿主");
    assert_eq!(
        LOG_CALLS.lock().expect("log calls lock").len(),
        1,
        "library_close 之后不得再回调宿主"
    );
}
