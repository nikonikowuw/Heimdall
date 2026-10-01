//! `library_open` 失败路径的日志桥接回归测试。
//!
//! 本文件独立成二进制（与 `plugin_lifecycle.rs` 分离），因为它需要导出一个
//! **带可失败 open hook** 的插件：`export_algo!` 的导出符号名固定，同一二进制
//! 内不能展开两次。
//!
//! # 锁住的行为
//!
//! `library_open` 存在**不产生库句柄**的失败返回路径（`open_hook` 返回 `Err`）。
//! 这些路径下宿主拿不到句柄，因而**不会**调用 `library_close`
//! （`crates/infer/src/c_abi/loader.rs` 只在 `raw_lib` 非空时才关闭）。
//!
//! 日志桥接的持有权因此必须是 **RAII** 的（随 `LibraryContext` 析构释放）。
//! 若改为在 `library_close` 里手工配对，每次失败加载都会永久泄漏一个计数，
//! 最终使计数永不归零——最后一个真实句柄关闭后，插件日志仍会继续转发给宿主。
//! 该缺陷曾以独立探针实测复现，本文件是其端到端回归锁。

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use algo_sdk::prelude::*;
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
struct MockConfig {}

struct MockPlugin;

impl AlgoPlugin for MockPlugin {
    type Config = MockConfig;

    fn init(_ctx: &InitContext<'_>, _config: Self::Config) -> Result<Self, AlgoError> {
        Ok(Self)
    }

    fn process(
        &mut self,
        _frame: SafeFrame<'_>,
        _emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        Ok(())
    }
}

/// 置位时让 `library_open` 在登记日志桥接**之后**失败。
///
/// 这正是真实场景：`shared_models()` 会因 `ModelLoad`（模型缺失）或锁中毒失败，
/// 而该调用发生在桥接登记之后、句柄返回之前。
static FAIL_OPEN_HOOK: AtomicBool = AtomicBool::new(false);

fn failing_open_hook(_package_root: &std::path::Path) -> Result<(), AlgoError> {
    if FAIL_OPEN_HOOK.load(Ordering::SeqCst) {
        return Err(AlgoError::ModelLoad {
            reason: "测试：模拟模型加载失败".to_string(),
        });
    }
    Ok(())
}

fn noop_close_hook(_package_root: &std::path::Path) {}

export_algo!(
    MockPlugin,
    algo_id: "mock_open_failure",
    version: "1.0.0",
    algo_type: "object_detection",
    alarm_type_id: "intrusion",
    library_open_hook: failing_open_hook,
    library_close_hook: noop_close_hook
);

/// 串行化：桥接登记槽位与计数是进程级全局状态。
static LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());

/// 宿主日志回调收到的调用次数。
static LOG_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn counting_logger(
    _user: *mut c_void,
    _level: c_int,
    msg: *const c_char,
    len: u32,
) {
    if msg.is_null() {
        return;
    }
    // SAFETY: 桥接层保证 msg 指向本调用期有效、以 0 结尾的缓冲。
    let _ = unsafe { CStr::from_ptr(msg) };
    // SAFETY: 桥接层保证缓冲至少有 len + 1 字节。
    let _ = unsafe { *msg.add(len as usize) };
    LOG_CALLS.fetch_add(1, Ordering::SeqCst);
}

/// 发一条事件并返回宿主回调被调用的次数。
fn emit(tag: &str) -> usize {
    LOG_CALLS.store(0, Ordering::SeqCst);
    tracing::warn!(probe = tag, "失败路径回归探针");
    LOG_CALLS.load(Ordering::SeqCst)
}

/// 经真实 ABI 虚表调用 `library_open`；`log_some` 控制是否提供日志回调。
fn open(abi: &AvAlgoAbi, log_some: bool) -> (c_int, AvAlgoLibrary) {
    let pkg_root = CString::new("/tmp/mock_open_failure_pkg").expect("cstring");
    let platform = CString::new("rk3588").expect("cstring");
    let open_args = AvAlgoLibraryArgs {
        size: std::mem::size_of::<AvAlgoLibraryArgs>() as u32,
        api_version: AV_ALGO_API_VERSION,
        package_root: pkg_root.as_ptr(),
        platform_id: platform.as_ptr(),
        platform_tag: 0,
        log: if log_some {
            Some(counting_logger)
        } else {
            None
        },
        log_user: std::ptr::null_mut(),
    };

    let mut lib: AvAlgoLibrary = std::ptr::null_mut();
    // SAFETY: 传入合法的 open_args 与 out 指针。
    let status = unsafe { (abi.library_open.expect("open"))(&open_args, &mut lib) };
    (status, lib)
}

fn close(abi: &AvAlgoAbi, lib: AvAlgoLibrary) {
    if !lib.is_null() {
        // SAFETY: lib 由 library_open 返回且尚未释放。
        unsafe { (abi.library_close.expect("close"))(lib) };
    }
}

/// **P1 回归锁（端到端）**：`open_hook` 失败时不得泄漏桥接计数。
///
/// 修复前该用例失败：失败 open 会让计数永久 +1，导致最后一个真实句柄关闭后
/// 事件仍被转发（实测返回 1，期望 0）。
#[test]
fn failed_open_does_not_leak_log_bridge() {
    let _guard = LIFECYCLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // SAFETY: 调取本模块导出的 ABI 虚表指针。
    let abi = unsafe { &*av_algo_get_abi(AV_ALGO_API_VERSION) };

    assert_eq!(emit("baseline"), 0, "无任何句柄时不得转发");

    // 1) 失败 open：宿主拿不到句柄，因此永远不会调用 library_close。
    FAIL_OPEN_HOOK.store(true, Ordering::SeqCst);
    let (status, lib) = open(abi, true);
    assert_ne!(status, AV_OK, "open_hook 失败时 library_open 必须返回错误");
    assert!(lib.is_null(), "失败路径不得交出句柄");
    assert_eq!(
        emit("after-failed-open"),
        0,
        "失败 open 不得留下持有权（否则计数永不归零）"
    );

    // 2) 正常 open —— 与失败路径交替，锁住“计数不多不少”。
    FAIL_OPEN_HOOK.store(false, Ordering::SeqCst);
    let (status, lib) = open(abi, true);
    assert_eq!(status, AV_OK, "hook 恢复后 open 必须成功");
    assert!(!lib.is_null());
    assert_eq!(emit("live-handle"), 1, "存活句柄期间必须转发");

    // 3) 关闭唯一的真实句柄：计数必须归零。
    close(abi, lib);
    assert_eq!(
        emit("after-last-close"),
        0,
        "最后一个真实句柄关闭后必须停用（失败 open 的残留持有权会让这里为 1）"
    );

    // 4) 反复失败不得累积：再失败 8 次后行为仍与首次一致。
    FAIL_OPEN_HOOK.store(true, Ordering::SeqCst);
    for _ in 0..8 {
        let (status, lib) = open(abi, true);
        assert_ne!(status, AV_OK);
        assert!(lib.is_null());
    }
    assert_eq!(
        emit("after-repeated-failed-opens"),
        0,
        "失败 open 不得累积计数"
    );

    // 5) 失败若干次后，正常 open/close 仍能正确回到关闭态。
    FAIL_OPEN_HOOK.store(false, Ordering::SeqCst);
    let (status, lib) = open(abi, true);
    assert_eq!(status, AV_OK);
    assert_eq!(emit("recovered"), 1);
    close(abi, lib);
    assert_eq!(emit("final"), 0, "恢复正常后仍必须回到停用态");
}

/// **B5 后半句（端到端）**：`log = None` 时**不登记、不计数**，其 `close` 不递减。
///
/// 判别力来自用例顺序：若 `log = None` 的 open 不计数的同时其 `close` 却递减，
/// 就会额外扣掉**另一个**存活句柄的配额，使桥接在仍有句柄时提前停用（第 4 步
/// 会读到 0 次调用而不是 1 次）。
#[test]
fn none_log_close_does_not_decrement_other_handles() {
    let _guard = LIFECYCLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    FAIL_OPEN_HOOK.store(false, Ordering::SeqCst);

    // SAFETY: 调取本模块导出的 ABI 虚表指针。
    let abi = unsafe { &*av_algo_get_abi(AV_ALGO_API_VERSION) };
    let _ = emit("reset");

    // 1) 携带回调的“常驻”句柄 A。
    let (status, resident) = open(abi, true);
    assert_eq!(status, AV_OK);
    assert!(!resident.is_null());

    // 2) 不带回调的短命句柄 B（宿主未提供 log）。
    let (status, bare) = open(abi, false);
    assert_eq!(status, AV_OK, "log = None 时 open 仍必须成功");
    assert!(!bare.is_null());

    // 3) 关闭 B：它从未登记，因此不得递减计数。
    close(abi, bare);

    // 4) 关键断言：A 仍存活，桥接必须继续转发。
    assert_eq!(
        emit("resident-still-alive"),
        1,
        "log = None 的 close 递减了别人的持有权，使桥接提前停用"
    );

    // 5) 关闭 A：此时才应停用。
    close(abi, resident);
    assert_eq!(emit("all-closed"), 0, "全部句柄关闭后必须停用");
}
