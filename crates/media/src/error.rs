use thiserror::Error;

/// 流媒体接入与解码模块错误枚举
#[derive(Debug, Error)]
pub enum MediaError {
    #[error("RTSP 连接失败: {url}, 原因: {reason}")]
    RtspConnect { url: String, reason: String },

    #[error("RTSP 协议错误: {0}")]
    Protocol(String),

    #[error("SPS 序列参数集解析失败: {0}")]
    SpsParse(String),

    #[error("硬件解码器初始化失败: {codec}")]
    DecoderInit { codec: String, reason: String },

    #[error("视频帧解码失败: {reason}")]
    Decode { reason: String },

    #[error("不支持的编解码格式: {0}")]
    UnsupportedCodec(String),

    #[error("媒体探测超时 (超过 {0:?})")]
    ProbeTimeout(std::time::Duration),

    #[error("媒体流静默超时 (超过 {0:?} 未收到数据包)")]
    InactivityTimeout(std::time::Duration),

    #[error("流会话未找到: {0}")]
    SessionNotFound(String),

    #[error("底层帧错误: {0}")]
    Frame(#[from] types::FrameError),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

impl MediaError {
    /// 对应的 API 规范标准 5 位业务错误码 (20000~29999 摄像头/流媒体模块)
    pub fn error_code(&self) -> u32 {
        match self {
            Self::RtspConnect { .. } => 20001,
            Self::Protocol(_) => 20002,
            Self::SpsParse(_) => 20003,
            Self::ProbeTimeout(_) => 20004,
            Self::DecoderInit { .. } => 20005,
            Self::Decode { .. } => 20006,
            Self::UnsupportedCodec(_) => 20007,
            Self::SessionNotFound(_) => 20008,
            Self::Frame(_) => 20009,
            Self::Io(_) => 20010,
            Self::InactivityTimeout(_) => 20011,
        }
    }
}
