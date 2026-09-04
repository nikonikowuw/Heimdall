use types::MotionGateConfig;

/// 帧差法运动门控计算状态
#[derive(Debug)]
pub struct MotionGate {
    pub config: MotionGateConfig,
    last_eval_time_ms: i64,
}

impl MotionGate {
    pub fn new(config: MotionGateConfig) -> Self {
        Self {
            config,
            last_eval_time_ms: 0,
        }
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
