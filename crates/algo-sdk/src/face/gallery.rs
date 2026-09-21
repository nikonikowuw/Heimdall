//! 人脸底库内存检索与置信度标定组件 (FaceGallery)
//!
//! 提供基于 RCU (读写分离不可变快照) 的多线程并发 1:N 向量检索，
//! 结合 SIMD 点积与模型度量标定，直接输出业务级百分比分数。

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::{Arc, RwLock};

use crate::c_abi::AvFaceCandidate;
use crate::math::cosine_similarity;

/// 旷视同款人脸识别置信度分段线性标定函数
///
/// 锚点映射：
/// - 0.00 -> 0.50 (512 维正交空间无偏基准)
/// - 0.10 -> 0.55 (底库负样本基线噪底)
/// - 0.40 -> 0.68 (疑似待复核门限)
/// - 0.48 -> 0.78 (高置信确认放行门限)
/// - 0.58 -> 0.884 (近景高质量时域融合命中)
/// - 1.00 -> 1.00 (理论满分)
pub fn megvii_calibrate_cosine(raw_cos: f32) -> f32 {
    if !raw_cos.is_finite() || raw_cos <= -1.0 {
        return 0.0;
    }
    if raw_cos < 0.10 {
        (0.50 + raw_cos * 0.50).clamp(0.0, 1.0)
    } else if raw_cos < 0.40 {
        (0.55 + (raw_cos - 0.10) * (13.0 / 30.0)).clamp(0.0, 1.0)
    } else if raw_cos < 0.48 {
        (0.68 + (raw_cos - 0.40) * 1.25).clamp(0.0, 1.0)
    } else if raw_cos < 0.58 {
        (0.78 + (raw_cos - 0.48) * 1.04).clamp(0.0, 1.0)
    } else if raw_cos >= 1.0 {
        1.0
    } else {
        (0.884 + (raw_cos - 0.58) * (29.0 / 105.0)).clamp(0.0, 1.0)
    }
}

/// 人脸底库样本实体 (仅保留纯向量与数字 ID，剥离业务元数据)
#[derive(Debug, Clone, PartialEq)]
pub struct GalleryFace {
    pub id: u64,
    pub feature: Vec<f32>,
}

/// 1:N 检索候选人明细
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateItem {
    pub rank: u32,
    pub id: u64,
    pub similarity: f32, // 标定后标准相似度 0.0 ~ 1.0
    pub raw_score: f32,  // 原始模型度量得分 (如余弦)
}

impl CandidateItem {
    /// 转换为 C ABI POD 结构体
    pub fn to_c_abi(&self) -> AvFaceCandidate {
        AvFaceCandidate {
            size: std::mem::size_of::<AvFaceCandidate>() as u32,
            rank: self.rank,
            id: self.id,
            similarity: self.similarity,
            raw_score: self.raw_score,
            reserved0: 0,
        }
    }
}

/// 不可变内存底库快照 (供多线程无锁并发读取)
#[derive(Debug, Clone, Default)]
struct GallerySnapshot {
    faces: Vec<GalleryFace>,
}

/// 线程安全、无锁并发人脸底库检索器
pub struct FaceGallery {
    snapshot: RwLock<Arc<GallerySnapshot>>,
    calibration_fn: fn(f32) -> f32,
}

impl std::fmt::Debug for FaceGallery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FaceGallery")
            .field("count", &self.count())
            .finish()
    }
}

impl Default for FaceGallery {
    fn default() -> Self {
        Self::new()
    }
}

impl FaceGallery {
    /// 创建基于旷视标定的底库检索器
    pub fn new() -> Self {
        Self::with_calibration(megvii_calibrate_cosine)
    }

    /// 创建指定标定函数的底库检索器
    pub fn with_calibration(calibration_fn: fn(f32) -> f32) -> Self {
        Self {
            snapshot: RwLock::new(Arc::new(GallerySnapshot::default())),
            calibration_fn,
        }
    }

    /// 全量清空底库
    pub fn clear(&self) {
        let mut guard = self
            .snapshot
            .write()
            .expect("gallery snapshot lock poisoned");
        *guard = Arc::new(GallerySnapshot::default());
    }

    /// 增量插入或更新一张人脸样本
    pub fn insert(&self, face: GalleryFace) {
        let mut guard = self
            .snapshot
            .write()
            .expect("gallery snapshot lock poisoned");
        let mut new_snapshot = (**guard).clone();
        if let Some(pos) = new_snapshot.faces.iter().position(|f| f.id == face.id) {
            new_snapshot.faces[pos] = face;
        } else {
            new_snapshot.faces.push(face);
        }
        *guard = Arc::new(new_snapshot);
    }

    /// 增量删除指定 ID 的人脸样本
    pub fn remove(&self, id: u64) {
        let mut guard = self
            .snapshot
            .write()
            .expect("gallery snapshot lock poisoned");
        let mut new_snapshot = (**guard).clone();
        new_snapshot.faces.retain(|f| f.id != id);
        *guard = Arc::new(new_snapshot);
    }

    /// 获取底库内有效样本总数
    pub fn count(&self) -> usize {
        let snapshot = self
            .snapshot
            .read()
            .expect("gallery snapshot lock poisoned")
            .clone();
        snapshot.faces.len()
    }

    /// 执行 1:N 向量检索 (返回 Top-K 候选人)
    pub fn search(
        &self,
        query_vec: &[f32],
        top_k: usize,
        min_threshold: f32,
    ) -> Vec<CandidateItem> {
        if query_vec.is_empty() || top_k == 0 {
            return Vec::new();
        }

        // 瞬间获取快照指针，立刻释放读锁（执行点积计算时完全无锁竞争）
        let snapshot = self
            .snapshot
            .read()
            .expect("gallery snapshot lock poisoned")
            .clone();
        if snapshot.faces.is_empty() {
            return Vec::new();
        }

        // 1. 利用最小堆选出 Top-K
        #[derive(Clone)]
        struct HeapItem<'a> {
            similarity: f32,
            raw_score: f32,
            face: &'a GalleryFace,
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

        for face in &snapshot.faces {
            if face.feature.len() != query_vec.len() {
                continue;
            }

            let raw_cos = cosine_similarity(query_vec, &face.feature);
            let sim = (self.calibration_fn)(raw_cos);

            if sim >= min_threshold {
                heap.push(HeapItem {
                    similarity: sim,
                    raw_score: raw_cos,
                    face,
                });
                if heap.len() > top_k {
                    heap.pop();
                }
            }
        }

        if heap.is_empty() {
            return Vec::new();
        }

        // 2. 从大到小排序输出
        let mut sorted: Vec<HeapItem<'_>> = heap.into_vec();
        sorted.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(Ordering::Equal)
        });

        sorted
            .into_iter()
            .enumerate()
            .map(|(i, item)| CandidateItem {
                rank: (i + 1) as u32,
                id: item.face.id,
                similarity: item.similarity,
                raw_score: item.raw_score,
            })
            .collect()
    }
}

/// 字节切片转换为 float 切片辅助函数
pub fn bytes_to_floats(bytes: &[u8]) -> Option<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut floats = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        floats.push(f32::from_le_bytes(
            chunk.try_into().expect("4-byte chunk conversion"),
        ));
    }
    Some(floats)
}

/// float 切片转换为小端字节切片辅助函数
pub fn floats_to_bytes(floats: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(floats.len() * 4);
    for val in floats {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_megvii_calibration_anchors() {
        assert_eq!(megvii_calibrate_cosine(0.0), 0.50);
        assert_eq!(megvii_calibrate_cosine(0.10), 0.55);
        assert_eq!(megvii_calibrate_cosine(0.40), 0.68);
        assert_eq!(megvii_calibrate_cosine(0.48), 0.78);
        assert!((megvii_calibrate_cosine(0.58) - 0.884).abs() < 1e-4);
        assert_eq!(megvii_calibrate_cosine(1.00), 1.00);

        // 负数正交截断
        assert_eq!(megvii_calibrate_cosine(-1.5), 0.0);
        assert_eq!(megvii_calibrate_cosine(f32::NAN), 0.0);
    }

    #[test]
    fn test_face_gallery_lifecycle_and_search() {
        let gallery = FaceGallery::new();
        assert_eq!(gallery.count(), 0);

        let f1 = vec![1.0, 0.0, 0.0];
        let f2 = vec![0.0, 1.0, 0.0];

        gallery.insert(GalleryFace {
            id: 1,
            feature: f1.clone(),
        });

        gallery.insert(GalleryFace {
            id: 2,
            feature: f2.clone(),
        });

        assert_eq!(gallery.count(), 2);

        // 用 f1 检索
        let results = gallery.search(&f1, 5, 0.0);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, 1);
        assert_eq!(results[0].rank, 1);
        assert_eq!(results[0].raw_score, 1.0);
        assert_eq!(results[0].similarity, 1.0);

        // 验证 C ABI 结构体转换
        let c_cand = results[0].to_c_abi();
        assert_eq!(c_cand.rank, 1);
        assert_eq!(c_cand.id, 1);
        assert_eq!(c_cand.similarity, 1.0);

        // 删除 ID 为 1 的样本
        gallery.remove(1);
        assert_eq!(gallery.count(), 1);
        let results_after_del = gallery.search(&f1, 5, 0.0);
        assert_eq!(results_after_del[0].id, 2);

        // 重新插入 ID 为 1 的样本
        gallery.insert(GalleryFace {
            id: 1,
            feature: f1.clone(),
        });
        assert_eq!(gallery.count(), 2);

        // 清空
        gallery.clear();
        assert_eq!(gallery.count(), 0);
    }

    #[test]
    fn test_bytes_floats_roundtrip() {
        let original = vec![1.23_f32, -4.56_f32, 7.89_f32];
        let bytes = floats_to_bytes(&original);
        assert_eq!(bytes.len(), 12);
        let recovered = bytes_to_floats(&bytes).expect("recovered floats");
        assert_eq!(original, recovered);

        assert!(bytes_to_floats(&[1, 2, 3]).is_none());
    }
}
