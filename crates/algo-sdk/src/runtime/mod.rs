//! 异构推理运行时统一抽象层 (NPU Runtime HAL)
//!
//! 提供跨芯片平台（Rockchip RKNN、未来华为昇腾 CANN/ACL、Apple CoreML 等）的统一推理会话抽象、
//! 零拷贝显存直通接口与动态平台路由。

pub mod fallback;
pub mod platforms;

use std::path::Path;

use crate::cv::buffer::CvBuffer;
use crate::cv::postprocess::RknnTensorOutput;
use crate::env::PackageEnv;
use crate::error::AlgoError;

pub use fallback::{
    classify_session_availability, hardware_unavailable_error, normalize_platform_id,
    platform_requires_hardware, resolve_fallback_policy, FallbackPolicy, HardwareAvailability,
    HardwareStatus, ALLOW_CPU_FALLBACK_ENV_KEY,
};

/// 异构推理引擎的原始输出统一视图
#[derive(Debug)]
pub enum InferenceOutput<'a> {
    /// 9 张量/多分支 INT8 结构化特征图输出（如 Rockchip RKNN 特征图）
    MultiBranch(Vec<RknnTensorOutput<'a>>),
    /// 单通道平铺浮点张量输出（如 Ascend / CoreML / 开发调试回退模拟路径）
    SingleFloat(&'a [f32]),
}

/// 异构推理会话统一 Trait
pub trait NpuSession: Send + 'static {
    /// 将经过 HAL 处理后的显存容器 `CvBuffer` 送入推理引擎，
    /// 并在输出张量借用生命周期内安全执行后处理闭包 `f`。
    fn infer_with<F, R>(&mut self, input: &CvBuffer, f: F) -> Result<R, AlgoError>
    where
        F: FnOnce(&InferenceOutput<'_>) -> Result<R, AlgoError>;
}

/// 通用开发调试回退推理会话（在无硬件 NPU 驱动时提供确定性模拟推理）
#[derive(Debug)]
pub struct CpuFallbackSession {
    outputs: Vec<f32>,
}

impl CpuFallbackSession {
    pub fn open(package_root: &Path, model_rel_path: &Path) -> Result<Self, AlgoError> {
        let full_path = package_root.join(model_rel_path);
        if !full_path.exists() {
            return Err(AlgoError::ModelLoad {
                reason: format!("模型文件不存在: {}", full_path.display()),
            });
        }
        // 模拟 YOLOv8 典型输出: 84 channels x 5040 anchors (640x384)
        // 预埋 2 个高置信度目标供生命周期与后处理测试验证
        let num_anchors = 5040;
        let channels = 84;
        let mut outputs = vec![0.0f32; channels * num_anchors];

        // 目标 1: anchor 100, class 0 (置信度 0.92, cx=320, cy=192, w=80, h=60)
        outputs[100] = 320.0;
        outputs[num_anchors + 100] = 192.0;
        outputs[2 * num_anchors + 100] = 80.0;
        outputs[3 * num_anchors + 100] = 60.0;
        outputs[4 * num_anchors + 100] = 0.92;

        // 目标 2: anchor 200, class 0 (置信度 0.88, cx=160, cy=96, w=50, h=40)
        outputs[200] = 160.0;
        outputs[num_anchors + 200] = 96.0;
        outputs[2 * num_anchors + 200] = 50.0;
        outputs[3 * num_anchors + 200] = 40.0;
        outputs[4 * num_anchors + 200] = 0.88;

        Ok(Self { outputs })
    }
}

impl NpuSession for CpuFallbackSession {
    fn infer_with<F, R>(&mut self, _input: &CvBuffer, f: F) -> Result<R, AlgoError>
    where
        F: FnOnce(&InferenceOutput<'_>) -> Result<R, AlgoError>,
    {
        let output = InferenceOutput::SingleFloat(&self.outputs);
        f(&output)
    }
}

/// 默认平台自动路由的推理会话
#[derive(Debug)]
pub enum RuntimeSession {
    #[cfg(feature = "rknn")]
    Rockchip(Box<platforms::rockchip::RknnSession>),
    Fallback(CpuFallbackSession),
}

impl RuntimeSession {
    /// 会话的硬件可用性状态
    ///
    /// `Fallback` 变体未使用任何硬件单元，一律为 [`HardwareStatus::Simulated`]。
    /// "需要硬件但不可用"由构造返回的 `Err` 表达，会在本枚举域之外。
    pub fn hardware_status(&self) -> HardwareStatus {
        match self {
            #[cfg(feature = "rknn")]
            Self::Rockchip(s) => s.hardware_status(),
            Self::Fallback(_) => HardwareStatus::Simulated,
        }
    }

    /// 是否处于开发调试 CPU 回退模拟模式
    pub fn is_fallback(&self) -> bool {
        self.hardware_status() == HardwareStatus::Simulated
    }

    /// 以默认策略（[`FallbackPolicy::Allow`]）打开会话
    ///
    /// 签名与行为与历史版本逐位一致，既有调用点不受影响。
    pub fn open(package_root: &Path, model_rel_path: &Path) -> Result<Self, AlgoError> {
        Self::open_with_policy(package_root, model_rel_path, FallbackPolicy::default())
    }

    /// 以显式回退策略打开会话（加法式新增入口）
    ///
    /// `policy` 为 [`FallbackPolicy::RequireHardware`] 且无可用运行时时返回
    /// `Err(AlgoError::ModelLoad { .. })`，绝不返回模拟会话。
    pub fn open_with_policy(
        package_root: &Path,
        model_rel_path: &Path,
        policy: FallbackPolicy,
    ) -> Result<Self, AlgoError> {
        #[cfg(feature = "rknn")]
        {
            // `RknnSessionOptions` 为 `#[non_exhaustive]`，但本 crate 内部可用结构体更新语法。
            let options = platforms::rockchip::RknnSessionOptions {
                fallback_policy: policy,
                ..Default::default()
            };
            let session = platforms::rockchip::RknnSession::open_or_fallback(
                package_root,
                model_rel_path,
                options,
            )?;
            Ok(Self::Rockchip(Box::new(session)))
        }

        #[cfg(not(feature = "rknn"))]
        {
            // 本 SDK 未编译任何硬件平台驱动，"真实硬件"在该构建下物理上不存在。
            if policy == FallbackPolicy::RequireHardware {
                return Err(hardware_unavailable_error(
                    package_root,
                    model_rel_path,
                    "当前构建未启用任何硬件推理后端（如 `rknn` feature）",
                ));
            }
            let s = CpuFallbackSession::open(package_root, model_rel_path)?;
            Ok(Self::Fallback(s))
        }
    }

    /// 解析本实例应使用的回退策略（自检模式 + 宿主平台标识 + 调用方显式声明 + 包私有 `.env`）
    ///
    /// 供声明式骨架与直接构造会话的算法包统一调用，避免各自重复实现优先级。
    /// `explicit` 为 `None` 时行为与历史一致（仅由自检与平台决定）。
    pub fn resolve_policy(
        is_self_test: bool,
        platform_id: &str,
        env: Option<&PackageEnv>,
        explicit: Option<FallbackPolicy>,
    ) -> FallbackPolicy {
        resolve_fallback_policy(is_self_test, platform_id, env, explicit)
    }
}

impl NpuSession for RuntimeSession {
    fn infer_with<F, R>(&mut self, input: &CvBuffer, f: F) -> Result<R, AlgoError>
    where
        F: FnOnce(&InferenceOutput<'_>) -> Result<R, AlgoError>,
    {
        match self {
            #[cfg(feature = "rknn")]
            Self::Rockchip(s) => s.infer_with(input, f),
            Self::Fallback(s) => s.infer_with(input, f),
        }
    }
}
