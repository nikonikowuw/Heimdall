use thiserror::Error;

/// 核心领域类型基础错误枚举
#[derive(Debug, PartialEq, Error)]
pub enum TypeError {
    #[error("几何多边形顶点数量不足: 至少需要 3 个顶点，当前有 {0} 个")]
    InvalidPolygonPoints(usize),

    #[error("折线顶点数量不足: 至少需要 2 个顶点，当前有 {0} 个")]
    InvalidLinePoints(usize),

    #[error("坐标超出归一化 [0.0, 1.0] 区间: ({x}, {y})")]
    PointOutOfBounds { x: f64, y: f64 },

    #[error("帧载体错误: {0}")]
    Frame(#[from] FrameError),

    #[error("算法实例分析帧率必须在 0..=60 范围内，当前值: {value}")]
    InvalidAnalysisFps { value: i32 },

    #[error("算法参数必须为 JSON object，当前类型为: {actual_type}")]
    InvalidAlgoParams { actual_type: String },

    #[error("任务算法实例集合中存在重复算法 ID: {algorithm_id}")]
    DuplicateAlgorithmId { algorithm_id: String },

    #[error("状态码不在已知范围内: {0}")]
    UnknownTaskStatusCode(i32),

    #[error("算法 ID 不能为空")]
    EmptyAlgorithmId,
}

/// 媒体帧与缓冲区抽象相关错误
#[derive(Debug, PartialEq, Error)]
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
