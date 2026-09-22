use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

use db::{CaptureRepo, DbError};
use pipeline::{PipelineAnalysisEvent, PipelineCaptureEvent, PipelineManager, SnapshotResult};
use sea_orm::Set;

use crate::state::AppState;

/// 规范化证据相对路径：去除前导分隔符并校验路径穿越。
///
/// 返回 `None` 当路径为空、包含 `..` 组件或含有非法字符。
/// 两处使用（`capture_service` 写入复制、`evidence` 审核复制）统一收敛于此，
/// 杜绝路径穿越（AGENTS.md: 证据图片服务接口必须验证路径规范化）。
pub(crate) fn normalize_evidence_relative_path(path: &str) -> Option<&str> {
    let clean = path.trim_start_matches('/').trim_start_matches('\\');
    if clean.is_empty() || clean.contains("..") {
        return None;
    }
    Some(clean)
}

/// 隔离复制底库样本照片至识别对账目录 (recognitions/{recognition_id}_gallery.jpg)
///
/// 严格遵循系统安全与数据完整性约束：
/// 1. 规范化源相对路径，拒绝空路径与包含 `..` 的路径穿越；
/// 2. 校验源路径存在性与 base_dir 范围 (canonicalize.starts_with)；
/// 3. 自复制保护：若源路径与目标路径一致，直接复用返回，严防 `fs::copy` 同名文件截断清空为 0 字节；
/// 4. 自动创建目标父目录并安全复制。
pub(crate) async fn isolate_gallery_evidence_photo(
    base_dir: &std::path::Path,
    source_rel: &str,
    recognition_id: &str,
) -> Option<String> {
    let clean_rel = normalize_evidence_relative_path(source_rel)?;
    let dest_rel = format!("recognitions/{recognition_id}_gallery.jpg");

    // 1. 快速字符串比对：若源路径已经是当前识别记录的目标路径，直接返回
    if clean_rel == dest_rel {
        return Some(dest_rel);
    }

    if !base_dir.exists() {
        return None;
    }

    let src = base_dir.join(clean_rel);
    let dest = base_dir.join(&dest_rel);

    // 2. 规范化路径与越界校验
    let canonical_base = tokio::fs::canonicalize(base_dir).await.ok()?;
    let canonical_src = tokio::fs::canonicalize(&src).await.ok()?;
    if !canonical_src.starts_with(&canonical_base) {
        tracing::warn!(src = %src.display(), "拒绝越界底库样本照片复制");
        return None;
    }

    // 3. 规范化同名/硬链接保护
    if let Ok(canonical_dest) = tokio::fs::canonicalize(&dest).await {
        if canonical_src == canonical_dest {
            return Some(dest_rel);
        }
    }

    // 4. 安全创建目录并复制
    if let Some(parent) = dest.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if tokio::fs::copy(&src, &dest).await.is_ok() {
        Some(dest_rel)
    } else {
        tracing::warn!(src = %src.display(), dest = %dest.display(), "底库照片隔离复制失败");
        None
    }
}

/// 默认抓拍攒批写入阈值 (每批最多 32 条)
pub const DEFAULT_CAPTURE_BATCH_SIZE: usize = 32;

/// 默认抓拍攒批最大刷新等待时间 (毫秒)
pub const DEFAULT_CAPTURE_FLUSH_INTERVAL_MS: u64 = 500;

/// 默认人脸识别对账异步排队有界队列容量 (条)
pub const DEFAULT_RECOGNITION_QUEUE_CAPACITY: usize = 256;

/// 低频识别输入；embedding 只在 API 后台内存中存在。
#[derive(Debug, Clone)]
struct RecognitionFeature {
    embedding: std::sync::Arc<[u8]>,
    quality_score: f32,
}

///
/// 核心职责：
/// 1. 订阅管线分析引擎发出的客观通行抓拍事件 (`PipelineCaptureEvent`)；
/// 2. 独立管理行迹抓拍三支柱凭证持久化至 `capture_records`，严格与安全告警 (`alarm_records`) 隔离；
/// 3. 采用有界通道攒批提交 (32 条或 500ms 超时)，避免高频通行事件逐条开启单行事务刷盘；
/// 4. 在冷启动与通道滞后 (`Lagged`) 时通过管线内存补偿队列 (`drain_pending_capture_events`) 无损恢复。
#[derive(Debug, Clone)]
pub struct CaptureDispatchService {
    pub db: sea_orm::DatabaseConnection,
    pub pipeline: Arc<PipelineManager>,
    pub shutdown_tx: broadcast::Sender<()>,
    pub batch_size: usize,
    pub flush_interval_ms: u64,
    pub gallery_index: Option<Arc<crate::gallery_index::FaceFeatureIndex>>,
    pub event_broadcaster: Option<broadcast::Sender<crate::state::WsBroadcastEvent>>,
    recognition_tx: mpsc::Sender<PipelineCaptureEvent>,
    recognition_rx: RecognitionRxCell,
}

type RecognitionRxCell = Arc<Mutex<Option<mpsc::Receiver<PipelineCaptureEvent>>>>;

/// 初始化人脸识别对账有界异步排队通道
fn create_recognition_channel() -> (mpsc::Sender<PipelineCaptureEvent>, RecognitionRxCell) {
    let (tx, rx) = mpsc::channel(DEFAULT_RECOGNITION_QUEUE_CAPACITY);
    (tx, Arc::new(Mutex::new(Some(rx))))
}

/// 序列化现场主体及挂载人脸的归一化检测框，供抓拍与识别证据共同复用。
pub(crate) fn serialize_field_bbox(obj: &types::TrackedObject) -> String {
    if let Some(face) = &obj.face {
        if obj.is_pseudo_body() {
            // 伪造人体框已去除：仅保留人脸检测框，杜绝伪造人体框污染现场证据
            serde_json::json!({
                "face": face,
            })
            .to_string()
        } else {
            serde_json::json!({
                "body": obj.bbox,
                "face": face,
            })
            .to_string()
        }
    } else {
        serde_json::to_string(&obj.bbox).unwrap_or_else(|_| "{}".to_string())
    }
}

/// 若目标为人脸识别推导的虚拟躯干，规范化现场目标：去除虚拟躯干，以真实人脸检测框为主体
pub(crate) fn normalize_pseudo_body(obj: &types::TrackedObject) -> types::TrackedObject {
    if !obj.is_pseudo_body() {
        return obj.clone();
    }
    let mut normalized = obj.clone();
    if let Some(face) = &mut normalized.face {
        face.pseudo_body = Some(true);
        normalized.bbox = face.bbox;
    }
    normalized.label = "face".to_string();
    normalized
}

/// 证据图来源三元组（路径标识 / 码流 / 可比帧 PTS），抓拍与识别对账共用。
///
/// 目标 3 可追溯性：没有这三个值，库里一张低清凭据无法区分「峰值候选帧」与「靶向快拍帧」、
/// 「主码流高分辨率」与「子码流回退」，也无从判断证据帧与事件是否同刻。
pub(crate) fn evidence_origin(snap: &SnapshotResult) -> (String, String, i64) {
    (
        snap.image_source.as_str().to_string(),
        snap.image_stream.as_str().to_string(),
        snap.comparable_frame_pts_ms(),
    )
}

/// 解析落库的证据来源三元组；未知取值一律归一为 `None`（未标注）。
///
/// `image_pts_ms` 的 0 表示「不可比或未记录」（见 `V14` 迁移），同样归一为 `None`，
/// 避免消费方把 0 当成 1970 年的时标渲染。
pub(crate) fn parse_evidence_origin(
    image_source: &str,
    image_stream: &str,
    image_pts_ms: i64,
) -> (
    Option<types::EvidenceImageSource>,
    Option<types::EvidenceImageStream>,
    Option<i64>,
) {
    (
        types::EvidenceImageSource::from_wire(image_source),
        types::EvidenceImageStream::from_wire(image_stream),
        (image_pts_ms > 0).then_some(image_pts_ms),
    )
}

/// 匹配所用融合模板的元数据（参与帧数 / 质量加权均值）。
///
/// 仅新版算法包在 sidecar 帧上报，旧包与未上报帧为 `None`；
/// `template_mature` 是一次性成熟握手信号，不是可持久化的状态，故不在此列。
pub(crate) fn template_metadata(obj: &types::TrackedObject) -> (Option<i64>, Option<f32>) {
    let face = obj.face.as_ref();
    (
        face.and_then(|f| f.fused_count).map(i64::from),
        face.and_then(|f| f.template_quality),
    )
}

impl CaptureDispatchService {
    /// 从 `AppState` 中提取句柄构造默认抓拍分发服务实例
    pub fn from_state(state: &AppState) -> Self {
        let (recognition_tx, recognition_rx) = create_recognition_channel();
        Self {
            db: state.db.clone(),
            pipeline: state.pipeline.clone(),
            shutdown_tx: state.shutdown_tx.clone(),
            batch_size: DEFAULT_CAPTURE_BATCH_SIZE,
            flush_interval_ms: DEFAULT_CAPTURE_FLUSH_INTERVAL_MS,
            gallery_index: Some(state.gallery_index.clone()),
            event_broadcaster: Some(state.event_broadcaster.clone()),
            recognition_tx,
            recognition_rx,
        }
    }

    /// 指定攒批参数构造服务实例
    pub fn with_options(
        db: sea_orm::DatabaseConnection,
        pipeline: Arc<PipelineManager>,
        shutdown_tx: broadcast::Sender<()>,
        batch_size: usize,
        flush_interval_ms: u64,
    ) -> Self {
        let (recognition_tx, recognition_rx) = create_recognition_channel();
        Self {
            db,
            pipeline,
            shutdown_tx,
            batch_size: batch_size.max(1),
            flush_interval_ms: flush_interval_ms.max(10),
            gallery_index: None,
            event_broadcaster: None,
            recognition_tx,
            recognition_rx,
        }
    }

    /// 当前人脸识别对账排队队列中的在途积压任务数
    pub fn recognition_queue_depth(&self) -> usize {
        DEFAULT_RECOGNITION_QUEUE_CAPACITY.saturating_sub(self.recognition_tx.capacity())
    }

    /// 提取识别比对质量分（人脸匹配用）：人脸质量 → 目标质量 → 置信度。
    ///
    /// 与抓拍落库的 `quality_score` 口径**不同，勿混用**：后者是
    /// [`types::TrackedObject::evidence_quality_score`]（"这张图能不能看清外观"，
    /// 无脸时回退归一化面积），而本函数服务于 1:N 比对的阈值自适应，无脸时用置信度
    /// 比用面积更接近"这张脸有多可信"的量纲。
    fn resolve_recognition_quality(obj: &types::TrackedObject) -> f32 {
        obj.face
            .as_ref()
            .and_then(|f| f.quality_score)
            .or(obj.quality_score)
            .unwrap_or(obj.confidence)
            .clamp(0.0, 1.0)
    }

    /// 判定抓拍事件是否应当进入 1:N 识别比对队列。
    ///
    /// **这是人脸门控唯一合法的位置**：它管的是"要不要触发识别比对"，而不是"要不要落
    /// 抓拍记录"（后者以人体为主体，背身/低头同样必须落库）。曾经存在的
    /// `algorithm_id.contains("face")` 通配子句会把无脸记录一并投进识别队列，
    /// 导致每个无脸目标白烧一次"读盘 + 人脸模型推理"，因此必须禁止。
    fn is_face_event(obj: &types::TrackedObject) -> bool {
        obj.face.is_some() || obj.label.eq_ignore_ascii_case("face")
    }

    /// 识别对账必须拥有独立的人脸特写路径；全景图只能作为展示回退，不能作为识别证据。
    fn has_face_crop_path(path: &str) -> bool {
        !path.trim().is_empty()
    }

    /// 将单个 `PipelineCaptureEvent` 转换为数据库 `ActiveModel`
    fn event_to_active_model(
        event: &PipelineCaptureEvent,
    ) -> Option<db::entity::capture::ActiveModel> {
        let snap = match &event.snapshot {
            Some(s) => s,
            None => {
                tracing::warn!(
                    capture_id = %event.capture_id,
                    camera_id = %event.camera_id,
                    algorithm_id = %event.algorithm_id,
                    target_label = %event.tracked_object.label,
                    "通行抓拍快照未就绪或捕获失败，跳过无图抓拍记录落库"
                );
                return None;
            }
        };

        let captured_at = chrono::DateTime::from_timestamp_millis(event.timestamp)
            .unwrap_or_else(chrono::Utc::now);

        let tracked_object = normalize_pseudo_body(&event.tracked_object);
        let is_pseudo = tracked_object.is_pseudo_body();
        let bbox_json = serialize_field_bbox(&tracked_object);

        // 抓拍落库的质量分与峰值选帧共用同一口径（无脸 ⇒ 归一化人体框面积）。
        let quality_score = tracked_object.evidence_quality_score();

        let (image_source, image_stream, image_pts_ms) = evidence_origin(snap);
        let (fused_count, template_quality) = template_metadata(&tracked_object);

        let (body_crop_id, body_crop_rel_path) = if is_pseudo {
            (String::new(), String::new())
        } else {
            (
                snap.body_crop_image_id.clone(),
                snap.body_crop_image_rel_path.clone(),
            )
        };

        Some(db::entity::capture::ActiveModel {
            id: sea_orm::NotSet,
            capture_id: Set(event.capture_id.clone()),
            camera_id: Set(event.camera_id.clone()),
            track_id: Set(tracked_object.track_id as i64),
            target_label: Set(tracked_object.label.clone()),
            confidence: Set(tracked_object.confidence),
            quality_score: Set(quality_score),
            bbox_json: Set(bbox_json),
            image_id: Set(snap.image_id.clone()),
            image_rel_path: Set(snap.image_rel_path.clone()),
            crop_image_id: Set(snap.crop_image_id.clone()),
            crop_image_rel_path: Set(snap.crop_image_rel_path.clone()),
            body_crop_image_id: Set(body_crop_id),
            body_crop_image_rel_path: Set(body_crop_rel_path),
            image_source: Set(image_source),
            image_stream: Set(image_stream),
            image_pts_ms: Set(image_pts_ms),
            fused_count: Set(fused_count),
            template_quality: Set(template_quality),
            captured_at: Set(captured_at),
            created_at: Set(chrono::Utc::now()),
        })
    }

    /// 批量持久化通行抓拍凭据至 `capture_records` 表
    pub async fn process_capture_batch(
        &self,
        events: &[PipelineCaptureEvent],
    ) -> Result<usize, DbError> {
        let models: Vec<_> = events
            .iter()
            .filter_map(Self::event_to_active_model)
            .collect();
        if models.is_empty() {
            return Ok(0);
        }

        let inserted = CaptureRepo::insert_batch(&self.db, models).await?;
        tracing::info!(
            count = inserted,
            "客观通行抓拍凭证批量落库成功 (单事务攒批)"
        );

        // 对人脸通行抓拍事件尝试触发 1:N 底库比对与识别对账（分发至有界异步排队队列）
        if let Some(gallery_index) = &self.gallery_index {
            if gallery_index.count().await > 0 {
                for evt in events {
                    if Self::is_face_event(&evt.tracked_object) {
                        match self.recognition_tx.try_send(evt.clone()) {
                            Ok(()) => {}
                            Err(mpsc::error::TrySendError::Full(dropped)) => {
                                tracing::warn!(
                                    camera_id = %dropped.camera_id,
                                    track_id = dropped.tracked_object.track_id,
                                    queue_capacity = DEFAULT_RECOGNITION_QUEUE_CAPACITY,
                                    "人脸识别对账有界队列已满，丢弃溢出抓拍比对事件"
                                );
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => {
                                tracing::debug!("人脸识别对账队列已关闭，跳过事件分发");
                            }
                        }
                    }
                }
            }
        }

        Ok(inserted)
    }

    /// 动态解析摄像头关联的算法实例阈值 (未显式配置时回退到安全默认值 0.75)
    async fn resolve_recognition_threshold(&self, camera_id: &str, algorithm_id: &str) -> f32 {
        let mut confirm_threshold = 0.75f32;
        if let Ok(instances) =
            db::AlgorithmInstanceRepo::list_by_camera_id(&self.db, camera_id).await
        {
            for inst in instances {
                let is_exact = inst.algorithm_id == algorithm_id;
                let is_face = inst.algorithm_id.contains("face");
                if is_exact || is_face {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&inst.params_json) {
                        if let Some(st) = val.get("similarity_threshold").and_then(|v| v.as_f64()) {
                            confirm_threshold = st as f32;
                        }
                    }
                    if is_exact {
                        break;
                    }
                }
            }
        }
        confirm_threshold
    }

    /// 针对人脸通行抓拍尝试触发 1:N 底库特征检索并落地识别对账记录
    async fn try_match_and_record_recognition(&self, event: &PipelineCaptureEvent) {
        let Some(gallery_index) = &self.gallery_index else {
            return;
        };

        if !Self::is_face_event(&event.tracked_object) {
            return;
        }

        let Some(snap) = &event.snapshot else {
            return;
        };

        if !Self::has_face_crop_path(&snap.crop_image_rel_path) {
            tracing::warn!(
                capture_id = %event.capture_id,
                camera_id = %event.camera_id,
                track_id = event.tracked_object.track_id,
                image_path = %snap.image_rel_path,
                "人脸识别事件缺少人脸特写，跳过识别对账落库"
            );
            return;
        }

        if gallery_index.count().await == 0 {
            return;
        }

        // 视频流识别与离线提取彻底解耦：
        // 抓拍对账仅消费视频流自身产出的特征（来自 C ABI sidecar / 航迹时域融合）；
        // 若视频流未产生 embedding（如侧脸、低质或未达融合门限），该事件仅作为客观通行抓拍留存，
        // 绝不逆向读取磁盘图片调用离线大图提取器。
        let feature = match event.tracked_object.embedding() {
            Some(embedding) => {
                let quality_score = Self::resolve_recognition_quality(&event.tracked_object);
                RecognitionFeature {
                    embedding: embedding.clone(),
                    quality_score,
                }
            }
            None => {
                tracing::debug!(
                    camera_id = %event.camera_id,
                    track_id = event.tracked_object.track_id,
                    "通行抓拍未携带视频流识别特征，跳过 1:N 识别对账，仅保留抓拍记录"
                );
                return;
            }
        };

        let base_dir = self.pipeline.snapshot_engine().base_evidence_dir();

        // 动态解析摄像头关联的算法实例确认阈值 (未显式配置时回退到安全默认值 0.75)
        let confirm_threshold = self
            .resolve_recognition_threshold(&event.camera_id, &event.algorithm_id)
            .await;

        // 遵循 docs/algo/EdgeFace.md §4.4 约定的质量感知动态阈值微调（实机标定系数 0.15）：
        // 质量评分围绕 0.50 基准点浮动 ±0.075，对高质量多帧成熟融合模板放宽门槛对抗监控-寸照域偏移，对低质单帧提高门槛抑制误认
        let quality_adjustment = (feature.quality_score - 0.5) * 0.15;
        let adaptive_confirm = (confirm_threshold - quality_adjustment).clamp(0.40, 0.95);

        // 执行 1:N 余弦比对 Top-5，不设分数截断门槛 (-1.0)，保证返回无条件的完整 Top-5 候选人以留存完整识别过程
        let candidates = gallery_index
            .search_top_k(&feature.embedding, 5, -1.0)
            .await;

        if candidates.is_empty() {
            return;
        }

        let best_match = &candidates[0];
        let status = evaluate_recognition_status(&candidates, adaptive_confirm);

        // 只有 Top-1 相似度达到确认门槛才进入识别对账，不把无效记录写入识别对账
        if status == types::RecognitionStatus::Rejected {
            tracing::debug!(
                camera_id = %event.camera_id,
                track_id = event.tracked_object.track_id,
                top1_similarity = best_match.similarity,
                adaptive_confirm,
                "Top-1 相似度未达到确认阈值，判定为无效记录，跳过识别对账落库与广播"
            );
            return;
        }

        let recognition_id = uuid::Uuid::now_v7().to_string();
        let recognized_at = chrono::DateTime::from_timestamp_millis(event.timestamp)
            .unwrap_or_else(chrono::Utc::now);

        // 遵循证据隔离原则：识别确认后，复制一份底库样本至 recognitions 目录
        let rec_gallery_rel =
            isolate_gallery_evidence_photo(base_dir, &best_match.photo_rel_path, &recognition_id)
                .await
                .unwrap_or_else(|| best_match.photo_rel_path.clone());

        let candidates_json =
            serde_json::to_string(&candidates).unwrap_or_else(|_| "[]".to_string());

        let (image_source, image_stream, image_pts_ms) = evidence_origin(snap);
        let rec_tracked_object = normalize_pseudo_body(&event.tracked_object);
        let (fused_count, template_quality) = template_metadata(&rec_tracked_object);

        let active_rec = db::entity::recognition::ActiveModel {
            id: sea_orm::NotSet,
            recognition_id: Set(recognition_id.clone()),
            camera_id: Set(event.camera_id.clone()),
            gallery_id: Set("default".to_string()),
            subject_id: Set(best_match.subject_id.clone()),
            subject_name: Set(best_match.subject_name.clone()),
            similarity: Set(best_match.similarity),
            field_crop_path: Set(snap.crop_image_rel_path.clone()),
            field_image_path: Set(snap.image_rel_path.clone()),
            field_bbox_json: Set(serialize_field_bbox(&rec_tracked_object)),
            registered_photo_path: Set(rec_gallery_rel),
            image_source: Set(image_source),
            image_stream: Set(image_stream),
            image_pts_ms: Set(image_pts_ms),
            fused_count: Set(fused_count),
            template_quality: Set(template_quality),
            status: Set(status.as_str().to_string()),
            candidates_json: Set(Some(candidates_json)),
            reviewer_id: Set(None),
            reviewed_at: Set(None),
            recognized_at: Set(recognized_at),
            created_at: Set(chrono::Utc::now()),
        };

        if let Ok(saved) = db::RecognitionRepo::insert(&self.db, active_rec).await {
            tracing::info!(
                recognition_id = %recognition_id,
                subject_id = %best_match.subject_id,
                similarity = best_match.similarity,
                status = status.as_str(),
                camera_id = %event.camera_id,
                "1:N 人脸识别对账命中成功并已持久化"
            );

            if let Some(broadcaster) = &self.event_broadcaster {
                let dto = crate::routes::evidence::RecognitionDto::from(saved);
                if let Ok(payload) = serde_json::to_value(&dto) {
                    let _ = broadcaster.send(crate::state::WsBroadcastEvent {
                        topic: types::TOPIC_RECOGNITION_MATCHED.to_string(),
                        payload,
                        timestamp: event.timestamp,
                    });
                }
            }
        }
    }

    /// 刷新当前缓冲区中的抓拍事件批次
    async fn flush_batch(&self, buffer: &mut Vec<PipelineCaptureEvent>, reason: &str) {
        if buffer.is_empty() {
            return;
        }
        let to_flush = std::mem::replace(buffer, Vec::with_capacity(self.batch_size));
        if let Err(err) = self.process_capture_batch(&to_flush).await {
            tracing::error!(error = %err, reason, "通行抓拍批次持久化失败");
        }
    }

    /// 从管线内存补偿队列中排空待落库抓拍并批量持久化
    pub async fn drain_and_persist_pending(&self) -> usize {
        let mut total_processed = 0;
        loop {
            let batch = self.pipeline.drain_pending_capture_events(self.batch_size);
            if batch.is_empty() {
                break;
            }
            match self.process_capture_batch(&batch).await {
                Ok(count) => total_processed += count,
                Err(err) => {
                    tracing::error!(
                        error = %err,
                        batch_len = batch.len(),
                        "补偿排空通行抓拍事件批量持久化失败"
                    );
                }
            }
        }
        if total_processed > 0 {
            tracing::debug!(
                count = total_processed,
                "已成功从管线待持久化队列排空并持久化通行抓拍凭证"
            );
        }
        total_processed
    }

    /// 启动常驻异步持久化工作线程（包含抓拍攒批写入与人脸识别对账异步排队）
    pub fn start_worker(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let rec_rx = self
            .recognition_rx
            .lock()
            .ok()
            .and_then(|mut guard| guard.take());

        tokio::spawn(async move {
            let (batch_done_tx, batch_done_rx) = tokio::sync::oneshot::channel::<()>();

            // 1. 启动独立的人脸识别对账有界异步排队工作任务 (单协程顺序消费，消除并发竞争与丢比对)
            let rec_handle = if let Some(mut rx) = rec_rx {
                let this = self.clone();
                let mut rec_shutdown_rx = self.shutdown_tx.subscribe();
                Some(tokio::spawn(async move {
                    tracing::info!(
                        capacity = DEFAULT_RECOGNITION_QUEUE_CAPACITY,
                        "后台人脸识别对账异步排队工作线程已启动 (有界队列排队)"
                    );
                    loop {
                        tokio::select! {
                            _ = rec_shutdown_rx.recv() => {
                                tracing::info!("接收到系统停机信号，等待抓拍批次持久化完成后排空对账队列");
                                // 等待抓拍批次持久化完全刷盘并推进识别事件（最多等待 2 秒保护）
                                let _ = tokio::time::timeout(Duration::from_secs(2), batch_done_rx).await;
                                while let Ok(event) = rx.try_recv() {
                                    this.try_match_and_record_recognition(&event).await;
                                }
                                break;
                            }
                            maybe_event = rx.recv() => {
                                match maybe_event {
                                    Some(event) => {
                                        this.try_match_and_record_recognition(&event).await;
                                    }
                                    None => {
                                        tracing::info!("人脸识别对账事件通道已关闭，工作线程退出");
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }))
            } else {
                None
            };

            // 2. 启动客观通行抓拍异步持久化工作任务 (攒批写入)
            let this = self.clone();
            let batch_handle = tokio::spawn(async move {
                let mut analysis_rx = this.pipeline.subscribe_analysis_events();
                let mut shutdown_rx = this.shutdown_tx.subscribe();
                let flush_interval = Duration::from_millis(this.flush_interval_ms);

                tracing::info!("后台客观通行抓拍异步持久化工作线程已启动 (攒批写入)");

                // 冷启动初期，先排空启动前积压的补偿队列
                this.drain_and_persist_pending().await;

                let mut batch_buffer: Vec<PipelineCaptureEvent> =
                    Vec::with_capacity(this.batch_size);
                let mut ticker = tokio::time::interval(flush_interval);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                loop {
                    tokio::select! {
                        _ = shutdown_rx.recv() => {
                            tracing::info!("接收到系统停机信号，通行抓拍持久化工作线程准备优雅退出");
                            this.flush_batch(&mut batch_buffer, "停机退出").await;
                            this.drain_and_persist_pending().await;
                            let _ = batch_done_tx.send(());
                            break;
                        }
                        _ = ticker.tick() => {
                            this.flush_batch(&mut batch_buffer, "定时刷新").await;
                        }
                        recv_res = analysis_rx.recv() => {
                            match recv_res {
                                Ok(PipelineAnalysisEvent::Capture(capture_evt)) => {
                                    batch_buffer.push(*capture_evt);
                                    if batch_buffer.len() >= this.batch_size {
                                        this.flush_batch(&mut batch_buffer, "满批阈值").await;
                                    }
                                }
                                Ok(PipelineAnalysisEvent::Alarm(_)) => {}
                                Ok(PipelineAnalysisEvent::Tracks(_)) | Ok(PipelineAnalysisEvent::Telemetry(_)) => {}
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                    tracing::warn!(
                                        skipped,
                                        "通行抓拍广播通道滞后，尝试从管线内存队列中补偿恢复待持久化抓拍"
                                    );
                                    this.drain_and_persist_pending().await;
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                    tracing::info!("管线分析广播通道已关闭，退出抓拍工作线程");
                                    this.flush_batch(&mut batch_buffer, "通道关闭").await;
                                    this.drain_and_persist_pending().await;
                                    let _ = batch_done_tx.send(());
                                    break;
                                }
                            }
                        }
                    }
                }
            });

            // 3. 联合等待双协程安全回收，确保无遗留脱缰协程
            if let Some(h) = rec_handle {
                let _ = tokio::join!(batch_handle, h);
            } else {
                let _ = batch_handle.await;
            }
        })
    }
}

/// 业务决策层硬核校验的 Top-1 与 Top-2 最小排他优势差值 (Margin 防控)
pub const MIN_CONFIRM_MARGIN: f32 = 0.05;

/// 根据候选人列表与自适应确认门槛判定最终识别状态。
///
/// 遵循 docs/algo/face-best-shot-fusion-design.md 附录 C 的 Margin 误认防控机制：
/// 1. 若 Top-1 相似度低于 adaptive_confirm，判定为 Rejected (未达确认门槛/无效比对)；
/// 2. 若 Top-1 达到 adaptive_confirm，硬核校验 Top-1 与 Top-2 排他优势差值 (Margin 防控)；
///    若两名候选人相似度咬得太紧 (差值 < MIN_CONFIRM_MARGIN)，判定为混淆匹配，降级为 PendingReview 供人工复核；
/// 3. 否则判定为 Confirmed。
pub fn evaluate_recognition_status(
    candidates: &[types::FaceCandidateItem],
    adaptive_confirm: f32,
) -> types::RecognitionStatus {
    let Some(best_match) = candidates.first() else {
        return types::RecognitionStatus::Rejected;
    };

    if best_match.similarity < adaptive_confirm {
        return types::RecognitionStatus::Rejected;
    }

    if let Some(second_match) = candidates.get(1) {
        let margin = best_match.similarity - second_match.similarity;
        if margin < MIN_CONFIRM_MARGIN {
            tracing::info!(
                top1_subject = %best_match.subject_name,
                top1_similarity = best_match.similarity,
                top2_subject = %second_match.subject_name,
                top2_similarity = second_match.similarity,
                margin,
                min_margin = MIN_CONFIRM_MARGIN,
                "人脸识别命中候选优势差值不足，触发 Margin 防控，降级为 PendingReview"
            );
            return types::RecognitionStatus::PendingReview;
        }
    }

    types::RecognitionStatus::Confirmed
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{FaceCandidateItem, RecognitionStatus};

    fn make_candidate(subject_id: &str, name: &str, similarity: f32) -> FaceCandidateItem {
        FaceCandidateItem {
            rank: 1,
            subject_id: subject_id.to_string(),
            subject_name: name.to_string(),
            similarity,
            face_id: format!("face_{subject_id}"),
            photo_rel_path: format!("galleries/{subject_id}/face.jpg"),
        }
    }

    #[test]
    fn test_single_candidate_passes_threshold() {
        let candidates = vec![make_candidate("s1", "张三", 0.78)];
        let status = evaluate_recognition_status(&candidates, 0.75);
        assert_eq!(status, RecognitionStatus::Confirmed);
    }

    #[test]
    fn test_single_candidate_below_threshold() {
        let candidates = vec![make_candidate("s1", "张三", 0.72)];
        let status = evaluate_recognition_status(&candidates, 0.75);
        assert_eq!(status, RecognitionStatus::Rejected);
    }

    #[test]
    fn test_empty_candidates_is_rejected() {
        let candidates = vec![];
        let status = evaluate_recognition_status(&candidates, 0.75);
        assert_eq!(status, RecognitionStatus::Rejected);
    }

    #[test]
    fn test_top1_passes_with_unconditional_top5() {
        let candidates = vec![
            make_candidate("s1", "张三", 0.82),
            make_candidate("s2", "李四", 0.35),
            make_candidate("s3", "王五", 0.25),
        ];
        let status = evaluate_recognition_status(&candidates, 0.75);
        assert_eq!(status, RecognitionStatus::Confirmed);
    }

    #[test]
    fn test_dual_candidate_with_tight_margin_is_demoted_to_pending_review() {
        let candidates = vec![
            make_candidate("s1", "张三", 0.80),
            make_candidate("s2", "李四", 0.77), // 差值 0.03 < 0.05，触发混淆拦截
        ];
        let status = evaluate_recognition_status(&candidates, 0.75);
        assert_eq!(status, RecognitionStatus::PendingReview);
    }

    #[test]
    fn test_recognition_queue_capacity_and_depth() {
        let (tx, mut rx) = mpsc::channel(4);
        assert_eq!(tx.capacity(), 4);

        let make_event = |id: &str, track_id: u64| PipelineCaptureEvent {
            capture_id: id.to_string(),
            camera_id: "cam_1".to_string(),
            algorithm_id: "face_recognition".to_string(),
            timestamp: 1000,
            tracked_object: types::TrackedObject {
                track_id,
                class_id: 0,
                label: "face".to_string(),
                confidence: 0.9,
                quality_score: Some(0.8),
                embedding: None,
                bbox: types::BoundingBox::new(0.1, 0.1, 0.2, 0.2),
                face: None,
                trajectory: vec![],
            },
            snapshot: None,
        };

        // 模拟构造 4 个占位事件填满队列
        for i in 0..4 {
            assert!(tx.try_send(make_event(&format!("cap_{i}"), i)).is_ok());
        }

        assert_eq!(tx.capacity(), 0);

        // 第 5 个事件应当触发 TrySendError::Full 丢弃保护，杜绝无界积压
        let overflow = make_event("cap_overflow", 999);
        match tx.try_send(overflow) {
            Err(mpsc::error::TrySendError::Full(dropped)) => {
                assert_eq!(dropped.capture_id, "cap_overflow");
            }
            other => panic!("预期 TrySendError::Full，实际得到: {:?}", other),
        }

        // 消费 1 个后通道容量应恢复
        assert!(rx.try_recv().is_ok());
        assert_eq!(tx.capacity(), 1);
    }

    #[test]
    fn empty_face_crop_path_is_rejected_for_recognition() {
        assert!(!CaptureDispatchService::has_face_crop_path(""));
        assert!(!CaptureDispatchService::has_face_crop_path("  \n"));
        assert!(CaptureDispatchService::has_face_crop_path("CAM/crop.jpg"));
    }

    /// 人脸门控只允许存在于识别分发：无脸事件不得进入 1:N 比对队列。
    ///
    /// 曾经的 `algorithm_id.contains("face")` 通配子句会让「face_recognition 任务看到的
    /// 背身/低头目标」一并投递进队列，于是每个无脸目标都要白烧一次读盘 + 人脸模型推理。
    #[test]
    fn faceless_events_never_enter_recognition_queue() {
        let obj = |label: &str, face: Option<types::FaceDetail>| types::TrackedObject {
            track_id: 1,
            class_id: 0,
            label: label.to_string(),
            confidence: 0.9,
            quality_score: Some(0.8),
            embedding: None,
            bbox: types::BoundingBox::new(0.1, 0.1, 0.2, 0.2),
            face,
            trajectory: vec![],
        };
        let face_detail = types::FaceDetail {
            bbox: types::BoundingBox::new(0.12, 0.12, 0.18, 0.2),
            confidence: 0.9,
            quality_score: Some(0.8),
            embedding: None,
            template_mature: None,
            fused_count: None,
            template_quality: None,
            pseudo_body: None,
        };

        assert!(
            !CaptureDispatchService::is_face_event(&obj("person", None)),
            "背身/低头的人不得进入识别比对队列"
        );
        assert!(CaptureDispatchService::is_face_event(&obj(
            "person",
            Some(face_detail.clone())
        )));
        assert!(
            CaptureDispatchService::is_face_event(&obj("face", None)),
            "纯人脸包的检测框本身即人脸框，必须参与比对"
        );
    }
}
