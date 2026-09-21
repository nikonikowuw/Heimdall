//! 视觉预处理硬件抽象层 (HAL & SPI)
//! 提供统一门面函数 `cv::letterbox` / `cv::resize`，底层自动分派到最佳硬件加速器。

use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

pub mod buffer;
pub mod engine;
pub mod layout;
pub mod platforms;
pub mod postprocess;
pub mod types;

pub use buffer::{CvBuffer, DmaBufLayout};
pub use engine::CvEngine;
pub use layout::{compute_letterbox_layout, compute_stretch_layout};
pub use types::{CropRect, LetterboxLayout, PixelFormat, PreprocessMode};

use crate::c_abi::AvImageOps;
use crate::error::AlgoError;
use crate::frame::SafeFrame;

/// 由一个插件实例独占的预处理引擎引用。
pub type SharedCvEngine = Arc<dyn CvEngine>;

thread_local! {
    /// `cv::letterbox` 的当前调用作用域。实例之间不会共享宿主回调表。
    static ENGINE_STACK: RefCell<Vec<SharedCvEngine>> = RefCell::new(Vec::new());
}

static DEFAULT_ENGINE: OnceLock<SharedCvEngine> = OnceLock::new();

/// 进程内活跃算法实例计数。
///
/// 每个算法包是独立 `cdylib`，静态链接本 SDK，因此该计数天然按库隔离，
/// 不会跨算法包互相干扰。
static ACTIVE_INSTANCES: AtomicUsize = AtomicUsize::new(0);

/// 默认引擎的实例租约：最后一个实例销毁时回收进程级硬件资源。
///
/// **为何需要显式回收**：[`default_engine`] 把引擎存放在 `static OnceLock` 中，
/// 而 Rust 静态变量**永不执行 `Drop`**；RGA 池持有的 DMA-BUF 导入句柄因此会一直
/// 挂在 `rga_mm` 上，直到进程退出才由内核强制回收，在内核日志留下
/// `rga_mm: [tgid:N] Destroy handle[M] when the user exits` 噪声。
///
/// 本租约在算法实例创建时登记、销毁时注销，归零即回收硬件资源，
/// 使动态卸载算法包或服务停机不再依赖内核兜底。
///
/// 引擎本身保留在 [`DEFAULT_ENGINE`] 中，后续实例会按需重建缓冲池。
pub struct DefaultEngineLease {
    _not_constructible: (),
}

impl DefaultEngineLease {
    /// 登记一个持有默认引擎的算法实例。
    pub fn acquire() -> Self {
        ACTIVE_INSTANCES.fetch_add(1, Ordering::AcqRel);
        Self {
            _not_constructible: (),
        }
    }
}

impl Drop for DefaultEngineLease {
    fn drop(&mut self) {
        // 饱和递减：计数异常（如未配对 acquire）时不得回绕成天文数字而永不归零。
        let previous =
            ACTIVE_INSTANCES.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count.checked_sub(1)
            });
        if previous == Ok(1) {
            release_default_engine();
        }
    }
}

impl std::fmt::Debug for DefaultEngineLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DefaultEngineLease")
            .field("activeInstances", &ACTIVE_INSTANCES.load(Ordering::Relaxed))
            .finish()
    }
}

/// 回收默认引擎持有的硬件资源（RGA 导入句柄与缓冲池）。
///
/// 幂等：引擎尚未初始化或已被回收时为空操作。
/// 通常无需直接调用——实例销毁会经 [`DefaultEngineLease`] 自动触发。
pub fn release_default_engine() {
    if let Some(engine) = DEFAULT_ENGINE.get() {
        engine.release_hardware();
    }
}

fn default_engine() -> SharedCvEngine {
    DEFAULT_ENGINE
        .get_or_init(|| {
            #[cfg(all(target_os = "linux", feature = "rga"))]
            {
                Arc::new(platforms::rockchip::RgaCvEngine::new())
            }
            #[cfg(target_os = "macos")]
            {
                Arc::new(platforms::apple::AppleCvEngine::new())
            }
            #[cfg(not(any(target_os = "macos", all(target_os = "linux", feature = "rga"))))]
            {
                Arc::new(platforms::cpu::CpuCvEngine::new())
            }
        })
        .clone()
}

/// 从宿主注入的操作表构造一个实例级预处理引擎。
///
/// # Safety
/// `ops` 为空时表示使用平台默认引擎；非空时必须指向至少包含完整
/// `AvImageOps` 当前版本内容的、正确对齐且在实例生命周期内有效的表，
/// 且其中的 `ctx` 与回调必须保持有效。
pub unsafe fn engine_for_image_ops(ops: *const AvImageOps) -> Result<SharedCvEngine, AlgoError> {
    if ops.is_null() {
        return Ok(default_engine());
    }

    // SAFETY: 调用方保证 ops 指向宿主提供的有效 ABI 表；from_raw 会再次校验头部。
    let host_engine =
        unsafe { platforms::HostCvEngine::from_raw(ops)? }.ok_or_else(|| AlgoError::Internal {
            reason: "宿主图像操作表为空".to_string(),
        })?;
    Ok(Arc::new(host_engine))
}

/// 在一个插件实例的预处理引擎作用域内执行闭包。
///
/// Guard 通过 Drop 恢复线程局部栈，因此算法 panic 被外层 `catch_unwind` 捕获时
/// 也不会污染后续实例调用。
pub fn with_engine<R>(engine: SharedCvEngine, f: impl FnOnce() -> R) -> R {
    ENGINE_STACK.with(|stack| stack.borrow_mut().push(engine));

    struct EngineScope;
    impl Drop for EngineScope {
        fn drop(&mut self) {
            ENGINE_STACK.with(|stack| {
                let _ = stack.borrow_mut().pop();
            });
        }
    }

    let _scope = EngineScope;
    f()
}

/// 获取当前作用域的视觉引擎；不在插件回调内时使用平台默认引擎。
pub fn active_engine() -> SharedCvEngine {
    ENGINE_STACK
        .with(|stack| stack.borrow().last().cloned())
        .unwrap_or_else(default_engine)
}

/// 统一门面：低频 ROI 裁剪为 RGB24。
///
/// Rockchip 路径优先在 RGA 中裁剪并返回 DMA-BUF；其他平台使用对应引擎的保底实现。
pub fn crop_rgb(frame: &SafeFrame<'_>, rect: CropRect) -> Result<CvBuffer, AlgoError> {
    active_engine().crop_rgb(frame, rect)
}

/// 统一门面：Letterbox 预处理（自动保持原图比例居中缩放并填充底色）。
pub fn letterbox(
    frame: &SafeFrame<'_>,
    dst_w: u32,
    dst_h: u32,
    fill_color: [u8; 3],
) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
    active_engine().letterbox(frame, dst_w, dst_h, fill_color)
}

/// 统一门面：Resize 缩放预处理。
pub fn resize(
    frame: &SafeFrame<'_>,
    dst_w: u32,
    dst_h: u32,
) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
    active_engine().resize(frame, dst_w, dst_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 串行化共享 `ACTIVE_INSTANCES` 静态计数的用例（cargo test 默认多线程执行）。
    static LEASE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 租约计数必须在最后一个实例销毁时归零并触发引擎回收。
    ///
    /// 这锁住的是「进程级 `static` 引擎不能被 `Drop` 回收」这一 Rust 语言约束下的
    /// 显式回收契约：若退化为依赖静态析构，内核会持续打印
    /// `rga_mm: [tgid:N] Destroy handle[M] when the user exits`。
    ///
    /// 测试只验证计数语义，不依赖 librga：未初始化引擎时 `release_default_engine` 为空操作。
    #[test]
    fn lease_releases_engine_when_last_instance_drops() {
        let _guard = LEASE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let first = DefaultEngineLease::acquire();
        let second = DefaultEngineLease::acquire();
        assert_eq!(ACTIVE_INSTANCES.load(Ordering::Acquire), 2);

        drop(first);
        assert_eq!(
            ACTIVE_INSTANCES.load(Ordering::Acquire),
            1,
            "还有实例存活时不得回收引擎"
        );

        // 最后一个租约销毁：计数归零并触发回收路径。
        drop(second);
        assert_eq!(ACTIVE_INSTANCES.load(Ordering::Acquire), 0);
    }

    /// 计数为零时的递减必须饱和而非回绕（回绕会使引擎永不回收）。
    #[test]
    fn lease_decrement_saturates_at_zero() {
        let _guard = LEASE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(ACTIVE_INSTANCES.load(Ordering::Acquire), 0);

        let previous =
            ACTIVE_INSTANCES.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count.checked_sub(1)
            });
        assert_eq!(previous, Err(0), "计数为零时必须拒绝递减而非回绕");
        assert_eq!(ACTIVE_INSTANCES.load(Ordering::Acquire), 0);
    }
}
