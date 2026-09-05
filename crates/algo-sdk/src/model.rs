//! 模型权重共享与多硬件核心调度 (model)
//!
//! 支持物理显存只占 1 份 (`SharedWeights`)，并为各通道按 Round-Robin 自动轮询或显式绑定 NPU/ANE 计算核心。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::cv::buffer::CvBuffer;
use crate::error::AlgoError;

/// 硬件计算核心调度描述符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Core {
    /// 自动轮询 / 驱动默认调度
    #[default]
    Auto,
    /// 绑定特定物理核心序号 (如 RK3588 NPU 核心 0, 1, 2)
    Id(u8),
    /// 全核心协同并行执行
    All,
    /// 核心掩码 (位掩码)
    Mask(u32),
}

/// 共享模型权重 Trait（物理显存/内存全局通常只驻留 1 份）
pub trait ModelWeights: Send + Sync + 'static {
    type Session: InferenceSession;

    /// 在指定硬件核心上初始化或拉起一个专属推理会话
    fn session_on(&self, core: Core) -> Result<Self::Session, AlgoError>;
}

/// 独占推理会话 Trait（每条视频流通道独占，`&mut self` 强制不可跨线程并发）
pub trait InferenceSession: Send + 'static {
    type Output;

    /// 接收经过 HAL 预处理后的显存容器 `CvBuffer`，执行推理并产出模型原始输出
    fn infer(&mut self, input: &CvBuffer) -> Result<Self::Output, AlgoError>;
}

/// 共享模型权重容器，提供无锁 Round-Robin 多核心轮询分发机制
#[derive(Debug, Clone)]
pub struct SharedWeights<W: ModelWeights> {
    inner: Arc<W>,
    core_count: u8,
    rr_counter: Arc<AtomicUsize>,
}

impl<W: ModelWeights> SharedWeights<W> {
    /// 构造共享权重管理器
    /// `core_count`: 可用物理硬件核心数（至少为 1）
    pub fn new(weights: W, core_count: u8) -> Self {
        Self {
            inner: Arc::new(weights),
            core_count: core_count.max(1),
            rr_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// 从已有的 `Arc<W>` 构造
    pub fn from_arc(inner: Arc<W>, core_count: u8) -> Self {
        Self {
            inner,
            core_count: core_count.max(1),
            rr_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// 自动以 Round-Robin 方式在多核心间轮询分配推理会话
    pub fn session(&self) -> Result<W::Session, AlgoError> {
        let idx = self.rr_counter.fetch_add(1, Ordering::Relaxed);
        let core_id = (idx % self.core_count as usize) as u8;
        self.inner.session_on(Core::Id(core_id))
    }

    /// 显式指定硬件核心拉起推理会话
    pub fn session_on(&self, core: Core) -> Result<W::Session, AlgoError> {
        self.inner.session_on(core)
    }

    /// 获取底层权重的 Arc 引用
    pub fn inner(&self) -> &Arc<W> {
        &self.inner
    }

    /// 可用核心数
    pub fn core_count(&self) -> u8 {
        self.core_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cv::types::PixelFormat;

    struct DummyWeights;
    struct DummySession {
        assigned_core: Core,
    }

    impl ModelWeights for DummyWeights {
        type Session = DummySession;
        fn session_on(&self, core: Core) -> Result<Self::Session, AlgoError> {
            Ok(DummySession {
                assigned_core: core,
            })
        }
    }

    impl InferenceSession for DummySession {
        type Output = Vec<f32>;
        fn infer(&mut self, _input: &CvBuffer) -> Result<Self::Output, AlgoError> {
            Ok(vec![1.0, 2.0, 3.0])
        }
    }

    #[test]
    fn test_single_core_round_robin() {
        let shared = SharedWeights::new(DummyWeights, 1);
        for _ in 0..5 {
            let session = shared.session().expect("创建会话");
            assert_eq!(session.assigned_core, Core::Id(0));
        }
    }

    #[test]
    fn test_triple_core_round_robin() {
        let shared = SharedWeights::new(DummyWeights, 3);
        let expected_cores = [
            Core::Id(0),
            Core::Id(1),
            Core::Id(2),
            Core::Id(0),
            Core::Id(1),
            Core::Id(2),
        ];

        for expected in expected_cores {
            let session = shared.session().expect("创建会话");
            assert_eq!(session.assigned_core, expected);
        }
    }

    #[test]
    fn test_explicit_core_binding() {
        let shared = SharedWeights::new(DummyWeights, 3);
        let mut session = shared.session_on(Core::Id(2)).expect("显式绑定核心 2");
        assert_eq!(session.assigned_core, Core::Id(2));

        let buf = CvBuffer::from_host(vec![0; 10], 1, 1, PixelFormat::Rgb24);
        let out = session.infer(&buf).expect("推理成功");
        assert_eq!(out, vec![1.0, 2.0, 3.0]);
    }
}
