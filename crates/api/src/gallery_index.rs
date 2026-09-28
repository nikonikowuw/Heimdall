use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};
use types::FaceCandidateItem;

use db::{DatabaseConnection, DbError, GalleryFaceRepo, PersonnelRepo};
use infer::c_abi::loader::RawAlgoGallery;
use types::FaceMatchResult;

static NEXT_FACE_NUMERIC_ID: AtomicU64 = AtomicU64::new(1);

/// 512 维 FP32 向量序列化为标准小端字节流 (2048 bytes)
pub fn embedding_to_le_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for val in embedding {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

/// 512 维 FP32 特征向量的标准字节长度（512 × 4）
pub const EMBEDDING_BYTES_LEN: usize = 2048;

/// 特征字节流是否具备可参与比对的维度
///
/// 这是「合法特征」的唯一定义：`reload` / `upsert_faces` 在入口用它把关，
/// 避免非 512 维（迁移数据、第三方写入、手工改库）的样本静默混入索引后
/// 让识别给出偏差分数，而冲撞校验却 fail closed。
#[inline]
pub fn is_valid_embedding_bytes(bytes: &[u8]) -> bool {
    bytes.len() == EMBEDDING_BYTES_LEN
}

/// 从小端字节流反序列化为 512 维 FP32 向量（向后兼容）
pub fn le_bytes_to_embedding(bytes: &[u8]) -> Option<[f32; 512]> {
    if !is_valid_embedding_bytes(bytes) {
        return None;
    }
    let mut vector = [0.0f32; 512];
    for (&chunk, out) in bytes.as_chunks::<4>().0.iter().zip(vector.iter_mut()) {
        *out = f32::from_le_bytes(chunk);
    }
    Some(vector)
}

/// 直接在小端 FP32 字节切片上计算归一化余弦，避免为每个样本分配 `Vec<f32>`。
fn normalized_cosine_from_le_bytes(query: &[f32], bytes: &[u8]) -> Option<f32> {
    if bytes.len() != query.len().checked_mul(4)? || query.is_empty() {
        return None;
    }

    let mut dot = 0.0f64;
    let mut query_norm = 0.0f64;
    let mut feature_norm = 0.0f64;
    for (&query_value, chunk) in query.iter().zip(bytes.as_chunks::<4>().0) {
        let feature_value = f32::from_le_bytes(*chunk);
        if !query_value.is_finite() || !feature_value.is_finite() {
            return None;
        }
        let query_value = f64::from(query_value);
        let feature_value = f64::from(feature_value);
        dot += query_value * feature_value;
        query_norm += query_value * query_value;
        feature_norm += feature_value * feature_value;
    }

    let denominator = (query_norm * feature_norm).sqrt();
    if !denominator.is_finite() || denominator <= f64::EPSILON {
        return None;
    }
    Some((dot / denominator).clamp(-1.0, 1.0) as f32)
}

pub use types::{bytes_to_floats, megvii_calibrate_cosine};

/// 内存常驻的人脸底库样本结构
#[derive(Debug, Clone)]
pub struct RegisteredFace {
    pub id: u64,
    pub subject_id: String,
    pub subject_name: String,
    pub face_id: String,
    pub photo_rel_path: String,
    pub feature_bytes: Vec<u8>,
}

impl RegisteredFace {
    pub fn new(
        subject_id: String,
        subject_name: String,
        face_id: String,
        photo_rel_path: String,
        feature_bytes: Vec<u8>,
    ) -> Self {
        Self::with_id(
            NEXT_FACE_NUMERIC_ID.fetch_add(1, AtomicOrdering::Relaxed),
            subject_id,
            subject_name,
            face_id,
            photo_rel_path,
            feature_bytes,
        )
    }

    pub fn with_id(
        id: u64,
        subject_id: String,
        subject_name: String,
        face_id: String,
        photo_rel_path: String,
        feature_bytes: Vec<u8>,
    ) -> Self {
        Self {
            id,
            subject_id,
            subject_name,
            face_id,
            photo_rel_path,
            feature_bytes,
        }
    }

    pub fn from_512(
        subject_id: String,
        subject_name: String,
        face_id: String,
        photo_rel_path: String,
        vector: [f32; 512],
    ) -> Self {
        Self::new(
            subject_id,
            subject_name,
            face_id,
            photo_rel_path,
            embedding_to_le_bytes(&vector),
        )
    }

    pub fn from_512_with_id(
        id: u64,
        subject_id: String,
        subject_name: String,
        face_id: String,
        photo_rel_path: String,
        vector: [f32; 512],
    ) -> Self {
        Self::with_id(
            id,
            subject_id,
            subject_name,
            face_id,
            photo_rel_path,
            embedding_to_le_bytes(&vector),
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CrossSubjectFaceMatch {
    pub subject_id: String,
    pub subject_name: String,
    pub face_id: String,
    pub raw_cosine: f32,
}

#[derive(Debug, thiserror::Error)]
pub enum GalleryIndexError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("算法包底库同步失败: {0}")]
    AlgorithmSync(String),
}

/// 人脸底库特征向量检索索引（支持 C ABI 共享底库委托与宿主无状态回退）
#[derive(Debug)]
pub struct FaceFeatureIndex {
    faces: Arc<RwLock<Arc<Vec<RegisteredFace>>>>,
    algo_gallery: Arc<RwLock<Option<Arc<RawAlgoGallery>>>>,
    enrollment_semaphore: Arc<Semaphore>,
    algo_sync_state: AtomicU64,
}

const ALGO_SYNC_DEGRADED_BIT: u64 = 1;
const ALGO_SYNC_VERSION_STEP: u64 = 2;

/// 空变更批次是否可以走「无需触碰包内索引」的捷径。
///
/// 只有在**本次没有任何实际增删**且**进入时包内索引未被标记可疑**时才成立。
/// 降级中即使零变更也必须走重建路径：`finish_gallery_sync` 的 CAS 会一并清除
/// 降级位，直接调它等于在没有重建的情况下宣布索引已可信。
#[inline]
fn can_skip_gallery_touch(removed: usize, inserted: usize, was_degraded: bool) -> bool {
    removed == 0 && inserted == 0 && !was_degraded
}

impl Default for FaceFeatureIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl FaceFeatureIndex {
    pub fn new() -> Self {
        Self {
            faces: Arc::new(RwLock::new(Arc::new(Vec::new()))),
            algo_gallery: Arc::new(RwLock::new(None)),
            enrollment_semaphore: Arc::new(Semaphore::new(1)),
            algo_sync_state: AtomicU64::new(0),
        }
    }

    fn begin_gallery_sync(&self) -> (u64, bool) {
        let previous = self
            .algo_sync_state
            .fetch_add(ALGO_SYNC_VERSION_STEP, AtomicOrdering::AcqRel);
        self.algo_sync_state
            .fetch_or(ALGO_SYNC_DEGRADED_BIT, AtomicOrdering::AcqRel);
        (
            previous.wrapping_add(ALGO_SYNC_VERSION_STEP) & !ALGO_SYNC_DEGRADED_BIT,
            previous & ALGO_SYNC_DEGRADED_BIT != 0,
        )
    }

    fn finish_gallery_sync(&self, generation: u64) -> bool {
        self.algo_sync_state
            .compare_exchange(
                generation | ALGO_SYNC_DEGRADED_BIT,
                generation,
                AtomicOrdering::AcqRel,
                AtomicOrdering::Acquire,
            )
            .is_ok()
    }

    fn gallery_sync_degraded(&self) -> bool {
        self.algo_sync_state.load(AtomicOrdering::Acquire) & ALGO_SYNC_DEGRADED_BIT != 0
    }

    async fn faces_snapshot(&self) -> Arc<Vec<RegisteredFace>> {
        let guard = self.faces.read().await;
        Arc::clone(&guard)
    }

    /// 串行化人员录入，避免两个并发请求都在对方入库前通过冲撞检查。
    pub async fn acquire_enrollment_permit(&self) -> Result<OwnedSemaphorePermit, String> {
        self.enrollment_semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| format!("获取人员录入许可失败: {err}"))
    }

    /// 绑定或解绑底层 C ABI 算法包共享底库句柄
    pub async fn bind_algo_gallery(&self, gallery: Option<Arc<RawAlgoGallery>>) {
        if gallery.is_some() {
            self.begin_gallery_sync();
        }
        let mut guard = self.algo_gallery.write().await;
        *guard = gallery;
    }

    /// 检查当前是否已绑定 C ABI 底库句柄
    pub async fn has_algo_gallery(&self) -> bool {
        self.algo_gallery.read().await.is_some()
    }

    /// 从数据库全量重新加载所有有效的人脸特征样本。
    ///
    /// 宿主快照先切换，随后在 blocking pool 中清空并批量重建 C ABI 底库；同步失败时
    /// 保持降级标记，让检索走宿主快照，并将错误返回给启动/维护调用方。
    pub async fn reload(&self, db: &DatabaseConnection) -> Result<usize, GalleryIndexError> {
        let (all_personnel, _) = PersonnelRepo::list_filtered(db, None, 100_000, 0).await?;
        let name_map: HashMap<String, String> = all_personnel
            .into_iter()
            .map(|person| (person.subject_id, person.name))
            .collect();

        let raw_faces = GalleryFaceRepo::list_all_valid_vectors(db).await?;
        let mut loaded = Vec::with_capacity(raw_faces.len());
        let mut rejected = Vec::new();
        for face in raw_faces {
            // 维度校验在入口把关：非法维度的样本无法参与任何比对，留在索引里只会
            // 让检索静默给不出分数、冲撞校验却 fail closed，形成难以归因的行为分叉。
            if !is_valid_embedding_bytes(&face.feature_vector) {
                rejected.push((face.face_id.clone(), face.feature_vector.len()));
                continue;
            }
            let subject_name = name_map
                .get(&face.subject_id)
                .cloned()
                .unwrap_or_else(|| face.subject_id.clone());
            loaded.push(RegisteredFace::with_id(
                face.id as u64,
                face.subject_id,
                subject_name,
                face.face_id,
                face.photo_rel_path,
                face.feature_vector,
            ));
        }
        loaded.sort_unstable_by_key(|face| face.id);

        let algo_gallery = self.algo_gallery.read().await.clone();
        if algo_gallery.is_some() {
            self.begin_gallery_sync();
        }
        let count = loaded.len();
        *self.faces.write().await = Arc::new(loaded);

        if !rejected.is_empty() {
            tracing::warn!(
                rejected = rejected.len(),
                samples = ?rejected.iter().take(5).collect::<Vec<_>>(),
                expected_bytes = EMBEDDING_BYTES_LEN,
                "底库存在维度非法的特征样本，已从内存索引中排除"
            );
        }

        if let Some(gallery) = algo_gallery.as_ref() {
            self.rebuild_algo_gallery(gallery)
                .await
                .map_err(GalleryIndexError::AlgorithmSync)?;
        }

        tracing::info!(
            count,
            rejected = rejected.len(),
            has_c_abi = algo_gallery.is_some(),
            "人脸底库特征内存索引全量加载/更新成功"
        );
        Ok(count)
    }

    /// 增量批量插入或更新人脸特征样本。
    ///
    /// 宿主快照不可变替换；C ABI 写失败时按最新宿主快照重建。同步状态未知期间，
    /// 识别和冲撞校验使用宿主路径，不会继续消费可能缺样本的算法包底库。
    pub async fn upsert_faces(&self, new_faces: Vec<RegisteredFace>) {
        if new_faces.is_empty() {
            return;
        }

        // 维度校验：非法维度的样本无法参与任何比对，写入索引只会让后续检索静默缺项，
        // 而冲撞校验 fail closed，形成两条路径的行为分叉。
        let new_faces: Vec<RegisteredFace> = new_faces
            .into_iter()
            .filter(|face| {
                let valid = is_valid_embedding_bytes(&face.feature_bytes);
                if !valid {
                    tracing::warn!(
                        face_id = %face.face_id,
                        bytes = face.feature_bytes.len(),
                        expected_bytes = EMBEDDING_BYTES_LEN,
                        "拒绝写入维度非法的底库特征样本"
                    );
                }
                valid
            })
            .collect();
        if new_faces.is_empty() {
            return;
        }

        let algo_gallery = self.algo_gallery.read().await.clone();
        let sync_context = algo_gallery.as_ref().map(|_| self.begin_gallery_sync());
        let mut inserted = Vec::with_capacity(new_faces.len());
        let mut replaced_ids = Vec::new();
        for face in &new_faces {
            inserted.push((face.id, face.feature_bytes.clone()));
        }

        {
            let mut guard = self.faces.write().await;
            let faces = Arc::make_mut(&mut guard);
            for face in new_faces {
                if let Some(position) = faces
                    .iter()
                    .position(|existing| existing.face_id == face.face_id)
                {
                    let old_id = faces[position].id;
                    if old_id != face.id {
                        replaced_ids.push(old_id);
                    }
                    faces[position] = face;
                } else {
                    faces.push(face);
                }
            }
            faces.sort_unstable_by_key(|face| face.id);
        }

        if let (Some(gallery), Some((generation, was_degraded))) = (algo_gallery, sync_context) {
            self.apply_gallery_delta(
                gallery,
                generation,
                was_degraded,
                replaced_ids,
                inserted,
                "人员人脸样本增量同步",
            )
            .await;
        }
    }

    /// 增量删除单张人脸特征样本。
    pub async fn remove_face(&self, face_id: &str) {
        let algo_gallery = self.algo_gallery.read().await.clone();
        let mut sync_context = None;
        let removed_id = {
            let mut guard = self.faces.write().await;
            let faces = Arc::make_mut(&mut guard);
            faces
                .iter()
                .position(|face| face.face_id == face_id)
                .map(|position| {
                    if algo_gallery.is_some() {
                        sync_context = Some(self.begin_gallery_sync());
                    }
                    faces.remove(position).id
                })
        };

        if let (Some(id), Some(gallery), Some((generation, was_degraded))) =
            (removed_id, algo_gallery, sync_context)
        {
            self.apply_gallery_delta(
                gallery,
                generation,
                was_degraded,
                vec![id],
                Vec::new(),
                "删除人员人脸样本",
            )
            .await;
        }
    }

    /// 增量删除指定人员的所有特征样本。
    pub async fn remove_subject(&self, subject_id: &str) {
        let algo_gallery = self.algo_gallery.read().await.clone();
        let mut sync_context = None;
        let removed_ids = {
            let mut guard = self.faces.write().await;
            let faces = Arc::make_mut(&mut guard);
            if algo_gallery.is_some() {
                sync_context = Some(self.begin_gallery_sync());
            }
            let mut ids = Vec::new();
            faces.retain(|face| {
                if face.subject_id == subject_id {
                    ids.push(face.id);
                    false
                } else {
                    true
                }
            });
            ids
        };

        if let (Some(gallery), Some((generation, was_degraded))) = (algo_gallery, sync_context) {
            if can_skip_gallery_touch(removed_ids.len(), 0, was_degraded) {
                self.finish_gallery_sync(generation);
            } else {
                self.apply_gallery_delta(
                    gallery,
                    generation,
                    was_degraded,
                    removed_ids,
                    Vec::new(),
                    "删除人员全部人脸样本",
                )
                .await;
            }
        }
    }

    async fn apply_gallery_delta(
        &self,
        gallery: Arc<RawAlgoGallery>,
        generation: u64,
        was_degraded: bool,
        removed_ids: Vec<u64>,
        inserted: Vec<(u64, Vec<u8>)>,
        operation: &'static str,
    ) {
        if was_degraded {
            if let Err(err) = self.rebuild_algo_gallery(&gallery).await {
                tracing::error!(operation, error = %err, "C ABI 底库已降级，按宿主快照重建失败");
            }
            return;
        }

        // 增量写入完成后原地校验包内数量是否与宿主快照一致。
        // 缺了这一步，单样本分歧会一直潜伏到某次无关录入的冲撞校验才被发现
        // （那是一次跨主体的 fail closed，归因噪音极大）。
        let expected_packaged_count = self.faces_snapshot().await.len();
        let worker_gallery = Arc::clone(&gallery);
        let delta_result = tokio::task::spawn_blocking(move || -> Result<(), String> {
            if !removed_ids.is_empty() {
                worker_gallery
                    .remove_batch(&removed_ids)
                    .map_err(|err| err.to_string())?;
            }
            if !inserted.is_empty() {
                worker_gallery
                    .insert_batch(&inserted)
                    .map_err(|err| err.to_string())?;
            }
            let packaged_count = worker_gallery.count().map_err(|err| err.to_string())? as usize;
            if packaged_count != expected_packaged_count {
                return Err(format!(
                    "底库数量不一致: algorithm={packaged_count}, host={expected_packaged_count}"
                ));
            }
            Ok(())
        })
        .await
        .map_err(|err| err.to_string())
        .and_then(|result| result);

        if let Err(err) = delta_result {
            tracing::error!(operation, error = %err, "C ABI 底库增量写入失败，开始全量自愈");
            if let Err(rebuild_err) = self.rebuild_algo_gallery(&gallery).await {
                tracing::error!(operation, error = %rebuild_err, "C ABI 底库自愈失败，检索保持宿主降级路径");
            }
            return;
        }

        if !self.finish_gallery_sync(generation) {
            if let Err(err) = self.rebuild_algo_gallery(&gallery).await {
                tracing::error!(operation, error = %err, "并发底库更新后全量自愈失败");
            }
        }
    }

    /// 稳定版本下按宿主快照重建包内底库；并发变更时重试最新版本。
    async fn rebuild_algo_gallery(&self, gallery: &Arc<RawAlgoGallery>) -> Result<(), String> {
        let mut last_error = "索引在重建过程中持续变化".to_string();
        for _ in 0..3 {
            let state = self.algo_sync_state.load(AtomicOrdering::Acquire);
            let generation = state & !ALGO_SYNC_DEGRADED_BIT;
            let faces = self.faces_snapshot().await;
            let worker_gallery = Arc::clone(gallery);
            let rebuild_result = tokio::task::spawn_blocking(move || {
                let samples: Vec<(u64, Vec<u8>)> = faces
                    .iter()
                    .map(|face| (face.id, face.feature_bytes.clone()))
                    .collect();
                worker_gallery.clear().map_err(|err| err.to_string())?;
                worker_gallery
                    .insert_batch(&samples)
                    .map_err(|err| err.to_string())?;
                let packaged_count =
                    worker_gallery.count().map_err(|err| err.to_string())? as usize;
                if packaged_count != samples.len() {
                    return Err(format!(
                        "底库数量不一致: algorithm={packaged_count}, host={}",
                        samples.len()
                    ));
                }
                Ok::<usize, String>(samples.len())
            })
            .await
            .map_err(|err| err.to_string())
            .and_then(|result| result);

            match rebuild_result {
                Ok(count) => {
                    let current = self.algo_sync_state.load(AtomicOrdering::Acquire);
                    if current & !ALGO_SYNC_DEGRADED_BIT != generation {
                        continue;
                    }
                    if self.finish_gallery_sync(generation) {
                        tracing::info!(count, "算法包 C ABI 底库已按宿主快照重建");
                        return Ok(());
                    }
                }
                Err(err) => {
                    last_error = err;
                    tracing::warn!(error = %last_error, "C ABI 底库重建尝试失败，将重试");
                }
            }
        }

        self.algo_sync_state
            .fetch_or(ALGO_SYNC_DEGRADED_BIT, AtomicOrdering::Release);
        Err(last_error)
    }

    async fn degrade_and_rebuild_algo_gallery(
        &self,
        gallery: &Arc<RawAlgoGallery>,
        operation: &'static str,
    ) {
        self.algo_sync_state
            .fetch_or(ALGO_SYNC_DEGRADED_BIT, AtomicOrdering::Release);
        if let Err(error) = self.rebuild_algo_gallery(gallery).await {
            tracing::error!(operation, error = %error, "C ABI 底库异常后的全量自愈失败");
        }
    }

    /// 仅替换既有条目的特征字节（按 `face_id` 定位，保留元数据与数字 ID）。
    ///
    /// 重提取场景的专用入口：`upsert_faces` 会整条替换条目，
    /// 而重提取只改变向量，人员身份、姓名、照片路径均应不变。
    /// 传入不存在的 `face_id` 会被忽略（该样本可能已被并发删除）。
    pub async fn replace_face_features(&self, updates: Vec<(String, Vec<u8>)>) {
        let updates: Vec<(String, Vec<u8>)> = updates
            .into_iter()
            .filter(|(face_id, feature_bytes)| {
                let valid = is_valid_embedding_bytes(feature_bytes);
                if !valid {
                    tracing::warn!(
                        face_id = %face_id,
                        bytes = feature_bytes.len(),
                        expected_bytes = EMBEDDING_BYTES_LEN,
                        "拒绝写入维度非法的重提取特征"
                    );
                }
                valid
            })
            .collect();
        if updates.is_empty() {
            return;
        }

        let mut updates_by_face_id = HashMap::with_capacity(updates.len());
        for (face_id, feature_bytes) in updates {
            updates_by_face_id.insert(face_id, feature_bytes);
        }

        let algo_gallery = self.algo_gallery.read().await.clone();
        let sync_context = algo_gallery.as_ref().map(|_| self.begin_gallery_sync());

        // 一次建立 face_id 查找表再扫描宿主快照，避免全量重提取时每条更新都线性搜索。
        let mut packaged_updates: Vec<(u64, Vec<u8>)> =
            Vec::with_capacity(updates_by_face_id.len());
        {
            let mut guard = self.faces.write().await;
            let faces = Arc::make_mut(&mut guard);
            for face in faces.iter_mut() {
                if let Some(feature_bytes) = updates_by_face_id.remove(&face.face_id) {
                    face.feature_bytes = feature_bytes.clone();
                    packaged_updates.push((face.id, feature_bytes));
                }
            }
        }
        for face_id in updates_by_face_id.keys() {
            tracing::warn!(face_id = %face_id, "重提取目标样本不在索引中，已跳过");
        }

        if let (Some(gallery), Some((generation, was_degraded))) = (algo_gallery, sync_context) {
            if can_skip_gallery_touch(0, packaged_updates.len(), was_degraded) {
                self.finish_gallery_sync(generation);
            } else {
                self.apply_gallery_delta(
                    gallery,
                    generation,
                    was_degraded,
                    Vec::new(),
                    packaged_updates,
                    "重提取人脸特征同步",
                )
                .await;
            }
        }
    }

    /// 增量更新指定人员姓名 (仅更新宿主元数据，C ABI 无需变更)
    pub async fn update_subject_name(&self, subject_id: &str, new_name: &str) {
        let mut guard = self.faces.write().await;
        for face in Arc::make_mut(&mut guard).iter_mut() {
            if face.subject_id == subject_id {
                face.subject_name = new_name.to_string();
            }
        }
    }

    /// 执行 1:N 检索 (优先委托给 C ABI 底库，无 C ABI 时回退至宿主 SIMD 与标定)
    pub async fn search_top_k(
        &self,
        query_bytes: &[u8],
        top_k: usize,
        min_threshold: f32,
    ) -> Vec<FaceCandidateItem> {
        if query_bytes.is_empty() || top_k == 0 {
            return Vec::new();
        }

        let algo_gallery = self.algo_gallery.read().await.clone();
        if let Some(gallery) = algo_gallery.filter(|_| !self.gallery_sync_degraded()) {
            let fetch_limit = top_k.saturating_mul(8).clamp(32, 256) as u32;
            let query = query_bytes.to_vec();
            let worker_gallery = Arc::clone(&gallery);
            let search_result = tokio::task::spawn_blocking(move || {
                worker_gallery.search(&query, fetch_limit, min_threshold)
            })
            .await
            .map_err(|err| err.to_string())
            .and_then(|result| result.map_err(|err| err.to_string()));

            match search_result {
                Ok(raw_cands) if !raw_cands.is_empty() => {
                    // 宿主快照按数字 ID 排序，最多 256 个候选各做一次二分查找。
                    let (resolved_count, resolved) = {
                        let faces = self.faces_snapshot().await;
                        let mut subject_best: HashMap<&str, (&RegisteredFace, f32)> =
                            HashMap::new();
                        let mut resolved_count = 0;
                        for candidate in &raw_cands {
                            let Ok(position) =
                                faces.binary_search_by_key(&candidate.id, |face| face.id)
                            else {
                                continue;
                            };
                            let face = &faces[position];
                            resolved_count += 1;
                            match subject_best.get_mut(face.subject_id.as_str()) {
                                Some(entry) if candidate.similarity > entry.1 => {
                                    *entry = (face, candidate.similarity);
                                }
                                Some(_) => {}
                                None => {
                                    subject_best.insert(
                                        face.subject_id.as_str(),
                                        (face, candidate.similarity),
                                    );
                                }
                            }
                        }

                        let mut sorted: Vec<(&RegisteredFace, f32)> =
                            subject_best.into_values().collect();
                        sorted.sort_by(|a, b| {
                            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
                        });
                        sorted.truncate(top_k);
                        let resolved = sorted
                            .into_iter()
                            .enumerate()
                            .map(|(rank, (face, similarity))| FaceCandidateItem {
                                rank: rank + 1,
                                subject_id: face.subject_id.clone(),
                                subject_name: face.subject_name.clone(),
                                face_id: face.face_id.clone(),
                                photo_rel_path: face.photo_rel_path.clone(),
                                similarity,
                            })
                            .collect::<Vec<_>>();
                        (resolved_count, resolved)
                    };

                    if resolved_count < raw_cands.len() {
                        tracing::error!(
                            packaged_candidates = raw_cands.len(),
                            resolved = resolved_count,
                            "算法包 C ABI 底库候选无法完整映射到宿主快照，切换降级路径"
                        );
                        self.degrade_and_rebuild_algo_gallery(&gallery, "C ABI 候选无法映射")
                            .await;
                    } else {
                        return resolved;
                    }
                }
                Ok(_) => {
                    // 包内底库返回空候选。宿主有样本时这不是「未命中」，而是包内索引缺样本
                    // （例如某次写入静默失败）；当成未命中会让识别恒不返回结果且无法归因。
                    let host_samples = self.faces_snapshot().await.len();
                    if host_samples == 0 {
                        return Vec::new();
                    }
                    tracing::error!(
                        host_samples,
                        "算法包 C ABI 底库返回空候选但宿主存在样本，切换降级路径"
                    );
                    self.degrade_and_rebuild_algo_gallery(&gallery, "C ABI 检索候选为空")
                        .await;
                }
                Err(err) => {
                    tracing::warn!(error = %err, "C ABI 底库检索异常，回退至宿主备选检索路径");
                    self.degrade_and_rebuild_algo_gallery(&gallery, "C ABI 检索异常")
                        .await;
                }
            }
        }
        let Some(query_floats) = bytes_to_floats(query_bytes) else {
            return Vec::new();
        };

        let faces = self.faces_snapshot().await;
        if faces.is_empty() {
            return Vec::new();
        }

        tokio::task::spawn_blocking(move || {
            search_host_snapshot(faces, query_floats, top_k, min_threshold)
        })
        .await
        .unwrap_or_else(|err| {
            tracing::error!(error = %err, "宿主人脸底库回退检索任务失败");
            Vec::new()
        })
    }

    /// 返回 query 与不同主体底库样本之间 raw cosine 最大的一项。
    ///
    /// 算法包 C ABI 可用时，比较由算法包执行，宿主只读取 raw_score；
    /// 无 C ABI 时沿用现有 FP32 特征回退路径。异常特征或索引不一致时失败，
    /// 让录入方拒绝未完成校验的样本，而不是静默放行。
    pub async fn most_similar_cross_subject(
        &self,
        query_bytes: &[u8],
        excluded_subject_id: Option<&str>,
    ) -> Result<Option<CrossSubjectFaceMatch>, String> {
        if query_bytes.is_empty() {
            return Err("待校验的人脸特征为空".to_string());
        }

        let faces = self.faces_snapshot().await;
        let algo_gallery = self.algo_gallery.read().await.clone();
        if let Some(gallery) = algo_gallery.filter(|_| !self.gallery_sync_degraded()) {
            let query = query_bytes.to_vec();
            let worker_gallery = Arc::clone(&gallery);
            let search_result = tokio::task::spawn_blocking(
                move || -> Result<(usize, Vec<infer::RawFaceCandidate>), String> {
                    let count = worker_gallery.count().map_err(|err| err.to_string())? as usize;
                    if count == 0 {
                        return Ok((count, Vec::new()));
                    }
                    let top_k = count.min(256) as u32;
                    let candidates = worker_gallery
                        .search(&query, top_k, -1.0)
                        .map_err(|err| err.to_string())?;
                    Ok((count, candidates))
                },
            )
            .await
            .map_err(|err| format!("底库冲撞检索任务失败: {err}"))
            .and_then(|result| result);
            let (gallery_count, candidates) = match search_result {
                Ok(result) => result,
                Err(error) => {
                    self.degrade_and_rebuild_algo_gallery(&gallery, "跨主体冲撞检索")
                        .await;
                    return Err(format!("底库冲撞检索失败: {error}"));
                }
            };

            if gallery_count != faces.len() {
                self.degrade_and_rebuild_algo_gallery(&gallery, "跨主体冲撞索引数量不一致")
                    .await;
                return Err(format!(
                    "算法底库与宿主索引样本数不一致: algorithm={gallery_count}, host={}",
                    faces.len()
                ));
            }
            if gallery_count == 0 {
                return Ok(None);
            }
            if candidates.is_empty() {
                self.degrade_and_rebuild_algo_gallery(&gallery, "跨主体冲撞候选为空")
                    .await;
                return Err("算法底库未返回候选，无法完成冲撞校验".to_string());
            }

            let mut best: Option<CrossSubjectFaceMatch> = None;
            for candidate in candidates {
                if !candidate.raw_score.is_finite() {
                    self.degrade_and_rebuild_algo_gallery(&gallery, "C ABI raw_score 非有限")
                        .await;
                    return Err("算法底库返回了非有限 raw_score".to_string());
                }
                let Ok(position) = faces.binary_search_by_key(&candidate.id, |face| face.id) else {
                    self.degrade_and_rebuild_algo_gallery(&gallery, "C ABI 候选 ID 无法映射")
                        .await;
                    return Err(format!(
                        "算法底库返回了宿主索引中不存在的样本 ID: {}",
                        candidate.id
                    ));
                };
                let face = &faces[position];
                if excluded_subject_id == Some(face.subject_id.as_str()) {
                    continue;
                }

                if best
                    .as_ref()
                    .is_none_or(|current| candidate.raw_score > current.raw_cosine)
                {
                    best = Some(CrossSubjectFaceMatch {
                        subject_id: face.subject_id.clone(),
                        subject_name: face.subject_name.clone(),
                        face_id: face.face_id.clone(),
                        raw_cosine: candidate.raw_score,
                    });
                }
            }
            return Ok(best);
        }

        let query = bytes_to_floats(query_bytes)
            .filter(|query| !query.is_empty())
            .ok_or_else(|| "待校验的人脸特征不是有效 FP32 字节流".to_string())?;
        let excluded_subject_id = excluded_subject_id.map(str::to_owned);

        tokio::task::spawn_blocking(move || {
            let mut best: Option<CrossSubjectFaceMatch> = None;
            for face in faces.iter() {
                if excluded_subject_id.as_deref() == Some(face.subject_id.as_str()) {
                    continue;
                }
                // 与 `search_host_snapshot` 共用同一度量实现：
                // 两条宿主回退路径各写一份余弦，是分数口径漂移的温床。
                let raw_cosine = normalized_cosine_from_le_bytes(&query, &face.feature_bytes)
                    .ok_or_else(|| {
                        format!(
                            "底库样本 {} 无法计算 raw cosine（维度非法或非有限值）",
                            face.id
                        )
                    })?;
                if best
                    .as_ref()
                    .is_none_or(|current| raw_cosine > current.raw_cosine)
                {
                    best = Some(CrossSubjectFaceMatch {
                        subject_id: face.subject_id.clone(),
                        subject_name: face.subject_name.clone(),
                        face_id: face.face_id.clone(),
                        raw_cosine,
                    });
                }
            }
            Ok(best)
        })
        .await
        .map_err(|err| format!("底库冲撞回退检索任务失败: {err}"))?
    }

    /// 浮点切片向后兼容检索入口
    pub async fn search_top_k_floats(
        &self,
        query_vec: &[f32],
        top_k: usize,
        min_threshold: f32,
    ) -> Vec<FaceCandidateItem> {
        let bytes = embedding_to_le_bytes(query_vec);
        self.search_top_k(&bytes, top_k, min_threshold).await
    }

    /// 执行 1:N 余弦相似度比对，返回置信度最高且满足阈值的匹配结果
    pub async fn search(&self, query_vec: &[f32], threshold: f32) -> Option<FaceMatchResult> {
        let top1 = self.search_top_k_floats(query_vec, 1, threshold).await;
        top1.into_iter().next().map(|cand| FaceMatchResult {
            subject_id: cand.subject_id,
            subject_name: cand.subject_name,
            similarity: cand.similarity,
            face_id: cand.face_id,
            photo_rel_path: cand.photo_rel_path,
        })
    }

    /// 获取底库当前样本数量
    pub async fn count(&self) -> usize {
        self.faces_snapshot().await.len()
    }
}

fn search_host_snapshot(
    faces: Arc<Vec<RegisteredFace>>,
    query: Vec<f32>,
    top_k: usize,
    min_threshold: f32,
) -> Vec<FaceCandidateItem> {
    let mut subject_best: HashMap<&str, (&RegisteredFace, f32)> = HashMap::new();
    for face in faces.iter() {
        let Some(cosine) = normalized_cosine_from_le_bytes(&query, &face.feature_bytes) else {
            continue;
        };
        let similarity = megvii_calibrate_cosine(cosine);
        if similarity < min_threshold {
            continue;
        }

        match subject_best.get_mut(face.subject_id.as_str()) {
            Some(entry) if similarity > entry.1 => *entry = (face, similarity),
            Some(_) => {}
            None => {
                subject_best.insert(face.subject_id.as_str(), (face, similarity));
            }
        }
    }

    #[derive(Clone)]
    struct HeapItem<'a> {
        similarity: f32,
        face: &'a RegisteredFace,
    }

    impl PartialEq for HeapItem<'_> {
        fn eq(&self, other: &Self) -> bool {
            self.similarity == other.similarity
        }
    }
    impl Eq for HeapItem<'_> {}
    impl Ord for HeapItem<'_> {
        fn cmp(&self, other: &Self) -> Ordering {
            other
                .similarity
                .partial_cmp(&self.similarity)
                .unwrap_or(Ordering::Equal)
        }
    }
    impl PartialOrd for HeapItem<'_> {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    let mut heap: BinaryHeap<HeapItem<'_>> = BinaryHeap::with_capacity(top_k + 1);
    for (face, similarity) in subject_best.into_values() {
        heap.push(HeapItem { similarity, face });
        if heap.len() > top_k {
            heap.pop();
        }
    }

    let mut results: Vec<_> = heap.into_vec();
    results.sort_by(|left, right| {
        right
            .similarity
            .partial_cmp(&left.similarity)
            .unwrap_or(Ordering::Equal)
    });
    results
        .into_iter()
        .enumerate()
        .map(|(rank, item)| FaceCandidateItem {
            rank: rank + 1,
            subject_id: item.face.subject_id.clone(),
            subject_name: item.face.subject_name.clone(),
            similarity: item.similarity,
            face_id: item.face.face_id.clone(),
            photo_rel_path: item.face.photo_rel_path.clone(),
        })
        .collect()
}

/// 尝试从算法包创建并绑定 C ABI 底库句柄，并全量同步当前数据库底库
pub async fn sync_algo_gallery_from_package(
    gallery_index: &FaceFeatureIndex,
    db: &DatabaseConnection,
    pkg: &infer::AlgoPackage,
) {
    if pkg.supports_gallery() {
        match pkg.create_gallery() {
            Ok(gallery) => {
                gallery_index
                    .bind_algo_gallery(Some(std::sync::Arc::new(gallery)))
                    .await;
                if let Err(e) = gallery_index.reload(db).await {
                    tracing::warn!(error = %e, "同步人脸识别算法包 C ABI 底库失败");
                } else {
                    tracing::info!("已成功绑定并同步人脸识别算法包 C ABI 共享底库");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "创建人脸识别算法包 C ABI 底库句柄失败");
            }
        }
    } else {
        gallery_index.bind_algo_gallery(None).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_vector(val: f32) -> [f32; 512] {
        let mut vec = [0.0f32; 512];
        vec[0] = val;
        vec
    }

    fn make_cosine_test_vector(cosine: f32) -> [f32; 512] {
        let mut vec = [0.0f32; 512];
        vec[0] = cosine;
        vec[1] = (1.0 - cosine * cosine).sqrt();
        vec
    }

    #[tokio::test]
    async fn test_gallery_search_top_k_and_dedup() {
        let index = FaceFeatureIndex::new();

        index
            .upsert_faces(vec![
                RegisteredFace::from_512(
                    "subj_1".into(),
                    "Alice".into(),
                    "face_1a".into(),
                    "/photos/alice1.jpg".into(),
                    make_cosine_test_vector(0.82),
                ),
                RegisteredFace::from_512(
                    "subj_1".into(),
                    "Alice".into(),
                    "face_1b".into(),
                    "/photos/alice2.jpg".into(),
                    make_cosine_test_vector(0.95),
                ),
                RegisteredFace::from_512(
                    "subj_2".into(),
                    "Bob".into(),
                    "face_2".into(),
                    "".into(),
                    make_cosine_test_vector(0.88),
                ),
            ])
            .await;

        let query = make_test_vector(1.0);

        let top2 = index.search_top_k_floats(&query, 2, 0.50).await;
        assert_eq!(top2.len(), 2);
        assert_eq!(top2[0].subject_id, "subj_1");
        assert_eq!(top2[0].face_id, "face_1b");
        assert_eq!(top2[0].photo_rel_path, "/photos/alice2.jpg");
        assert_eq!(top2[1].subject_id, "subj_2");

        let best = index.search(&query, 0.50).await.expect("should match");
        assert_eq!(best.subject_id, "subj_1");

        // 验证改名：仅需更新宿主内存，后续检索立即生效
        index.update_subject_name("subj_1", "Alice Wonder").await;
        let best_renamed = index.search(&query, 0.50).await.expect("should match");
        assert_eq!(best_renamed.subject_name, "Alice Wonder");

        // 验证按照片删除单脸
        index.remove_face("face_1b").await;
        let top_after_del = index.search_top_k_floats(&query, 2, 0.50).await;
        assert_eq!(top_after_del[0].face_id, "face_2");

        // 验证按人员删除主体
        index.remove_subject("subj_2").await;
        let top_after_del_subj = index.search_top_k_floats(&query, 2, 0.50).await;
        assert_eq!(top_after_del_subj.len(), 1);
        assert_eq!(top_after_del_subj[0].subject_id, "subj_1");
        assert_eq!(top_after_del_subj[0].face_id, "face_1a");
    }

    #[tokio::test]
    async fn test_cross_subject_match_uses_cosine_and_excludes_same_subject() {
        let index = FaceFeatureIndex::new();
        let query = make_test_vector(2.0);
        let mut same_subject = make_test_vector(3.0);
        same_subject[1] = 4.0;
        let mut other_subject = [0.0f32; 512];
        other_subject[0] = 6.0 * 0.48;
        other_subject[1] = 6.0 * (1.0f32 - 0.48f32.powi(2)).sqrt();

        index
            .upsert_faces(vec![
                RegisteredFace::from_512(
                    "subject-a".into(),
                    "Alice".into(),
                    "face-a".into(),
                    String::new(),
                    same_subject,
                ),
                RegisteredFace::from_512(
                    "subject-b".into(),
                    "Bob".into(),
                    "face-b".into(),
                    String::new(),
                    other_subject,
                ),
            ])
            .await;

        let match_result = index
            .most_similar_cross_subject(&embedding_to_le_bytes(&query), Some("subject-a"))
            .await
            .expect("cross-subject lookup should succeed")
            .expect("a different subject should be returned");

        assert_eq!(match_result.subject_id, "subject-b");
        assert_eq!(match_result.face_id, "face-b");
        assert!((match_result.raw_cosine - 0.48).abs() < 1e-5);
    }

    /// 零变更时跳过包内索引的前提是「进入时未降级」。
    ///
    /// `finish_gallery_sync` 的 CAS 会把降级位一并清掉，所以降级中调用它
    /// 等于在没重建的情况下宣布索引已可信——这是规范明确禁止的。
    #[test]
    fn empty_delta_only_skips_gallery_touch_when_not_degraded() {
        assert!(can_skip_gallery_touch(0, 0, false));
        assert!(
            !can_skip_gallery_touch(0, 0, true),
            "降级中即使零变更也必须走重建路径"
        );
        assert!(!can_skip_gallery_touch(1, 0, false));
        assert!(!can_skip_gallery_touch(0, 1, false));
    }

    /// 降级中调 `finish_gallery_sync` 确实会丢掉降级位：钉住这个事实，
    /// 以免日后有人把它当作无副作用的「清版本位」工具而到处复用。
    #[test]
    fn finish_gallery_sync_clears_the_degraded_bit() {
        let index = FaceFeatureIndex::new();
        let (generation, was_degraded) = index.begin_gallery_sync();
        assert!(!was_degraded, "首次进入不应处于降级态");
        assert!(index.gallery_sync_degraded());

        // 伪造一次「重建失败持续降级」：版本不变、降级位保持
        assert!(index.finish_gallery_sync(generation));
        assert!(!index.gallery_sync_degraded());

        index
            .algo_sync_state
            .fetch_or(ALGO_SYNC_DEGRADED_BIT, AtomicOrdering::Release);
        assert!(index.gallery_sync_degraded());
        assert!(
            index.finish_gallery_sync(generation),
            "版本未被推进时 CAS 会成功并连带清掉降级位"
        );
        assert!(
            !index.gallery_sync_degraded(),
            "必须重建才能清降级位：调用方需自行保证这一点"
        );
    }

    /// 进入降级态的调用能看到 `was_degraded = true`，
    /// 这是调用方决定「走增量还是走重建」的唯一依据。
    #[test]
    fn begin_gallery_sync_reports_prior_degradation() {
        let index = FaceFeatureIndex::new();
        let (first_generation, was_degraded) = index.begin_gallery_sync();
        assert!(!was_degraded);

        let (second_generation, was_degraded) = index.begin_gallery_sync();
        assert!(was_degraded, "上一次同步未完成重建，降级位仍置位");
        assert!(second_generation > first_generation, "版本必须推进");
        assert!(
            !index.finish_gallery_sync(first_generation),
            "陈旧版本不得清位"
        );
        assert!(index.gallery_sync_degraded());
    }

    /// 维度非法的特征不得进入索引：否则检索静默缺项、冲撞校验 fail closed，
    /// 同一个索引在两条路径上表现出不同行为且都难以归因。
    #[tokio::test]
    async fn invalid_dimension_features_are_rejected_from_the_index() {
        let index = FaceFeatureIndex::new();

        index
            .upsert_faces(vec![
                RegisteredFace::from_512(
                    "subject-a".into(),
                    "Alice".into(),
                    "face-ok".into(),
                    String::new(),
                    make_test_vector(1.0),
                ),
                RegisteredFace::with_id(
                    999,
                    "subject-b".into(),
                    "Bob".into(),
                    "face-short".into(),
                    String::new(),
                    vec![0u8; 128],
                ),
                RegisteredFace::with_id(
                    1000,
                    "subject-c".into(),
                    "Carol".into(),
                    "face-particle".into(),
                    String::new(),
                    vec![0u8; 2047],
                ),
            ])
            .await;

        assert_eq!(index.count().await, 1, "只应保留维度合法的样本");

        let query = make_test_vector(1.0);
        let hits = index.search_top_k_floats(&query, 5, 0.0).await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].face_id, "face-ok");

        let cross = index
            .most_similar_cross_subject(&embedding_to_le_bytes(&query), Some("subject-a"))
            .await;
        assert_eq!(cross.expect("no invalid sample left to fail on"), None);
    }

    /// 全非法批次不得把原有索引清空。
    #[tokio::test]
    async fn fully_invalid_upsert_batch_leaves_the_index_untouched() {
        let index = FaceFeatureIndex::new();
        index
            .upsert_faces(vec![RegisteredFace::from_512(
                "subject-a".into(),
                "Alice".into(),
                "face-ok".into(),
                String::new(),
                make_test_vector(1.0),
            )])
            .await;

        index
            .upsert_faces(vec![RegisteredFace::with_id(
                7,
                "subject-b".into(),
                "Bob".into(),
                "face-bad".into(),
                String::new(),
                Vec::new(),
            )])
            .await;

        assert_eq!(index.count().await, 1);
    }

    /// 宿主回退检索与冲撞回退检索必须共用同一度量：
    /// 两条路径各写一份余弦是分数口径漂移的温床。
    ///
    /// 只要同一个查询向量在两条路径上对同一样本得到同一个分数，
    /// 就证明它们没有分叉。
    #[tokio::test]
    async fn host_fallback_search_and_clash_agree_on_the_same_score() {
        let index = FaceFeatureIndex::new();
        let sample = make_cosine_test_vector(0.62);
        index
            .upsert_faces(vec![RegisteredFace::from_512(
                "subject-a".into(),
                "Alice".into(),
                "face-a".into(),
                String::new(),
                sample,
            )])
            .await;

        let query = make_test_vector(1.0);
        let hits = index.search_top_k_floats(&query, 1, 0.0).await;
        assert_eq!(hits.len(), 1);

        let cross = index
            .most_similar_cross_subject(&embedding_to_le_bytes(&query), Some("nobody"))
            .await
            .expect("cross-subject lookup should succeed")
            .expect("the sample itself should be returned");

        // 检索路径把 cosine 过标定函数，冲撞路径直接给 raw cosine；
        // 标定函数在 0.62 处单调，因此两者应能互相还原。
        assert!(
            (megvii_calibrate_cosine(cross.raw_cosine) - hits[0].similarity).abs() < 1e-6,
            "两条宿主回退路径的余弦口径必须一致: raw={} calibrated={} vs search={}",
            cross.raw_cosine,
            megvii_calibrate_cosine(cross.raw_cosine),
            hits[0].similarity
        );
    }

    /// 重提取只换特征字节：数字 ID 与元数据必须原地保留。
    ///
    /// 走 `upsert_faces` 会整条替换，把姓名、照片路径清成调用方传入值；
    /// 这个测试钉住「只换向量」的契约。
    #[tokio::test]
    async fn replace_face_features_keeps_identity_and_metadata() {
        let index = FaceFeatureIndex::new();
        let original = RegisteredFace::from_512(
            "subject-a".into(),
            "Alice".into(),
            "face-a".into(),
            "/photos/alice.jpg".into(),
            make_cosine_test_vector(0.10),
        );
        let original_id = original.id;
        index.upsert_faces(vec![original]).await;

        index
            .replace_face_features(vec![(
                "face-a".into(),
                embedding_to_le_bytes(&make_test_vector(1.0)),
            )])
            .await;

        assert_eq!(index.count().await, 1, "不得新增条目");

        let query = make_test_vector(1.0);
        let hits = index.search_top_k_floats(&query, 1, 0.0).await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].subject_id, "subject-a");
        assert_eq!(hits[0].subject_name, "Alice", "姓名必须保留");
        assert_eq!(hits[0].photo_rel_path, "/photos/alice.jpg");
        assert!(
            hits[0].similarity > 0.99,
            "特征确实已更新为与查询同向的向量: {}",
            hits[0].similarity
        );

        // 数字 ID 不变意味着包内索引无需删插，只需原地换特征
        let faces = index.faces_snapshot().await;
        assert_eq!(faces[0].id, original_id);
    }

    /// 未知 face_id 与非法维度必须被静静跳过，不得破坏既有索引。
    #[tokio::test]
    async fn replace_face_features_ignores_unknown_and_invalid_entries() {
        let index = FaceFeatureIndex::new();
        index
            .upsert_faces(vec![RegisteredFace::from_512(
                "subject-a".into(),
                "Alice".into(),
                "face-a".into(),
                String::new(),
                make_test_vector(1.0),
            )])
            .await;

        index
            .replace_face_features(vec![
                (
                    "face-missing".into(),
                    embedding_to_le_bytes(&make_test_vector(1.0)),
                ),
                ("face-a".into(), vec![0u8; 16]),
            ])
            .await;

        assert_eq!(index.count().await, 1);
        let remaining = index.faces_snapshot().await;
        assert_eq!(
            remaining[0].feature_bytes.len(),
            EMBEDDING_BYTES_LEN,
            "非法维度的替换必须被拒绝，旧特征保持原样"
        );
    }

    /// `is_valid_embedding_bytes` 是「合法维度」的唯一定义，
    /// 只接受 512 维（2048 字节）的小端 FP32 字节流。
    #[test]
    fn embedding_length_predicate_is_the_single_definition() {
        assert!(is_valid_embedding_bytes(&vec![0u8; EMBEDDING_BYTES_LEN]));
        assert!(!is_valid_embedding_bytes(&[]));
        assert!(!is_valid_embedding_bytes(&vec![
            0u8;
            EMBEDDING_BYTES_LEN - 1
        ]));
        assert!(!is_valid_embedding_bytes(&vec![
            0u8;
            EMBEDDING_BYTES_LEN + 1
        ]));
        assert!(le_bytes_to_embedding(&vec![0u8; EMBEDDING_BYTES_LEN]).is_some());
        assert!(le_bytes_to_embedding(&[0u8; 4]).is_none());
    }
}
