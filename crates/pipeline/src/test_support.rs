//! 单元测试共享替身与探针。
//!
//! 仅 crate 内测试可见。同一份帧构造 / 目录计数若在每个 `mod tests` 里各抄一遍，
//! 一旦行为漂移，很难判断是真实实现退化还是替身不一致。

use types::{FrameHandle, FrameRef, PixelFormat, StrideInfo};

/// 构造用于候选/证据编码的 Host 内存 NV12 帧 (320x240)。
pub(crate) fn test_nv12_frame(camera_id: &str, pts_ms: i64) -> FrameRef {
    let (width, height) = (320u32, 240u32);
    let nv12_size = (width * height * 3 / 2) as usize;
    FrameRef::new(
        camera_id.to_string(),
        pts_ms,
        width,
        height,
        StrideInfo::new(width, height),
        PixelFormat::Nv12,
        FrameHandle::Host(vec![160u8; nv12_size].into()),
    )
}

/// 统计目录下条目数（含子目录）；目录不存在或不可读时返回 0。
pub(crate) fn dir_entry_count(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| entries.flatten().count())
        .unwrap_or(0)
}
