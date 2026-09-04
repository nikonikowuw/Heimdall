use thiserror::Error;

/// 核心领域类型基础错误枚举
#[derive(Debug, Error)]
pub enum TypeError {
    #[error("几何多边形顶点数量不足: 至少需要 3 个顶点，当前有 {0} 个")]
    InvalidPolygonPoints(usize),

    #[error("折线顶点数量不足: 至少需要 2 个顶点，当前有 {0} 个")]
    InvalidLinePoints(usize),

    #[error("坐标超出归一化 [0.0, 1.0] 区间: ({x}, {y})")]
    PointOutOfBounds { x: f64, y: f64 },

    #[error("帧载体错误: {0}")]
    Frame(#[from] FrameError),
}

/// 媒体帧与缓冲区抽象相关错误
#[derive(Debug, Error)]
pub enum FrameError {
    #[error("不支持的像素格式: {0:?}")]
    UnsupportedPixelFormat(String),

    #[error("步长（stride）未对齐: 宽度 {width}, 步长 {stride}, 要求对齐倍数 {alignment}")]
    StrideUnaligned {
        width: u32,
        stride: u32,
        alignment: u32,
    },

    #[error("原生硬件缓冲区句柄已失效或释放")]
    BufferHandleInvalid,

    #[error("缓冲区池（Buffer Pool）已满且无可借用空闲帧")]
    BufferPoolExhausted,
}
