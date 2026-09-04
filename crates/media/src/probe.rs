use std::time::Duration;

use crate::error::MediaError;

/// 视频流探活信息
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

/// 摄像头码流测活探针
#[derive(Debug, Default)]
pub struct StreamProber;

impl StreamProber {
    pub async fn probe(_rtsp_url: &str, _timeout: Duration) -> Result<StreamInfo, MediaError> {
        // 探活实现桩（骨架阶段）：首批真实代码落地后打通 retina/ffmpeg 连接握手
        Ok(StreamInfo {
            codec: "h264".to_string(),
            width: 1920,
            height: 1080,
            fps: 25.0,
        })
    }
}
