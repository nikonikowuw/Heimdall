use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use tokio::sync::RwLock;
use types::FaceCandidateItem;

use db::{DatabaseConnection, DbError, GalleryFaceRepo, PersonnelRepo};
use types::FaceMatchResult;

/// 512 维 FP32 向量序列化为标准小端字节流 (2048 bytes)
pub fn embedding_to_le_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for val in embedding {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

/// 从 2048 字节小端字节流反序列化为 512 维 FP32 向量
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

/// 内存常驻的人脸底库样本结构
#[derive(Debug, Clone)]
pub struct RegisteredFace {
    pub subject_id: String,
    pub subject_name: String,
    pub face_id: String,
    pub photo_rel_path: String,
    pub vector: [f32; 512],
}

/// 人脸底库内存特征向量检索索引（全量常驻，微秒级 1:N 检索，支持增量原子更新）
#[derive(Debug, Default)]
pub struct FaceFeatureIndex {
    faces: Arc<RwLock<Vec<RegisteredFace>>>,
}

impl FaceFeatureIndex {
    pub fn new() -> Self {
        Self {
            faces: Arc::new(RwLock::new(Vec::new())),
        }
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

        for face in raw_faces {
            let Some(vector) = le_bytes_to_embedding(&face.feature_vector) else {
                tracing::warn!(
                    face_id = %face.face_id,
                    len = face.feature_vector.len(),
                    "人脸特征向量尺寸不匹配 2048 字节 (512*4)，跳过载入"
                );
                continue;
            };

            let subject_name = name_map
                .get(&face.subject_id)
                .cloned()
                .unwrap_or_else(|| face.subject_id.clone());

            loaded.push(RegisteredFace {
                subject_id: face.subject_id,
                subject_name,
                face_id: face.face_id,
                photo_rel_path: face.photo_rel_path,
                vector,
            });
        }

        let count = loaded.len();
        {
            let mut guard = self.faces.write().await;
            *guard = loaded;
        }

        tracing::info!(count = count, "人脸底库特征内存索引全量加载/更新成功");
        Ok(count)
    }

    /// 增量批量插入或更新人脸特征样本
    pub async fn upsert_faces(&self, new_faces: Vec<RegisteredFace>) {
        if new_faces.is_empty() {
            return;
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
        let mut guard = self.faces.write().await;
        guard.retain(|f| f.face_id != face_id);
    }

    /// 增量删除指定人员的所有特征样本
    pub async fn remove_subject(&self, subject_id: &str) {
        let mut guard = self.faces.write().await;
        guard.retain(|f| f.subject_id != subject_id);
    }

    /// 增量更新指定人员姓名
    pub async fn update_subject_name(&self, subject_id: &str, new_name: &str) {
        let mut guard = self.faces.write().await;
        for face in guard.iter_mut() {
            if face.subject_id == subject_id {
                face.subject_name = new_name.to_string();
            }
        }
    }

    /// 执行 1:N 余弦相似度比对，返回 Top-K 候选人列表 (基于 Min-Heap 检索，单人员多特征取最优得分)
    pub async fn search_top_k(
        &self,
        query_vec: &[f32],
        top_k: usize,
        min_threshold: f32,
    ) -> Vec<FaceCandidateItem> {
        if query_vec.len() != 512 || top_k == 0 {
            return Vec::new();
        }

        let guard = self.faces.read().await;
        if guard.is_empty() {
            return Vec::new();
        }

        // 1. 同一人员聚合最优得分样本
        let mut subject_best: HashMap<&str, (&RegisteredFace, f32)> = HashMap::new();
        for face in guard.iter() {
            let dot: f32 = query_vec
                .iter()
                .zip(face.vector.iter())
                .map(|(&a, &b)| a * b)
                .sum();

            if dot >= min_threshold {
                match subject_best.get_mut(face.subject_id.as_str()) {
                    Some(entry) => {
                        if dot > entry.1 {
                            *entry = (face, dot);
                        }
                    }
                    None => {
                        subject_best.insert(face.subject_id.as_str(), (face, dot));
                    }
                }
            }
        }

        if subject_best.is_empty() {
            return Vec::new();
        }

        // 2. 借助 Min-Heap 维护最高 Top-K 得分
        #[derive(Clone)]
        struct HeapItem<'a> {
            similarity: f32,
            face: &'a RegisteredFace,
        }

        impl<'a> PartialEq for HeapItem<'a> {
            fn eq(&self, other: &Self) -> bool {
                self.similarity == other.similarity
            }
        }

        impl<'a> Eq for HeapItem<'a> {}

        impl<'a> Ord for HeapItem<'a> {
            fn cmp(&self, other: &Self) -> Ordering {
                other
                    .similarity
                    .partial_cmp(&self.similarity)
                    .unwrap_or(Ordering::Equal)
            }
        }

        impl<'a> PartialOrd for HeapItem<'a> {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        let mut heap: BinaryHeap<HeapItem> = BinaryHeap::with_capacity(top_k + 1);
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

    /// 执行 1:N 余弦相似度比对，返回置信度最高且满足阈值的匹配结果
    pub async fn search(&self, query_vec: &[f32], threshold: f32) -> Option<FaceMatchResult> {
        let top1 = self.search_top_k(query_vec, 1, threshold).await;
        top1.into_iter().next().map(|cand| FaceMatchResult {
            subject_id: cand.subject_id,
            subject_name: cand.subject_name,
            face_id: cand.face_id,
            photo_rel_path: cand.photo_rel_path,
            similarity: cand.similarity,
        })
    }

    /// 获取当前常驻内存的有效样本特征数
    pub async fn count(&self) -> usize {
        self.faces.read().await.len()
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

        // 插入同一人员 subj1 的两张照片 (相似度 0.82 与 0.95)
        index
            .upsert_faces(vec![
                RegisteredFace {
                    face_id: "face_1a".into(),
                    subject_id: "subj_1".into(),
                    subject_name: "Alice".into(),
                    photo_rel_path: "/photos/alice1.jpg".into(),
                    vector: make_test_vector(0.82),
                },
                RegisteredFace {
                    face_id: "face_1b".into(),
                    subject_id: "subj_1".into(),
                    subject_name: "Alice".into(),
                    photo_rel_path: "/photos/alice2.jpg".into(),
                    vector: make_test_vector(0.95),
                },
                RegisteredFace {
                    face_id: "face_2".into(),
                    subject_id: "subj_2".into(),
                    subject_name: "Bob".into(),
                    photo_rel_path: "".into(),
                    vector: make_test_vector(0.88),
                },
                RegisteredFace {
                    face_id: "face_3".into(),
                    subject_id: "subj_3".into(),
                    subject_name: "Charlie".into(),
                    photo_rel_path: "".into(),
                    vector: make_test_vector(0.70),
                },
                RegisteredFace {
                    face_id: "face_4".into(),
                    subject_id: "subj_4".into(),
                    subject_name: "David".into(),
                    photo_rel_path: "".into(),
                    vector: make_test_vector(0.60),
                },
            ])
            .await;

        let query = make_test_vector(1.0);

        // 检索 Top-2, 门限 0.75
        let top2 = index.search_top_k(&query, 2, 0.75).await;
        assert_eq!(top2.len(), 2);
        // Alice 最优得分为 0.95，排第一，且对应的特征照片引用应正确更新为 face_1b
        assert_eq!(top2[0].subject_id, "subj_1");
        assert_eq!(top2[0].face_id, "face_1b");
        assert_eq!(top2[0].photo_rel_path, "/photos/alice2.jpg");
        assert!((top2[0].similarity - 0.95).abs() < 1e-5);
        // Bob 得分 0.88，排第二
        assert_eq!(top2[1].subject_id, "subj_2");
        assert!((top2[1].similarity - 0.88).abs() < 1e-5);

        // search 单结果向后兼容
        let best = index.search(&query, 0.75).await.expect("should match");
        assert_eq!(best.subject_id, "subj_1");
        assert!((best.similarity - 0.95).abs() < 1e-5);
    }
}
