use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use types::{FrameHandle, FrameRef, MotionGateConfig};

/// 帧差法运动门控计算状态
#[derive(Debug)]
pub struct MotionGate {
    pub config: MotionGateConfig,
    last_eval_time_ms: i64,
    previous_signature: Option<u64>,
}

impl MotionGate {
    pub fn new(config: MotionGateConfig) -> Self {
        Self {
            config,
            last_eval_time_ms: 0,
            previous_signature: None,
        }
    }

    /// 评估指定视频帧是否应跳过推理。
    ///
    /// 对于 Host 内存帧，采样有界像素签名计算帧间差异；
    /// 对于设备物理内存帧 (DMA-BUF 等)，避免 CPU 读回破坏 infer_fast_path 零拷贝边界，保守判定为有运动并放行。
    pub fn should_skip_frame(&mut self, frame: &FrameRef) -> bool {
        if !self.config.enabled {
            return false;
        }

        let signature = Self::calculate_frame_signature(frame);
        let has_motion = match (self.previous_signature, signature) {
            (Some(prev), Some(curr)) => prev != curr,
            _ => true,
        };
        if let Some(sig) = signature {
            self.previous_signature = Some(sig);
        }

        self.should_skip(frame.timestamp, has_motion)
    }

    /// 为 Host/Mock 帧计算有界签名。设备帧不读回像素，返回 None 以保守放行推理。
    fn calculate_frame_signature(frame: &FrameRef) -> Option<u64> {
        let FrameHandle::Host(bytes) = frame.handle() else {
            return None;
        };

        let mut hasher = DefaultHasher::new();
        frame.width.hash(&mut hasher);
        frame.height.hash(&mut hasher);
        frame.stride.hor_stride.hash(&mut hasher);
        frame.stride.ver_stride.hash(&mut hasher);
        let step = (bytes.len() / 4096).max(1);
        for (index, byte) in bytes.iter().step_by(step).take(4096).enumerate() {
            index.hash(&mut hasher);
            byte.hash(&mut hasher);
        }
        Some(hasher.finish())
    }

    /// 评估当前帧是否满足跳过推理（当画面无运动且未达到保活间隔时返回 true）
    pub fn should_skip(&mut self, current_time_ms: i64, has_motion: bool) -> bool {
        if !self.config.enabled {
            return false;
        }

        // 到达保活间隔周期，强制触发一次全帧推理
        if current_time_ms - self.last_eval_time_ms >= self.config.keepalive_interval_ms as i64 {
            self.last_eval_time_ms = current_time_ms;
            return false;
        }

        // 若检测到运动，更新评估时间并不跳过
        if has_motion {
            self.last_eval_time_ms = current_time_ms;
            false
        } else {
            true
        }
    }
}
