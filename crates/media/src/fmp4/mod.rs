//! fMP4 (Fragmented MP4) 极薄写入器
//!
//! 专用于事件录像：将 H.264/H.265 `EncodedPacket` 序列封装为可浏览器直接播放的 fMP4 文件。
//!
//! - 仅支持**写入**，不支持读/编辑/解析
//! - V1 仅单 video track，封装器通过 `Track` trait 预留多轨扩展
//! - 每个 GOP 一个 fragment（`moof` + `mdat`），支持断电恢复

mod boxes;
mod nalu;
mod writer;

pub use writer::{FMP4Writer, FragmentInfo};
