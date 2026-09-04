use thiserror::Error;

/// 流媒体接入与解码模块错误枚举
#[derive(Debug, Error)]
pub enum MediaError {
    #[error("RTSP 连接失败: {url}")]
    RtspConnect {
        url: String,
        #[source]
        source: std::io::Error,
    },

    #[error("硬件解码器初始化失败: {codec}")]
    DecoderInit { codec: String, reason: String },

    #[error("视频帧解码失败: {reason}")]
    Decode { reason: String },

    #[error("不支持的编解码格式: {0}")]
    UnsupportedCodec(String),

    #[error("媒体探测超时 (超过 {0:?})")]
    ProbeTimeout(std::time::Duration),

    #[error("底层帧错误: {0}")]
    Frame(#[from] types::FrameError),
}
