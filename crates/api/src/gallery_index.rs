use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;
use tokio::sync::RwLock;
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

/// 从小端字节流反序列化为 512 维 FP32 向量（向后兼容）
pub fn le_bytes_to_embedding(bytes: &[u8]) -> Option<[f32; 512]> {
    if bytes.len() != 2048 {
        return None;
    }
    let mut vector = [0.0f32; 512];
    for (&chunk, out) in bytes.as_chunks::<4>().0.iter().zip(vector.iter_mut()) {
        *out = f32::from_le_bytes(chunk);
    }
    Some(vector)
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

/// 人脸底库特征向量检索索引（支持 C ABI 共享底库委托与宿主无状态回退）
#[derive(Debug, Default)]
pub struct FaceFeatureIndex {
    faces: Arc<RwLock<Vec<RegisteredFace>>>,
    algo_gallery: Arc<RwLock<Option<Arc<RawAlgoGallery>>>>,
}

impl FaceFeatureIndex {
    pub fn new() -> Self {
        Self {
            faces: Arc::new(RwLock::new(Vec::new())),
            algo_gallery: Arc::new(RwLock::new(None)),
        }
    }

    /// 绑定或解绑底层 C ABI 算法包共享底库句柄
    pub async fn bind_algo_gallery(&self, gallery: Option<Arc<RawAlgoGallery>>) {
        let mut guard = self.algo_gallery.write().await;
        *guard = gallery;
    }

    /// 检查当前是否已绑定 C ABI 底库句柄
    pub async fn has_algo_gallery(&self) -> bool {
        self.algo_gallery.read().await.is_some()
    }

    /// 从数据库全量重新加载所有有效的人脸特征样本
    pub async fn reload(&self, db: &DatabaseConnection) -> Result<usize, DbError> {
        // 1. 获取所有人员姓名映射表 (subject_id -> name)
        let (all_personnel, _) = PersonnelRepo::list_filtered(db, None, 100_000, 0).await?;
        let name_map: HashMap<String, String> = all_personnel
            .into_iter()
            .map(|p| (p.subject_id, p.name))
            .collect();

        // 2. 获取所有已录入的人脸特征样本
        let raw_faces = GalleryFaceRepo::list_all_valid_vectors(db).await?;
        let mut loaded = Vec::with_capacity(raw_faces.len());

        let algo_gallery = self.algo_gallery.read().await.clone();
        if let Some(ref g) = algo_gallery {
            let _ = g.clear();
        }

        for face in raw_faces {
            if face.feature_vector.is_empty() {
                continue;
            }

            let subject_name = name_map
                .get(&face.subject_id)
                .cloned()
                .unwrap_or_else(|| face.subject_id.clone());

            let registered = RegisteredFace::with_id(
                face.id as u64,
                face.subject_id,
                subject_name,
                face.face_id,
                face.photo_rel_path,
                face.feature_vector,
            );

            if let Some(ref g) = algo_gallery {
                if let Err(e) = g.insert(registered.id, &registered.feature_bytes) {
                    tracing::warn!(
                        face_id = %registered.face_id,
                        error = %e,
                        "C ABI 底库样本同步失败"
                    );
                }
            }

            loaded.push(registered);
        }

        let count = loaded.len();
        {
            let mut guard = self.faces.write().await;
            *guard = loaded;
        }

        tracing::info!(
            count = count,
            has_c_abi = algo_gallery.is_some(),
            "人脸底库特征内存索引全量加载/更新成功"
        );
        Ok(count)
    }

    /// 增量批量插入或更新人脸特征样本
    pub async fn upsert_faces(&self, new_faces: Vec<RegisteredFace>) {
        if new_faces.is_empty() {
            return;
        }

        let algo_gallery = self.algo_gallery.read().await.clone();
        if let Some(ref g) = algo_gallery {
            for face in &new_faces {
                let _ = g.insert(face.id, &face.feature_bytes);
            }
        }

        let mut guard = self.faces.write().await;
        for face in new_faces {
            if let Some(pos) = guard.iter().position(|f| f.face_id == face.face_id) {
                guard[pos] = face;
            } else {
                guard.push(face);
            }
        }
    }

    /// 增量删除单张人脸特征样本
    pub async fn remove_face(&self, face_id: &str) {
        let removed_id = {
            let mut guard = self.faces.write().await;
            guard
                .iter()
                .position(|f| f.face_id == face_id)
                .map(|pos| guard.swap_remove(pos).id)
        };

        if let Some(id) = removed_id {
            let algo_gallery = self.algo_gallery.read().await.clone();
            if let Some(ref g) = algo_gallery {
                let _ = g.remove(id);
            }
        }
    }

    /// 增量删除指定人员的所有特征样本
    pub async fn remove_subject(&self, subject_id: &str) {
        let removed_ids: Vec<u64> = {
            let mut guard = self.faces.write().await;
            let mut ids = Vec::new();
            guard.retain(|f| {
                if f.subject_id == subject_id {
                    ids.push(f.id);
                    false
                } else {
                    true
                }
            });
            ids
        };

        if !removed_ids.is_empty() {
            let algo_gallery = self.algo_gallery.read().await.clone();
            if let Some(ref g) = algo_gallery {
                for id in removed_ids {
                    let _ = g.remove(id);
                }
            }
        }
    }

    /// 增量更新指定人员姓名 (仅更新宿主元数据，C ABI 无需变更)
    pub async fn update_subject_name(&self, subject_id: &str, new_name: &str) {
        let mut guard = self.faces.write().await;
        for face in guard.iter_mut() {
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

        // 1. 优先调用算法包导出的 C ABI 共享底库 (纯向量计算与 Top-K)
        let algo_gallery = self.algo_gallery.read().await.clone();
        if let Some(ref g) = algo_gallery {
            // 获取稍大的候选池（如 top_k * 8，下限 32，上限 256），由宿主按人员 (subject_id) 进行去重与元数据组装
            let fetch_limit = (top_k * 8).clamp(32, 256) as u32;
            match g.search(query_bytes, fetch_limit, min_threshold) {
                Ok(raw_cands) if !raw_cands.is_empty() => {
                    let guard = self.faces.read().await;
                    let id_map: HashMap<u64, &RegisteredFace> =
                        guard.iter().map(|f| (f.id, f)).collect();

                    let mut subject_best: HashMap<&str, (&RegisteredFace, f32)> = HashMap::new();
                    for c in raw_cands {
                        if let Some(face) = id_map.get(&c.id) {
                            match subject_best.get_mut(face.subject_id.as_str()) {
                                Some(entry) => {
                                    if c.similarity > entry.1 {
                                        *entry = (face, c.similarity);
                                    }
                                }
                                None => {
                                    subject_best
                                        .insert(face.subject_id.as_str(), (face, c.similarity));
                                }
                            }
                        }
                    }

                    if !subject_best.is_empty() {
                        let mut sorted: Vec<(&RegisteredFace, f32)> =
                            subject_best.into_values().collect();
                        sorted.sort_by(|a, b| {
                            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
                        });
                        sorted.truncate(top_k);

                        return sorted
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
                            .collect();
                    }
                }
                Ok(_) => {
                    return Vec::new();
                }
                Err(err) => {
                    tracing::warn!(error = %err, "C ABI 底库检索异常，回退至宿主备选检索路径");
                }
            }
        }

        // 2. 宿主备用检索路径
        let Some(query_floats) = bytes_to_floats(query_bytes) else {
            return Vec::new();
        };

        let guard = self.faces.read().await;
        if guard.is_empty() {
            return Vec::new();
        }

        // 同一人员聚合最优得分样本
        let mut subject_best: HashMap<&str, (&RegisteredFace, f32)> = HashMap::new();
        for face in guard.iter() {
            let Some(face_floats) = bytes_to_floats(&face.feature_bytes) else {
                continue;
            };
            if face_floats.len() != query_floats.len() {
                continue;
            }

            let dot: f32 = query_floats
                .iter()
                .zip(face_floats.iter())
                .map(|(&a, &b)| a * b)
                .sum();
            let similarity = megvii_calibrate_cosine(dot);

            if similarity >= min_threshold {
                match subject_best.get_mut(face.subject_id.as_str()) {
                    Some(entry) => {
                        if similarity > entry.1 {
                            *entry = (face, similarity);
                        }
                    }
                    None => {
                        subject_best.insert(face.subject_id.as_str(), (face, similarity));
                    }
                }
            }
        }

        if subject_best.is_empty() {
            return Vec::new();
        }

        // 维护 Top-K 最小堆
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
        for (_subj_id, (face, similarity)) in subject_best {
            heap.push(HeapItem { similarity, face });
            if heap.len() > top_k {
                heap.pop();
            }
        }

        let mut results = Vec::with_capacity(heap.len());
        while let Some(item) = heap.pop() {
            results.push(item);
        }
        results.reverse();

        results
            .into_iter()
            .enumerate()
            .map(|(i, item)| FaceCandidateItem {
                rank: i + 1,
                subject_id: item.face.subject_id.clone(),
                subject_name: item.face.subject_name.clone(),
                similarity: item.similarity,
                face_id: item.face.face_id.clone(),
                photo_rel_path: item.face.photo_rel_path.clone(),
            })
            .collect()
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
        let algo_gallery = self.algo_gallery.read().await.clone();
        if let Some(ref g) = algo_gallery {
            if let Ok(c) = g.count() {
                return c as usize;
            }
        }
        self.faces.read().await.len()
    }
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
                    make_test_vector(0.82),
                ),
                RegisteredFace::from_512(
                    "subj_1".into(),
                    "Alice".into(),
                    "face_1b".into(),
                    "/photos/alice2.jpg".into(),
                    make_test_vector(0.95),
                ),
                RegisteredFace::from_512(
                    "subj_2".into(),
                    "Bob".into(),
                    "face_2".into(),
                    "".into(),
                    make_test_vector(0.88),
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
}
