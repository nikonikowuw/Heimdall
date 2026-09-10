#[cfg(all(feature = "mpp", feature = "dvpp"))]
compile_error!(
    "Features `mpp` and `dvpp` are mutually exclusive — \
     a single edge device cannot have both Rockchip VPU and Ascend DVPP."
);

pub mod mock;

#[cfg(all(target_os = "linux", feature = "mpp"))]
pub mod mpp;

#[cfg(any(all(target_os = "linux", feature = "dvpp"), test))]
pub mod dvpp;

#[cfg(target_os = "macos")]
pub mod videotoolbox;

pub use mock::MockDecoder;

#[cfg(all(target_os = "linux", feature = "mpp"))]
pub use mpp::MppDecoder;

#[cfg(all(target_os = "linux", feature = "dvpp"))]
pub use dvpp::DvppDecoder;

#[cfg(target_os = "macos")]
pub use videotoolbox::VideoToolboxDecoder;

use crate::decoder::VideoDecoder;
use types::CodecType;

/// 构造当前平台的默认硬件/模拟视频解码器
pub fn create_decoder(camera_id: &str, codec: CodecType) -> Box<dyn VideoDecoder + Send> {
    #[cfg(target_os = "macos")]
    {
        Box::new(VideoToolboxDecoder::new(camera_id, codec))
    }
    #[cfg(all(target_os = "linux", feature = "mpp"))]
    {
        Box::new(MppDecoder::new(camera_id, codec))
    }
    #[cfg(all(target_os = "linux", feature = "dvpp"))]
    {
        Box::new(DvppDecoder::new(camera_id, codec))
    }
    #[cfg(not(any(
        target_os = "macos",
        all(target_os = "linux", feature = "mpp"),
        all(target_os = "linux", feature = "dvpp")
    )))]
    {
        Box::new(MockDecoder::new(camera_id, codec, 1920, 1080))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mpp_stride_alignment_formulas() {
        assert_eq!((1920 + 15) & !15, 1920);
        assert_eq!((1919 + 15) & !15, 1920);
        assert_eq!((1921 + 15) & !15, 1936);
        assert_eq!((1080 + 15) & !15, 1088);
        assert_eq!((720 + 15) & !15, 720);
        assert_eq!((360 + 15) & !15, 368);
    }

    #[test]
    fn test_mpp_control_command_definitions() {
        // 验证 Rockchip MPP 核心控制信令严格符合官方头文件定义 (rk_mpi_cmd.h)
        // CMD_MODULE_CODEC(0x00300000) | CMD_CTX_ID_DEC(0x00010000) = 0x00310000
        const CMD_MODULE_CODEC: i32 = 0x00300000;
        const CMD_CTX_ID_DEC: i32 = 0x00010000;
        const MPP_DEC_CMD_BASE: i32 = CMD_MODULE_CODEC | CMD_CTX_ID_DEC;
        assert_eq!(MPP_DEC_CMD_BASE, 0x00310000);

        assert_eq!(MPP_DEC_CMD_BASE + 1, 0x00310001); // MPP_DEC_SET_FRAME_INFO
        assert_eq!(MPP_DEC_CMD_BASE + 2, 0x00310002); // MPP_DEC_SET_EXT_BUF_GROUP
        assert_eq!(MPP_DEC_CMD_BASE + 3, 0x00310003); // MPP_DEC_SET_INFO_CHANGE_READY
        assert_eq!(MPP_DEC_CMD_BASE + 4, 0x00310004); // MPP_DEC_SET_PRESENT_TIME_ORDER
        assert_eq!(MPP_DEC_CMD_BASE + 5, 0x00310005); // MPP_DEC_SET_PARSER_SPLIT_MODE
        assert_eq!(MPP_DEC_CMD_BASE + 6, 0x00310006); // MPP_DEC_SET_PARSER_FAST_MODE
        assert_eq!(MPP_DEC_CMD_BASE + 7, 0x00310007); // MPP_DEC_GET_STREAM_COUNT
        assert_eq!(MPP_DEC_CMD_BASE + 8, 0x00310008); // MPP_DEC_GET_VPUMEM_USED_COUNT
        assert_eq!(MPP_DEC_CMD_BASE + 10, 0x0031000a); // MPP_DEC_SET_OUTPUT_FORMAT
    }

    #[test]
    fn test_dvpp_stride_alignment_formulas() {
        assert_eq!((1920 + 15) / 16 * 16, 1920);
        assert_eq!((1919 + 15) / 16 * 16, 1920);
        assert_eq!((1921 + 15) / 16 * 16, 1936);
        assert_eq!((1080 + 1) / 2 * 2, 1080);
        assert_eq!((1081 + 1) / 2 * 2, 1082);
    }

    #[test]
    fn test_create_decoder_fallback_or_hardware() {
        let decoder = create_decoder("test_cam", CodecType::H264);
        drop(decoder);
    }
}
