//! 通道级录像配置

use serde::{Deserialize, Serialize};

/// 录像模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RecordingMode {
    /// 关闭录像（默认）
    #[default]
    Disabled,
    /// 仅事件触发录像：前置缓冲 + 事件后录制
    EventOnly,
    // 未来扩展：Continuous（连续切片）/ Hybrid（混合）
}

/// 单通道录像配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingConfig {
    #[serde(default)]
    pub mode: RecordingMode,
    /// 事件前录制秒数（5~30）
    #[serde(default = "default_pre_seconds")]
    pub pre_capture_seconds: u32,
    /// 事件后录制秒数（5~30）
    #[serde(default = "default_post_seconds")]
    pub post_capture_seconds: u32,
    /// 单个录像文件最大时长（秒），超过则强制切片
    #[serde(default = "default_max_file_seconds")]
    pub max_file_seconds: u32,
    /// TTL 保留天数
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

fn default_pre_seconds() -> u32 {
    10
}
fn default_post_seconds() -> u32 {
    10
}
fn default_max_file_seconds() -> u32 {
    300 // 5 分钟硬上限，防止极端高频事件导致文件无限延长
}
fn default_retention_days() -> u32 {
    7
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            mode: RecordingMode::default(),
            pre_capture_seconds: default_pre_seconds(),
            post_capture_seconds: default_post_seconds(),
            max_file_seconds: default_max_file_seconds(),
            retention_days: default_retention_days(),
        }
    }
}

impl RecordingConfig {
    /// 是否启用录像
    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.mode != RecordingMode::Disabled
    }

    /// 规范化配置：钳位所有数值到合法范围
    pub fn normalized(mut self) -> Self {
        self.pre_capture_seconds = self.pre_capture_seconds.clamp(5, 30);
        self.post_capture_seconds = self.post_capture_seconds.clamp(5, 30);
        self.max_file_seconds = self.max_file_seconds.clamp(30, 3600);
        self.retention_days = self.retention_days.clamp(1, 365);
        self
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_default_disabled() {
        let cfg = RecordingConfig::default();
        assert!(!cfg.is_enabled());
        assert_eq!(cfg.pre_capture_seconds, 10);
        assert_eq!(cfg.post_capture_seconds, 10);
    }

    #[test]
    fn test_normalization_clamps() {
        let cfg = RecordingConfig {
            mode: RecordingMode::EventOnly,
            pre_capture_seconds: 100,
            post_capture_seconds: 1,
            max_file_seconds: 99999,
            retention_days: 0,
        }
        .normalized();

        assert_eq!(cfg.pre_capture_seconds, 30);
        assert_eq!(cfg.post_capture_seconds, 5);
        assert_eq!(cfg.max_file_seconds, 3600);
        assert_eq!(cfg.retention_days, 1);
    }

    #[test]
    fn test_serde_round_trip_compat() {
        // 旧配置缺少 recording 字段时应使用默认值（向后兼容）
        let json = r#"{"mode":"eventOnly"}"#;
        let cfg: RecordingConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.mode, RecordingMode::EventOnly);
        assert_eq!(cfg.pre_capture_seconds, 10); // 默认填充
        assert!(cfg.is_enabled());
    }
}
