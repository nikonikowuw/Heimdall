//! 事件录像 Worker 端到端测试
//!
//! 覆盖：前置缓冲 flush、事件触发、延长合并、SourceReset 截断、音频过滤、
//! 无关键帧跳过、落盘 fMP4 结构可解析。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use media::{CameraStreamSession, ConsumerKind};
use pipeline::recording::{
    FinishedRecording, RecordingConfig, RecordingEventType, RecordingMode, RecordingTrigger,
    RecordingWorker,
};
use types::{CodecType, EncodedPacket, StreamTag, TransportPolicy};

/// 带真实 SPS/PPS 的 H.264 keyframe（Annex-B，1080P High Profile）
fn keyframe(pts_ms: i64) -> Arc<EncodedPacket> {
    let sps = [
        0x67, 0x64, 0x00, 0x29, 0xac, 0x72, 0x84, 0x40, 0x78, 0x02, 0x27, 0xe5, 0xc0, 0x44, 0x00,
        0x00, 0x03, 0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xf0, 0x3c, 0x60, 0xc6, 0x58,
    ];
    let pps = [0x68, 0xEE, 0x3C, 0x80];
    let mut payload = Vec::new();
    payload.extend_from_slice(b"\x00\x00\x00\x01");
    payload.extend_from_slice(&sps);
    payload.extend_from_slice(b"\x00\x00\x00\x01");
    payload.extend_from_slice(&pps);
    payload.extend_from_slice(b"\x00\x00\x00\x01");
    payload.extend_from_slice(&[0x65, 0x88, 0x84, 0x00, 0x33, 0xFF]);

    Arc::new(EncodedPacket {
        pts_ms,
        is_keyframe: true,
        codec: CodecType::H264,
        payload: Bytes::from(payload),
        stream_tag: StreamTag::Video,
    })
}

fn pframe(pts_ms: i64) -> Arc<EncodedPacket> {
    let mut payload = Vec::new();
    payload.extend_from_slice(b"\x00\x00\x00\x01");
    payload.extend_from_slice(&[0x41, 0x9A, 0x00, 0x33, 0xFF]);
    Arc::new(EncodedPacket {
        pts_ms,
        is_keyframe: false,
        codec: CodecType::H264,
        payload: Bytes::from(payload),
        stream_tag: StreamTag::Video,
    })
}

struct TestHarness {
    worker: RecordingWorker,
    session: Arc<media::CameraStreamSession>,
    trigger_tx: std::sync::mpsc::Sender<RecordingTrigger>,
    finished: Arc<Mutex<Vec<FinishedRecording>>>,
    temp_dir: std::path::PathBuf,
}

impl TestHarness {
    fn new(pre_seconds: u32, post_seconds: u32) -> Self {
        let temp_dir = std::env::temp_dir().join(format!(
            "heimdall_rec_test_{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");

        let session = CameraStreamSession::mock("cam_test", "rtsp://mock", TransportPolicy::Tcp);
        let media_sub = session
            .dispatcher
            .subscribe("recording", ConsumerKind::Recording)
            .expect("subscribe");
        let subscription =
            media::StreamSubscription::from_media_for_test(media_sub, Arc::clone(&session));

        let (trigger_tx, trigger_rx) = std::sync::mpsc::channel();
        let finished: Arc<Mutex<Vec<FinishedRecording>>> = Arc::new(Mutex::new(Vec::new()));
        let finished_clone = Arc::clone(&finished);

        let config = RecordingConfig {
            mode: RecordingMode::EventOnly,
            pre_capture_seconds: pre_seconds,
            post_capture_seconds: post_seconds,
            max_file_seconds: 60,
            retention_days: 7,
        };

        let worker = RecordingWorker::spawn(
            "cam_test",
            subscription,
            trigger_rx,
            config,
            temp_dir.clone(),
            Box::new(move |rec| {
                finished_clone.lock().expect("lock").push(rec);
            }),
        );

        Self {
            worker,
            session,
            trigger_tx,
            finished,
            temp_dir,
        }
    }

    fn publish(&self, pkt: Arc<EncodedPacket>) {
        self.session.dispatcher.publish(pkt);
    }

    fn trigger(&self, event_id: &str, event_time_ms: i64) {
        self.trigger_tx
            .send(RecordingTrigger {
                event_id: event_id.to_string(),
                event_type: RecordingEventType::Alarm,
                event_time_ms,
            })
            .expect("send trigger");
    }

    fn wait_finished(&self, count: usize, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if self.finished.lock().expect("lock").len() >= count {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn finished_snapshot(&self) -> Vec<FinishedRecording> {
        self.finished.lock().expect("lock").clone()
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}

#[test]
fn test_event_triggers_recording_with_precapture() {
    let h = TestHarness::new(5, 5);

    // 前置缓冲：先送两个 GOP
    h.publish(keyframe(1000));
    for i in 1..=10 {
        h.publish(pframe(1000 + i * 40));
    }
    h.publish(keyframe(2000));
    for i in 1..=10 {
        h.publish(pframe(2000 + i * 40));
    }
    std::thread::sleep(Duration::from_millis(120));

    // 触发事件
    h.trigger("evt_001", 2200);
    std::thread::sleep(Duration::from_millis(120));

    // post-capture 期间的实时数据
    for i in 0..10 {
        h.publish(pframe(2500 + i * 40));
    }
    std::thread::sleep(Duration::from_millis(120));

    // 强制闭合（避免等 5 秒 post 窗口）
    h.worker.request_stop();
    assert!(h.wait_finished(1, Duration::from_secs(3)), "应有录像完成");

    let snapshot = h.finished_snapshot();
    let rec = &snapshot[0];
    assert_eq!(rec.camera_id, "cam_test");
    assert_eq!(rec.status, "completed");
    assert!(rec.file_size > 0, "文件应有内容");
    assert_eq!(rec.events.len(), 1);
    assert_eq!(rec.events[0].event_id, "evt_001");
    assert_eq!(rec.events[0].event_type, RecordingEventType::Alarm);
    // 前置片段应从 GOP 关键帧开始
    assert_eq!(rec.start_time_ms, 1000, "应包含完整前置缓冲");

    // 文件结构验证
    let bytes = std::fs::read(&rec.file_path).expect("read recording file");
    assert_eq!(&bytes[4..8], b"ftyp", "fMP4 应以 ftyp box 开头");
    assert!(bytes.windows(4).any(|w| w == b"moov"), "应包含 moov");
    assert!(bytes.windows(4).any(|w| w == b"moof"), "应包含 moof fragment");
    assert!(bytes.windows(4).any(|w| w == b"mdat"), "应包含 mdat");
}

#[test]
fn test_event_merge_extends_and_links() {
    let h = TestHarness::new(5, 5);

    h.publish(keyframe(3000));
    for i in 1..=5 {
        h.publish(pframe(3000 + i * 40));
    }
    std::thread::sleep(Duration::from_millis(120));

    h.trigger("evt_a", 3200);
    std::thread::sleep(Duration::from_millis(120));

    // post 尚未到期时第二个事件 → 合并
    h.trigger("evt_b", 3300);
    std::thread::sleep(Duration::from_millis(120));

    h.worker.request_stop();
    assert!(h.wait_finished(1, Duration::from_secs(3)));

    let snapshot = h.finished_snapshot();
    let rec = &snapshot[0];
    assert_eq!(rec.events.len(), 2, "合并录像应关联两个事件");
    assert_eq!(rec.events[0].event_id, "evt_a");
    assert_eq!(rec.events[1].event_id, "evt_b");
    assert_eq!(h.worker.recordings_written(), 1, "合并后只应产生一个文件");
}

#[test]
fn test_no_trigger_no_recording() {
    let h = TestHarness::new(5, 5);

    h.publish(keyframe(1000));
    for i in 1..=10 {
        h.publish(pframe(1000 + i * 40));
    }
    std::thread::sleep(Duration::from_millis(150));

    h.worker.request_stop();
    std::thread::sleep(Duration::from_millis(300));

    assert!(
        h.finished_snapshot().is_empty(),
        "无事件触发不应产生录像文件"
    );
    assert_eq!(h.worker.recordings_written(), 0);
}

#[test]
fn test_source_reset_truncates_active_recording() {
    let h = TestHarness::new(5, 5);

    h.publish(keyframe(5000));
    h.publish(pframe(5040));
    std::thread::sleep(Duration::from_millis(120));
    h.trigger("evt_reset", 5100);
    std::thread::sleep(Duration::from_millis(120));

    h.session.dispatcher.source_reset();
    assert!(h.wait_finished(1, Duration::from_secs(3)));

    let snapshot = h.finished_snapshot();
    assert_eq!(snapshot[0].status, "truncated", "SourceReset 应截断录像");
    assert_eq!(h.worker.recordings_truncated(), 1);
}

#[test]
fn test_audio_packets_ignored() {
    let h = TestHarness::new(5, 5);

    let audio = Arc::new(EncodedPacket {
        pts_ms: 1000,
        is_keyframe: false,
        codec: CodecType::Aac,
        payload: Bytes::from_static(b"\xFF\xF1audio_payload"),
        stream_tag: StreamTag::Audio,
    });
    h.publish(audio);
    h.publish(keyframe(2000));
    std::thread::sleep(Duration::from_millis(120));

    h.trigger("evt_audio", 2100);
    std::thread::sleep(Duration::from_millis(120));
    h.worker.request_stop();

    assert!(h.wait_finished(1, Duration::from_secs(3)));
    let snapshot = h.finished_snapshot();
    let bytes = std::fs::read(&snapshot[0].file_path).expect("read");
    // 录像文件不应包含音频 payload
    assert!(
        !bytes.windows(6).any(|w| w == b"audio_"),
        "音频数据不得进入视频录像文件"
    );
}

#[test]
fn test_frames_before_keyframe_skipped() {
    let h = TestHarness::new(5, 5);

    // 只有 P 帧，无关键帧
    for i in 0..5 {
        h.publish(pframe(1000 + i * 40));
    }
    std::thread::sleep(Duration::from_millis(150));

    h.trigger("evt_nokf", 1200);
    std::thread::sleep(Duration::from_millis(200));

    h.worker.request_stop();
    std::thread::sleep(Duration::from_millis(300));

    assert!(
        h.finished_snapshot().is_empty(),
        "无关键帧时不应产生录像（无法独立解码）"
    );
}

#[test]
fn test_worker_lifecycle_and_stats() {
    let h = TestHarness::new(5, 5);
    assert!(h.worker.is_alive());
    assert_eq!(h.worker.camera_id(), "cam_test");
    assert_eq!(h.worker.recordings_written(), 0);
    assert_eq!(h.worker.recordings_truncated(), 0);
}

#[test]
fn test_second_recording_after_completion() {
    let h = TestHarness::new(5, 5);

    // 第一段录像
    h.publish(keyframe(1000));
    h.publish(pframe(1040));
    std::thread::sleep(Duration::from_millis(120));
    h.trigger("evt_1st", 1100);
    std::thread::sleep(Duration::from_millis(120));
    h.session.dispatcher.source_reset(); // 强制闭合第一段
    assert!(h.wait_finished(1, Duration::from_secs(3)));

    // 第二段录像（新会话）
    h.publish(keyframe(9000));
    h.publish(pframe(9040));
    std::thread::sleep(Duration::from_millis(120));
    h.trigger("evt_2nd", 9100);
    std::thread::sleep(Duration::from_millis(120));
    h.worker.request_stop();

    assert!(h.wait_finished(2, Duration::from_secs(3)), "应有第二段录像");
    let snapshot = h.finished_snapshot();
    assert_eq!(snapshot.len(), 2);
    assert_eq!(snapshot[0].events[0].event_id, "evt_1st");
    assert_eq!(snapshot[1].events[0].event_id, "evt_2nd");
    // 两个文件路径应不同
    assert_ne!(snapshot[0].file_path, snapshot[1].file_path);
}
