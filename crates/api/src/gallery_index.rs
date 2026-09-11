use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

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

    /// 执行 1:N 余弦相似度比对，返回置信度最高且满足阈值的匹配结果
    pub async fn search(&self, query_vec: &[f32], threshold: f32) -> Option<FaceMatchResult> {
        if query_vec.len() != 512 {
            return None;
        }

        let guard = self.faces.read().await;
        if guard.is_empty() {
            return None;
        }

        let mut best_match: Option<(&RegisteredFace, f32)> = None;

        for face in guard.iter() {
            // L2 归一化后的向量，点积即为余弦相似度
            let dot: f32 = query_vec
                .iter()
                .zip(face.vector.iter())
                .map(|(&a, &b)| a * b)
                .sum();

            if dot >= threshold {
                match best_match {
                    Some((_, current_max)) if dot > current_max => {
                        best_match = Some((face, dot));
                    }
                    None => {
                        best_match = Some((face, dot));
                    }
                    _ => {}
                }
            }
        }

        best_match.map(|(face, similarity)| FaceMatchResult {
            subject_id: face.subject_id.clone(),
            subject_name: face.subject_name.clone(),
            face_id: face.face_id.clone(),
            photo_rel_path: face.photo_rel_path.clone(),
            similarity,
        })
    }

    /// 获取当前常驻内存的有效样本特征数
    pub async fn count(&self) -> usize {
        self.faces.read().await.len()
    }
}
