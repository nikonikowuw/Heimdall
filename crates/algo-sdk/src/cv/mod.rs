//! 视觉预处理硬件抽象层 (HAL & SPI)
//! 提供统一门面函数 `cv::letterbox` / `cv::resize`，底层自动分派到最佳硬件加速器。

use std::cell::RefCell;
use std::sync::{Arc, OnceLock};

pub mod buffer;
pub mod engine;
pub mod layout;
pub mod platforms;
pub mod postprocess;
pub mod types;

pub use buffer::{CvBuffer, DmaBufLayout};
pub use engine::CvEngine;
pub use layout::compute_letterbox_layout;
pub use types::{LetterboxLayout, PixelFormat, PreprocessMode};

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
