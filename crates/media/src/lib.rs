pub mod buffer_pool;
pub mod decoder;
pub mod decoders;
pub mod dispatcher;
pub mod dmabuf_sync;
pub mod encoders;
pub mod error;
pub mod flv;
pub mod gop_queue;
pub mod image_convert;
pub mod probe;
pub mod retina_ingest;
pub mod rga;
#[cfg(all(target_os = "linux", feature = "rga"))]
pub mod rga_crop;
pub mod ring_buffer;
pub mod rtsp;
pub mod sps;
pub mod stream_hub;
pub mod sub_stream;
pub mod webcodecs;

pub use buffer_pool::{BufferPoolStats, PoolDiagnostics, PoolError};
pub use decoder::{DecodeDeliveryPolicy, VideoDecoder};
#[cfg(target_os = "macos")]
pub use decoders::VideoToolboxDecoder;
pub use decoders::{create_decoder, MockDecoder};
pub use dispatcher::{
    ConsumerHealthSnapshot, ConsumerId, ConsumerKind, DispatcherError, DispatcherMetrics,
    DispatcherMetricsSnapshot, GopSnapshot, KeyframeCache, KeyframeCacheStore, MediaSubscription,
    PacketDispatcher, PreviewDistributionConfig, StreamHealthSnapshot, StreamItem,
};
pub use dmabuf_sync::{DmaBufSyncDirection, DmaBufSyncGuard};
pub use encoders::{
    compute_crop_roi, crop_rgb_with_padding, encode_jpeg_from_rgb, CpuSnapEncoder,
    DeviceSnapEncoder, SnapEncoder,
};
pub use error::MediaError;
pub use flv::{FlvMuxer, FlvStreamPipeline};
pub use gop_queue::{
    GopAwarePacketQueue, GopDropState, GopQueueConfig, GopQueueMetrics, PushAction,
};
pub use image_convert::{
    debug_cpu_fallback_nv12_to_rgb, fast_nv12_to_rgb_image, frame_to_rgb_image,
    snapshot_readback_to_rgb_image,
};
pub use probe::{StreamInfo, StreamProber};
pub use retina_ingest::{
    sanitize_rtsp_url_and_credentials, RetinaIngestor, DEFAULT_HANDSHAKE_TIMEOUT,
    DEFAULT_STREAM_INACTIVITY_TIMEOUT,
};
pub use rga::{RgaCore, RgaPolicyChecker, RgaScopedBuffer, RgaStaticBuffer};
pub use ring_buffer::{MainStreamRingBuffer, RingBufferConfig};
pub use rtsp::{mask_rtsp_url, RtspIngestor};
pub use sps::{
    is_keyframe_or_parameter_set, parse_h264_sps, parse_h265_sps, split_annex_b_nalus, SpsInfo,
};
pub use stream_hub::{AiTaskLease, CameraStreamSession, StreamHub, StreamSubscription};
pub use sub_stream::{deduce_primary_sub_stream, deduce_sub_stream, SubStreamCandidate};
pub use webcodecs::{
    pack_webcodecs_frame, pack_webcodecs_frame_with_flags, unpack_webcodecs_frame,
    WebCodecsFrameHeader, WEBCODECS_FLAG_DISCONTINUITY, WEBCODECS_FRAME_HEADER_LEN,
    WEBCODECS_PROTOCOL_VERSION,
};
