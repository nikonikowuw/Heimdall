//! Heimdall 独立 Rust 算法包开发套件 (`algo-sdk`)
//!
//! 提供纯 Rust 编写算法插件的轻量 SDK，零依赖主工程业务 crate，支持在独立仓库闭环开发与测试。
//!
//! # 快速开始
//!
//! 1. 实现 [`plugin::AlgoPlugin`] trait（`init` + `process`）
//! 2. 使用 [`cv::letterbox`] / [`cv::resize`] 做预处理
//! 3. 使用 [`cv::postprocess`] 的后处理工具解析模型输出
//! 4. 通过 [`emitter::ResultEmitter`] 发射检测结果
//! 5. 使用 [`export_algo!`] 宏导出 C ABI 虚表
//!
//! 详见 [` GUIDE.md`](https://github.com/) 开发指南。

pub mod c_abi;
pub mod cv;
pub mod emitter;
pub mod error;
pub mod frame;
pub mod macros;
pub mod math;
pub mod model;
pub mod plugin;
pub mod testing;

pub use emitter::ResultEmitter;
pub use error::AlgoError;
pub use frame::{FrameHandleView, SafeFrame};
pub use math::NormBox;
pub use model::{Core, InferenceSession, ModelWeights, SharedWeights};
pub use plugin::{AlgoPlugin, InitContext};
pub use testing::{MockEmitter, MockFrame, MockFrameBuilder, MockSession, MockWeights};

/// SDK 统一 Prelude，便于算法开发者一键导入核心类型
pub mod prelude {
    pub use crate::c_abi::*;
    pub use crate::cv::{self, CvBuffer, PreprocessMode};
    pub use crate::emitter::ResultEmitter;
    pub use crate::error::AlgoError;
    pub use crate::export_algo;
    pub use crate::frame::{FrameHandleView, SafeFrame};
    pub use crate::math::{self, NormBox};
    pub use crate::model::{Core, InferenceSession, ModelWeights, SharedWeights};
    pub use crate::plugin::{AlgoPlugin, InitContext};
    pub use crate::testing::{MockEmitter, MockFrame, MockFrameBuilder, MockSession, MockWeights};
}
