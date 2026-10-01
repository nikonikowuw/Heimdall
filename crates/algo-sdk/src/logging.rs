//! 宿主日志桥接：把插件内的 `tracing` 事件转发到宿主 `AvLogFn`
//!
//! 宿主在 `library_open` 时通过 [`crate::c_abi::AvAlgoLibraryArgs::log`] 提供一个
//! C 日志回调（见 `crates/infer/src/c_abi/loader.rs` 的 `default_c_logger`）。本模块负责：
//!
//! 1. 在 `library_open` 时登记该回调并取得一个 RAII 持有权
//!    （[`HostLogBridgeLease`]）；
//! 2. 安装一个把本 SDK（及其所在 cdylib）的 `tracing` 事件按级别转发过去的
//!    subscriber。
//!
//! # 为什么必须在插件内部安装 subscriber
//!
//! 算法包以 `cdylib` 交付，拥有**独立的 Cargo workspace 与独立 lock 文件**，
//! `tracing-core` 被**静态链入每个 `.so` 的私有副本**，且 `.so` 不导出任何
//! tracing 相关符号（实测导出符号仅含 `av_algo_*` 入口，人脸包另含
//! `av_algo_get_gallery_abi` / `av_algo_gallery_bulk`，tracing 符号数为 0）。
//!
//! 因此宿主的 `tracing_subscriber::registry().init()` 只作用于**宿主自己的**
//! `tracing-core`，宿主的 dispatcher 无法被插件复用；插件若不自行安装 subscriber，
//! 其 `tracing` 事件会回落到 no-op sink 并被静默丢弃。这是本模块存在的唯一理由。
//!
//! # 与既有 subscriber 共存
//!
//! `set_global_default` 进程内只能成功一次。插件若在 `run_local` 等场景已自行
//! 安装 subscriber，本模块的安装会返回 `Err` 并被**忽略**（见
//! [`install_host_log_subscriber`]），绝不 panic、绝不覆盖已有 dispatcher。
//!
//! 反向约束同样存在：**先**以 `log = Some` 打开算法库、**后**再调用
//! `tracing_subscriber::fmt().init()` 的可执行文件会因全局订阅器已被占用而 panic。
//! 仓库内不存在这种二进制（`run_local` 等探针走 rlib，不调用 `library_open`），
//! 且该组合只会出现在开发脚手架中，因此保持“共存优先于独占”的既有取舍。
//!
//! # 不做 target 过滤
//!
//! 设计草案曾讨论按 `target` 前缀过滤。cdylib 拥有**私有的** `tracing-core`
//! 副本，本订阅器只能看到**本插件自身**（SDK + 算法包代码）发出的事件，
//! 宿主事件根本不会进入这个 dispatcher，因此不存在“劫持宿主事件”的风险，
//! 过滤无对象。若将来某个依赖包出现高频噪声，再按 `target` 前缀裁剪。

use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;

use tracing::field::{Field, Visit};
use tracing::span::Id;
use tracing::{Event, Level, Metadata, Subscriber};

use crate::c_abi::AvLogFn;

/// 宿主日志回调的进程内注册点。
///
/// `log` 与 `user` 由宿主在 `library_open` 期间提供，并在 `library_close` 之前
/// 保持有效（宿主契约：库句柄与回调资源必须覆盖实例有效期）。
#[derive(Clone, Copy)]
pub struct HostLogSink {
    /// 宿主提供的日志函数指针。
    pub log: AvLogFn,
    /// 宿主不透明上下文，原样回传。
    pub user: *mut c_void,
}

impl std::fmt::Debug for HostLogSink {
    /// 刻意不打印函数指针与 `user` 地址（会泄露 ASLR 布局，也无可诊断价值）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostLogSink").finish_non_exhaustive()
    }
}

// SAFETY: `HostLogSink` 只承载宿主提供的函数指针与不透明上下文，桥接层只做
// 原子读取与调用，不对其解引用。跨线程性由宿主契约保证：`AvLogFn` 必须是
// 线程安全且可在任意调用线程上执行的函数（宿主 `default_c_logger` 即如此），
// `user` 指向的上下文必须在库句柄存活期内保持有效。转发可能发生在插件任意
// 线程上（如 RKNN 的推理 Worker），这是本桥接的存在前提。
unsafe impl Send for HostLogSink {}
// SAFETY: 同 `Send` 的说明。
unsafe impl Sync for HostLogSink {}

/// 当前宿主回调。回调在进程生命周期内稳定（宿主始终传同一 `default_c_logger`），
/// 因此只在首次写入时生效；**刻意不做真实释放**，避免 `library_close` 后的
/// use-after-free。
static HOST_LOG_SINK: OnceLock<HostLogSink> = OnceLock::new();

/// 当前存活的、已登记日志回调的库句柄数量。
///
/// 桥接的活跃性**没有独立标志位**，直接由本计数派生（见 `active_sink`）。
/// 不另设 `AtomicBool` 的原因：两个独立状态源之间存在丢失更新窗口——`release`
/// 把计数减到 0 的同时另一线程 `register`，前者的 `store(false)` 可能落在后者的
/// `store(true)` 之后，造成「计数为 1 但桥接已关」，新句柄的日志被静默丢弃。
/// 单一状态源从根本上消除该竞态。
///
/// **为何需要引用计数**：宿主的 `RawAlgoLibrary::drop` 会在**每次**释放时调用
/// `library_close`（见 `crates/infer/src/c_abi/loader.rs`），而 `RawAlgoLibrary::open`
/// 在宿主内有多个调用点：常驻 Worker 长期持有一个句柄
/// （`package.rs` 的 `create_worker`），`extract_face` / `create_gallery` 则会
/// 再开一个短命句柄、用完即关。若 `library_close` 无条件停用桥接，
/// 一次 `extract_face` 就会把常驻 Worker 的日志静默掐断——正是本模块要消灭的
/// “静默降级”。因此只在计数归零（最后一个句柄释放）时才真正停用。
/// 这与 [`crate::cv::DefaultEngineLease`] 处理同类进程级资源的手法一致。
static OPEN_LIBRARY_COUNT: AtomicUsize = AtomicUsize::new(0);

/// 一个库句柄对日志桥接的持有权（RAII，零尺寸）。
///
/// 由 [`register_host_log_sink`] 产出，必须随 `LibraryContext` 一同存活、
/// 一同析构；析构即释放一个持有权。
///
/// # 为什么必须是 RAII，而不是在 `library_close` 里手工配对
///
/// `library_open` 存在**不产生库句柄**的失败返回路径：`open_hook` 返回 `Err`
/// （例如 `shared_models()` 的 `ModelLoad` 失败）或在其内部 panic。这些路径下
/// 宿主拿不到句柄，因而**不会**调用 `library_close`（`crates/infer/src/c_abi/loader.rs`
/// 只在 `raw_lib` 非空时关闭）。若持有权靠 `library_close` 手工释放，每次失败
/// 加载都会永久泄漏一个计数，最终使计数永不归零——最后一个真实句柄关闭后，
/// 插件日志仍会继续转发给宿主回调，即 `active_sink()` 的活跃性判据失效。
///
/// 绑定到 `LibraryContext` 的析构后，正常返回、失败返回与 unwind 三条路径
/// 全部自动配对。
///
/// 本类型刻意**零尺寸**：它只代表一个计数配额，不复制回调负载（回调保存在
/// `HOST_LOG_SINK` 中），因此 `LibraryContext` 不会出现两份指针副本。
#[must_use = "持有权被立即丢弃会让桥接在库句柄存活期间停用；必须随 LibraryContext 一同存活"]
pub struct HostLogBridgeLease {
    /// 私有字段：禁止模块外构造，保证计数只能由 [`register_host_log_sink`] 递增。
    _private: (),
}

impl std::fmt::Debug for HostLogBridgeLease {
    /// 只表明持有权存在，不打印任何地址或计数（无可诊断价值且可能泄露布局）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostLogBridgeLease")
    }
}

impl Drop for HostLogBridgeLease {
    fn drop(&mut self) {
        release_host_log_bridge();
    }
}

/// 登记宿主日志回调，并返回一个库句柄持有权。
///
/// `library_open` 会被多次调用（同一包的多实例、多线程握手，以及 `extract_face`
/// 等短操作），而本槽位是 `OnceLock`；宿主契约要求回调与 `user` 在进程内稳定
/// （见 `crates/infer/src/c_abi/loader.rs` 始终传同一 `default_c_logger`），
/// 因此首次写入即代表最终状态，后续写入是被忽略的幂等操作。
///
/// 调用方**必须**保存返回的持有权直到库句柄析构，否则桥接会在句柄仍存活时
/// 提前停用。
pub fn register_host_log_sink(log: AvLogFn, user: *mut c_void) -> HostLogBridgeLease {
    let _ = HOST_LOG_SINK.set(HostLogSink { log, user });
    OPEN_LIBRARY_COUNT.fetch_add(1, Ordering::AcqRel);
    HostLogBridgeLease { _private: () }
}

/// 释放一个库句柄对桥接的持有权。
///
/// 由 [`HostLogBridgeLease::drop`] 调用；只在**最后一个**库句柄释放后停用转发：
/// 常驻 Worker 持有的句柄会让桥接持续存活，而 `extract_face` 之类短操作的
/// open/close 净影响为零。
///
/// 只递减计数，不释放回调：回调的所有权属于宿主，插件不得释放或改写它。
/// 计数归零时 `active_sink` 自然返回 `None`。
///
/// 不公开：外部若绕过持有权直接调用，会在常驻句柄仍存活时把日志关掉。
fn release_host_log_bridge() {
    // 饱和递减：计数为 0（如未配对的 close）时拒绝递减而非回绕，
    // 回绕会让计数变成天文数字从而永不归零。
    let _ = OPEN_LIBRARY_COUNT.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
        count.checked_sub(1)
    });
}

/// 强制停用转发，**仅用于测试**。
///
/// 刻意不公开：外部若误用它而不是持有权，会绕开引用计数，在常驻库句柄仍存活时
/// 把日志关掉——正是本模块要消灭的缺陷模式。
#[cfg(test)]
fn deactivate_host_log_bridge() {
    OPEN_LIBRARY_COUNT.store(0, Ordering::Release);
}

/// 读取当前活跃的宿主回调；无库句柄存活时返回 `None`。
fn active_sink() -> Option<HostLogSink> {
    // 计数是唯一活跃性判据：>0 说明至少有一个已登记的库句柄存活。
    // `Acquire` 与 `register` 的 `AcqRel` 配对，保证看到计数时也能看到已写入的 sink。
    if OPEN_LIBRARY_COUNT.load(Ordering::Acquire) == 0 {
        return None;
    }
    HOST_LOG_SINK.get().copied()
}

/// 安装转发 subscriber（幂等）。
///
/// 返回是否由本次调用成功安装。任何失败（已安装全局 subscriber、插件自身已
/// `fmt().init()`）都被**忽略**而非 panic——共存优先于独占。
pub fn install_host_log_subscriber() -> bool {
    tracing::subscriber::set_global_default(HostLogSubscriber::default()).is_ok()
}

/// `tracing` 级别 → 宿主 `AvLogFn` 的 `level` 取值。
///
/// **这是与宿主的双向契约**，必须与 `crates/infer/src/c_abi/loader.rs` 的
/// `default_c_logger` 保持一致（0=trace, 1=debug, 2=info, 3=warn, ≥4=error）。
/// 修改任一侧都必须同步另一侧与其表驱动测试。
fn level_to_c(level: &Level) -> c_int {
    if *level == Level::ERROR {
        4
    } else if *level == Level::WARN {
        3
    } else if *level == Level::INFO {
        2
    } else if *level == Level::DEBUG {
        1
    } else {
        0
    }
}

thread_local! {
    /// 复用的事件渲染缓冲。日志可能高频，热路径不得每事件重新分配：
    /// 这里的三块内存在首次使用后即保留容量，`clear()` 不释放。
    static LOG_BUFFERS: RefCell<LogBuffers> = RefCell::new(LogBuffers::default());
}

#[derive(Default)]
struct LogBuffers {
    /// `message` 字段渲染结果。
    message: String,
    /// 其余结构化字段，形如 ` reason=... frame_id=...`。
    fields: String,
    /// 最终交给宿主的 NUL 结尾字节缓冲。
    bytes: Vec<u8>,
}

/// 把 `Event` 的字段渲染进调用方提供的缓冲。
///
/// 除 `message` 外的字段以 `key=value` 追加：算法包的诊断日志普遍形如
/// `tracing::warn!(reason = ?e, "未检测到物理 librknnrt.so，...")`，
/// 丢掉 `reason` 会让日志失去诊断价值。
struct EventVisitor<'a> {
    message: &'a mut String,
    fields: &'a mut String,
}

impl Visit for EventVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            // 直接用 write! 写入目标缓冲，避免 format! 产生中间 String。
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.fields, " {}={}", field.name(), value);
        }
    }
}

/// 转发一个 `tracing` 事件到宿主回调。
fn forward_event(sink: HostLogSink, level: c_int, event: &Event<'_>) {
    LOG_BUFFERS.with(|cell| {
        // 用 try_borrow_mut：万一宿主回调经某种路径重入本模块，宁可丢弃这条日志，
        // 也不能因 RefCell 双重借用而 panic（panic 不得跨 FFI 边界）。
        let Ok(mut buffers) = cell.try_borrow_mut() else {
            return;
        };

        // 拆借各字段，保证 message / fields / bytes 之间是互不重叠的可变借用。
        let LogBuffers {
            message,
            fields,
            bytes,
        } = &mut *buffers;

        message.clear();
        fields.clear();

        {
            let mut visitor = EventVisitor { message, fields };
            event.record(&mut visitor);
        }

        if !fields.is_empty() {
            message.push_str(fields);
        }

        bytes.clear();
        if message.as_bytes().contains(&0) {
            // 宿主用 `CStr::from_ptr` 读取，内部 NUL 会让宿主截断消息（不是越界，
            // 但会静默丢失后半段）。替换为 U+FFFD 使截断可见且保持有效 UTF-8。
            bytes.extend_from_slice(message.replace('\0', "\u{FFFD}").as_bytes());
        } else {
            bytes.extend_from_slice(message.as_bytes());
        }
        // 宿主忽略 len 参数、按 NUL 结尾读取，因此终止符是硬要求；
        // 同时仍按契约填写不含终止符的正确字节数。
        bytes.push(0);
        let len = (bytes.len() - 1) as u32;
        let msg = bytes.as_ptr().cast::<c_char>();

        // SAFETY: `msg` 指向本线程局部缓冲，在本次同步调用期间非空且以 0 结尾，
        // `len` 为不含终止符的字节数，符合 AvLogFn 契约；`sink.log` 由宿主提供
        // 并保证在库生命周期内有效。
        //
        // **`catch_unwind` 的实际保护范围有限**（已实测）：`AvLogFn` 是 `extern "C"`，
        // 回调内部 panic 会在它自身的不可 unwind 守卫处直接 abort，根本到不了这里
        //（`panic in a function that cannot unwind` → SIGABRT）。保留它是为了兜住
        // 闭包体自身的 unwind，并在 `AvLogFn` 未来迁移到 `extern "C-unwind"` 时继续生效。
        // **宿主回调的 panic 隔离由宿主自己完成**：`default_c_logger` 已将函数体包在
        // `catch_unwind` 内（见 `crates/infer/src/c_abi/loader.rs`）。
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            (sink.log)(sink.user, level, msg, len);
        }));
    });
}

/// 把插件内 `tracing` 事件转发到宿主回调的最小 `Subscriber`。
///
/// 刻意不引入 `tracing-subscriber`：它只用于注册与转发这一个目的，
/// 引入会让每个算法包的 cdylib 体积无谓增大。本实现只依赖 `tracing`
/// 已 re-export 的 core 类型，**零新增依赖**。
#[derive(Debug, Default)]
struct HostLogSubscriber {
    next_span_id: AtomicU64,
}

impl Subscriber for HostLogSubscriber {
    fn register_callsite(
        &self,
        _metadata: &'static Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        // 必须 always：callsite 的 interest 会被 tracing 缓存，若在桥接尚未激活时
        // 返回 never，该 callsite 会被永久禁用，之后登记了回调也不会再触发。
        // 激活判定因此下沉到 `event()`，那里是每事件求值的。
        tracing::subscriber::Interest::always()
    }

    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        active_sink().is_some()
    }

    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> Id {
        // 本转发器不消费 span，只需返回唯一非零 Id 以维持 tracing 内部不变量。
        let next = self.next_span_id.fetch_add(1, Ordering::Relaxed);
        Id::from_u64(next.saturating_add(1))
    }

    fn record(&self, _span: &Id, _values: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let Some(sink) = active_sink() else {
            return;
        };
        forward_event(sink, level_to_c(event.metadata().level()), event);
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 记录到的回调调用：(level, 消息, msg[len] 是否为 NUL)。
    static RECORDED: Mutex<Vec<(c_int, String, bool)>> = Mutex::new(Vec::new());

    /// 串行化共享静态状态的用例（cargo test 默认多线程执行）。
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    // ⚠️ 用例编写约束：`HOST_LOG_SINK` 是 `OnceLock`（set-once，与生产语义一致：
    // 宿主始终传同一个稳定回调）。因此**所有用例必须登记同一个 `mock_logger`**。
    // 若某个用例登记了别的回调，谁先跑到就永久胜出，其余用例的断言会静默失真，
    // 且无法复现（实测过一个按此写法的用例，约 1/190 概率把进程打崩）。
    //
    // 也因此**不要**在这里加「回调 panic 是否被隔离」的用例：
    // `AvLogFn` 是 `extern "C"`，回调内 panic 会直接 abort（SIGABRT），
    // 外层 `catch_unwind` 无法介入，根本无法断言。已实测确认。

    unsafe extern "C" fn mock_logger(
        user: *mut c_void,
        level: c_int,
        msg: *const c_char,
        len: u32,
    ) {
        // `user` 在单测里当作 (level 无关的) 计数器使用，这里只验证非空回传。
        if user.is_null() || msg.is_null() {
            return;
        }
        // SAFETY: 桥接层保证 msg 指向本调用期有效、以 0 结尾的缓冲。
        let text = unsafe { std::ffi::CStr::from_ptr(msg) }
            .to_string_lossy()
            .into_owned();
        // 判定宿主实际读取的位置（第 len 字节）确实是终止符——这正是
        // `CStr::from_ptr` 的读取前提，如果插件漏补 NUL，宿主就会越界读。
        // SAFETY: 桥接层保证缓冲至少有 len + 1 字节。
        let nul_at_len = unsafe { *msg.add(len as usize) } == 0;
        RECORDED
            .lock()
            .expect("recorded lock")
            .push((level, text, nul_at_len));
    }

    /// 在「已登记 mock 回调 + 桥接活跃」的前提下，用线程局部 subscriber 执行闭包。
    ///
    /// 用 `with_default` 而不是全局安装，使用例彼此隔离、可并行。
    /// 持有权随本函数返回而释放，因此用例结束后桥接不会被永久激活。
    fn with_bridge<R>(f: impl FnOnce() -> R) -> R {
        let _lease = register_host_log_sink(mock_logger, test_user());
        with_subscriber(f)
    }

    /// 只安装线程局部 subscriber，**不**改变桥接活跃状态。
    ///
    /// 测试停用路径时必须用它：`with_bridge` 会重新登记并激活，
    /// 从而把 `deactivate_host_log_bridge` 的效果掩盖掉。
    fn with_subscriber<R>(f: impl FnOnce() -> R) -> R {
        let subscriber = HostLogSubscriber::default();
        tracing::subscriber::with_default(subscriber, f)
    }

    /// 单测不关心 `user` 语义，只验证它被原样回传（非空即可）。
    fn test_user() -> *mut c_void {
        std::ptr::NonNull::<u8>::dangling().as_ptr().cast()
    }

    fn take_recorded() -> Vec<(c_int, String, bool)> {
        std::mem::take(&mut *RECORDED.lock().expect("recorded lock"))
    }

    /// B1 / B2：登记回调后发出的事件必须真正到达回调，且内容可见。
    #[test]
    fn registered_callback_receives_event_content() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = take_recorded();

        with_bridge(|| tracing::warn!(reason = "缺少 librknnrt.so", "启用开发调试回退会话"));

        let recorded = take_recorded();
        assert_eq!(recorded.len(), 1, "回调必须被调用且只调用一次");
        let (level, text, nul_at_len) = &recorded[0];
        assert_eq!(*level, 3, "warn 必须映射为 level 3");
        assert!(
            text.contains("启用开发调试回退会话"),
            "消息内容必须到达回调: {text}"
        );
        assert!(
            text.contains("reason"),
            "结构化字段 reason 必须随消息一并转发: {text}"
        );
        assert!(nul_at_len, "宿主读取位置必须是 NUL 终止符");
    }

    /// B3：五级映射表驱动验证，与宿主 `default_c_logger` 的约定一致。
    #[test]
    fn level_mapping_matches_host_contract() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = take_recorded();

        with_bridge(|| {
            tracing::trace!("t");
            tracing::debug!("d");
            tracing::info!("i");
            tracing::warn!("w");
            tracing::error!("e");
        });

        let levels: Vec<c_int> = take_recorded().iter().map(|(l, _, _)| *l).collect();
        assert_eq!(levels, vec![0, 1, 2, 3, 4], "级别映射必须与宿主一致");
    }

    /// B4：消息必须以 NUL 结尾；含内部 NUL 的消息不得 panic 或越界。
    #[test]
    fn message_is_nul_terminated_and_interior_nul_is_sanitized() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = take_recorded();

        with_bridge(|| tracing::warn!("前段\u{0}后段"));

        let recorded = take_recorded();
        assert_eq!(recorded.len(), 1, "含内部 NUL 的消息也必须送达且不得 panic");
        let (_, text, nul_at_len) = &recorded[0];
        assert!(nul_at_len, "缓冲必须以 NUL 结尾");
        assert!(
            !text.contains('\u{0}'),
            "内部 NUL 必须被替换，避免宿主 CStr 读取时静默截断: {text:?}"
        );
        assert!(
            text.contains("前段") && text.contains("后段"),
            "替换后两段内容都应保留: {text:?}"
        );
    }

    /// B5：未登记 / 已停用时退化为无操作，业务路径不受影响。
    #[test]
    fn inactive_bridge_is_a_noop() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = take_recorded();

        deactivate_host_log_bridge();
        // 用 with_subscriber 而非 with_bridge：后者会重新登记并激活桥接。
        with_subscriber(|| tracing::warn!("库关闭后的日志"));
        assert!(take_recorded().is_empty(), "停用后不得再回调宿主");

        // 恢复活跃，避免影响后续用例。
        let _lease = register_host_log_sink(mock_logger, test_user());
        with_subscriber(|| tracing::warn!("重新激活"));
        assert_eq!(take_recorded().len(), 1, "重新登记后应恢复转发");
    }

    /// B5 后半句：`log = None` 时**不登记、不计数**。
    ///
    /// 这是 `plugin_lifecycle.rs::test_library_open_without_log_callback_does_not_register`
    /// 的白盒对照：那里走真实 C ABI，这里直接验证计数语义。
    /// 两侧必须一致，否则当宿主不传回调时 `library_close` 会多减一次。
    #[test]
    fn none_callback_registers_nothing() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        deactivate_host_log_bridge();

        // 未登记任何回调：桥接必须保持关闭，且计数为 0（不因未配对释放而异常）。
        with_subscriber(|| tracing::warn!("无回调时的日志"));
        assert!(take_recorded().is_empty(), "未登记时不得转发");
        release_host_log_bridge();
        with_subscriber(|| tracing::warn!("未登记时再发一次"));
        assert!(
            take_recorded().is_empty(),
            "未登记 + 未配对释放后仍不得转发（计数不得回绕）"
        );
    }

    /// B6：既有 subscriber 已占用全局槽位时，安装必须让路而非 panic。
    #[test]
    fn install_is_idempotent_and_never_panics() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lease = register_host_log_sink(mock_logger, test_user());

        // 首次可能成功，后续必然失败；两者都不得 panic。
        let _first = install_host_log_subscriber();
        let second = install_host_log_subscriber();
        assert!(!second, "全局 subscriber 只能安装一次，后续必须返回 false");
        // 关闭与重开不得 panic。
        deactivate_host_log_bridge();
        let _second_lease = register_host_log_sink(mock_logger, test_user());
    }

    /// B8：重复 open/close 级别的登记与停用不得泄漏或 panic。
    #[test]
    fn repeated_register_and_deactivate_is_safe() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = take_recorded();

        for _ in 0..64 {
            let lease = register_host_log_sink(mock_logger, test_user());
            with_subscriber(|| tracing::warn!("循环"));
            deactivate_host_log_bridge();
            // 停用后必须用 with_subscriber，否则重新登记会重新激活。
            with_subscriber(|| tracing::warn!("已停用"));
            // 持有权析构时计数已为 0，必须饱和而不会回绕。
            drop(lease);
        }

        // 每轮只有活跃期内那一条被记录。
        let recorded = take_recorded();
        assert_eq!(recorded.len(), 64, "只应记录活跃期内的日志");

        let _lease = register_host_log_sink(mock_logger, test_user());
    }

    #[test]
    fn level_mapping_is_total_over_all_levels() {
        assert_eq!(level_to_c(&Level::TRACE), 0);
        assert_eq!(level_to_c(&Level::DEBUG), 1);
        assert_eq!(level_to_c(&Level::INFO), 2);
        assert_eq!(level_to_c(&Level::WARN), 3);
        assert_eq!(level_to_c(&Level::ERROR), 4);
    }

    /// 持有权是零尺寸的：它只是一个计数配额，**不**复制回调负载。
    ///
    /// 锁住的是评审中提出的“同一 (fn, user) 载荷存两份”的回归：负载只保存在
    /// [`HOST_LOG_SINK`] 里，`LibraryContext` 不再携带一份指针副本。
    #[test]
    fn lease_carries_no_callback_payload() {
        static_assertions::assert_eq_size!(HostLogBridgeLease, ());
    }

    /// **P1 回归锁**：持有权必须在**析构**时释放，而不是依赖 `library_close`。
    ///
    /// `library_open` 存在不产生句柄的失败路径（`open_hook` 返回 `Err` 或 panic），
    /// 那些路径下宿主不会调用 `library_close`。若计数靠 close 手工释放，每次失败
    /// 加载都会泄漏一个计数，最终使桥接永不停用。
    /// 端到端版本见 `plugin_lifecycle.rs::test_failed_open_does_not_leak_log_bridge`。
    #[test]
    fn lease_drop_releases_exactly_one_holding() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = take_recorded();
        deactivate_host_log_bridge();

        let first = register_host_log_sink(mock_logger, test_user());
        let second = register_host_log_sink(mock_logger, test_user());

        // 丢弃一个持有权：另一个仍在，桥接必须继续转发。
        drop(first);
        with_subscriber(|| tracing::warn!("仍有一个持有权"));
        assert_eq!(take_recorded().len(), 1, "仍有持有权时不得停用");

        // 丢弃最后一个：桥接停用。
        drop(second);
        with_subscriber(|| tracing::warn!("持有权已全部释放"));
        assert!(
            take_recorded().is_empty(),
            "持有权全部析构后必须停用（失败 open 路径也因此不再泄漏）"
        );
    }

    /// 回归：桥接必须靠引用计数存活，直到**最后一个**库句柄释放才停用。
    ///
    /// 锁住的是真实宿主行为：常驻 Worker 长期持有一个 `RawAlgoLibrary`，而
    /// `extract_face` / `create_gallery` 会再开一个短命句柄、用完即 drop。
    /// 若释放时无条件停用，一次 `extract_face` 就会把常驻推理的日志全部静默丢弃。
    #[test]
    fn bridge_stays_active_until_last_handle_released() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        deactivate_host_log_bridge();

        // 两个句柄：常驻 Worker（A）与 extract_face 短操作（B）。
        let resident = register_host_log_sink(mock_logger, test_user());
        let short_lived = register_host_log_sink(mock_logger, test_user());

        // B 用完先关：A 仍存活，桥接必须继续转发。
        drop(short_lived);
        let _ = take_recorded();
        with_subscriber(|| tracing::warn!("常驻 Worker 仍存活"));
        assert_eq!(
            take_recorded().len(),
            1,
            "还有库句柄存活时不得停用桥接（否则短操作会搞死常驻日志）"
        );

        // A 也关：计数归零，真正停用。
        drop(resident);
        with_subscriber(|| tracing::warn!("全部句柄已释放"));
        assert!(take_recorded().is_empty(), "最后一个句柄释放后才应停用");
    }

    /// 未配对的 close 必须饱和而非回绕（回绕会让计数永不归零）。
    #[test]
    fn release_without_register_saturates_at_zero() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        deactivate_host_log_bridge();

        // 计数为 0 时释放不得 panic、不得下溢。
        release_host_log_bridge();
        release_host_log_bridge();

        // 之后登记仍能正常激活（计数未被回绕成天文数字）。
        let _ = take_recorded();
        let _lease = register_host_log_sink(mock_logger, test_user());
        with_subscriber(|| tracing::warn!("登记后应恢复正常"));
        assert_eq!(take_recorded().len(), 1, "计数不得因未配对释放而损坏");
    }
}
