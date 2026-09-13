//! 人体检测与人脸检测空间几何二分图挂载引擎
//!
//! 1. 基于人体上半身头肩先验 [y1, y1 + 0.45H] 过滤空间候选；
//! 2. 尺度合理性校验 (过滤背景远景微小人脸误挂载到近景大人体)；
//! 3. 归一化欧式中心距离二分图贪婪匹配；
//! 4. 极端近景特写自适应推导虚拟躯干 (Pseudo-body) 保底，防止特写镜头漏跟。

use crate::detect::RawFace;

/// 人体检测候选框，坐标为归一化 `[0.0, 1.0]` 或输入像素空间 `[x, y, w, h]`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PersonCandidate {
    pub bbox: [f32; 4], // [x, y, w, h]
    pub score: f32,
}

/// 匹配后的人体-人脸对
#[derive(Debug, Clone)]
pub struct AssociatedPerson {
    pub person_bbox: [f32; 4],
    pub person_score: f32,
    pub is_pseudo_body: bool,
    pub attached_face: Option<RawFace>,
}

/// 执行人体与人脸的空间几何二分图关联挂载
pub fn associate_persons_and_faces(
    persons: &[PersonCandidate],
    faces: &[RawFace],
) -> Vec<AssociatedPerson> {
    let mut person_matched = vec![false; persons.len()];
    let mut face_matched = vec![false; faces.len()];

    let mut matches = Vec::new();

    // 1. 计算所有人脸与人体的几何匹配代价
    for (p_idx, p) in persons.iter().enumerate() {
        let pw = p.bbox[2];
        let ph = p.bbox[3];
        if pw <= 0.0 || ph <= 0.0 {
            continue;
        }
        let px1 = p.bbox[0];
        let py1 = p.bbox[1];
        let px2 = px1 + pw;

        // 头部先验锚点 (位于身体中轴，顶部向下 15% 处)
        let anchor_x = px1 + pw * 0.5;
        let anchor_y = py1 + ph * 0.15;

        for (f_idx, f) in faces.iter().enumerate() {
            let fw = f.bbox[2];
            let fh = f.bbox[3];
            if fw <= 0.0 || fh <= 0.0 {
                continue;
            }
            let fcx = f.bbox[0] + fw * 0.5;
            let fcy = f.bbox[1] + fh * 0.5;

            // 垂直范围约束：人脸中心应落在上半身 [y1 - 0.05H, y1 + 0.50H]
            if fcy < py1 - 0.05 * ph || fcy > py1 + 0.50 * ph {
                continue;
            }

            // 水平范围约束：人脸中心应落在人体水平跨度附近
            if fcx < px1 - 0.15 * pw || fcx > px2 + 0.15 * pw {
                continue;
            }

            // 尺度约束：人脸面积应小于人体面积的 45%，且大于 0.5%
            let area_ratio = (fw * fh) / (pw * ph);
            if !(0.005..=0.45).contains(&area_ratio) {
                continue;
            }

            // 归一化欧式几何距离
            let dx = (fcx - anchor_x) / pw;
            let dy = (fcy - anchor_y) / (0.45 * ph);
            let cost = (dx * dx + dy * dy).sqrt();

            if cost < 1.0 {
                matches.push((cost, p_idx, f_idx));
            }
        }
    }

    // 按距离代价升序排序（最优先匹配）
    matches.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));

    let mut results = Vec::new();

    // 2. 贪婪提取匹配对
    for (_cost, p_idx, f_idx) in matches {
        if !person_matched[p_idx] && !face_matched[f_idx] {
            person_matched[p_idx] = true;
            face_matched[f_idx] = true;
            results.push(AssociatedPerson {
                person_bbox: persons[p_idx].bbox,
                person_score: persons[p_idx].score,
                is_pseudo_body: false,
                attached_face: Some(faces[f_idx]),
            });
        }
    }

    // 3. 处理未匹配到人脸的人体 (背身、低头、遮挡)
    for (p_idx, p) in persons.iter().enumerate() {
        if !person_matched[p_idx] {
            results.push(AssociatedPerson {
                person_bbox: p.bbox,
                person_score: p.score,
                is_pseudo_body: false,
                attached_face: None,
            });
        }
    }

    // 4. 处理未匹配到人体的人脸 (近景特写大头照自适应推导虚拟躯干 Pseudo-body)
    for (f_idx, f) in faces.iter().enumerate() {
        if !face_matched[f_idx] {
            let fw = f.bbox[2];
            let fh = f.bbox[3];
            let fcx = f.bbox[0] + fw * 0.5;

            // 根据人脸黄金比例外推躯干尺寸
            let pw = (fw * 2.5).min(1.0);
            let ph = (fh * 5.0).min(1.0);
            let px1 = (fcx - pw * 0.5).max(0.0);
            let py1 = f.bbox[1].max(0.0);

            results.push(AssociatedPerson {
                person_bbox: [px1, py1, pw, ph],
                person_score: f.score * 0.9,
                is_pseudo_body: true,
                attached_face: Some(*f),
            });
        }
    }

    results
}

/// 为当前活跃的航迹列表互斥关联最佳的人体-人脸对
///
/// 遵循全局单射匹配原则，按 IoU 降序贪婪提取不相交对，保证同一目标绝不被多路航迹串挂。
/// 返回与 `active_tracks` 严格一一对应的 `Vec<Option<AssociatedPerson>>`。
pub fn match_tracks_to_associated(
    active_tracks: &[crate::bytetrack::STrack],
    associated: &[AssociatedPerson],
    iou_threshold: f32,
) -> Vec<Option<AssociatedPerson>> {
    let mut pairs = Vec::with_capacity(active_tracks.len() * associated.len());
    for (t_idx, track) in active_tracks.iter().enumerate() {
        for (a_idx, a) in associated.iter().enumerate() {
            let iou = crate::bytetrack::box_iou(&track.bbox, &a.person_bbox);
            if iou >= iou_threshold {
                pairs.push((iou, t_idx, a_idx));
            }
        }
    }

    // 优先匹配重合度最高的对
    pairs.sort_unstable_by(|x, y| y.0.total_cmp(&x.0));

    let mut track_matched = vec![false; active_tracks.len()];
    let mut assoc_matched = vec![false; associated.len()];
    let mut results = vec![None; active_tracks.len()];

    for (_iou, t_idx, a_idx) in pairs {
        if !track_matched[t_idx] && !assoc_matched[a_idx] {
            track_matched[t_idx] = true;
            assoc_matched[a_idx] = true;
            results[t_idx] = Some(associated[a_idx].clone());
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normal_person_face_association() {
        let persons = vec![PersonCandidate {
            bbox: [100.0, 100.0, 100.0, 300.0],
            score: 0.9,
        }];
        let faces = vec![RawFace {
            bbox: [130.0, 120.0, 40.0, 50.0],
            landmarks: [[0.0; 2]; 5],
            landmark_scores: [0.9; 5],
            score: 0.88,
        }];

        let associated = associate_persons_and_faces(&persons, &faces);
        assert_eq!(associated.len(), 1);
        assert!(associated[0].attached_face.is_some());
        assert!(!associated[0].is_pseudo_body);
        assert_eq!(
            associated[0]
                .attached_face
                .as_ref()
                .expect("attached_face 应存在")
                .score,
            faces[0].score
        );
    }

    #[test]
    fn test_unmatched_face_generates_pseudo_body() {
        let persons = vec![];
        let faces = vec![RawFace {
            bbox: [0.2, 0.2, 0.1, 0.12],
            landmarks: [[0.0; 2]; 5],
            landmark_scores: [0.9; 5],
            score: 0.85,
        }];

        let associated = associate_persons_and_faces(&persons, &faces);
        assert_eq!(associated.len(), 1);
        assert!(associated[0].is_pseudo_body);
        assert!(associated[0].person_bbox[2] > faces[0].bbox[2]);
        assert!(associated[0].person_bbox[3] > faces[0].bbox[3]);
    }

    #[test]
    fn test_match_tracks_to_associated_mutual_exclusivity() {
        use crate::bytetrack::{STrack, TrackDetection};

        let det0 = TrackDetection {
            bbox: [0.1, 0.1, 0.2, 0.4],
            score: 0.9,
            class_id: 0,
        };
        let det1 = TrackDetection {
            bbox: [0.12, 0.1, 0.2, 0.4],
            score: 0.85,
            class_id: 0,
        };
        let mut t0 = STrack::new(&det0, 1);
        t0.activate(1, 1);
        let mut t1 = STrack::new(&det1, 1);
        t1.activate(2, 1);

        let active_tracks = vec![t0, t1];
        // 只有一个目标人体框，且两路航迹均与其有重叠
        let associated = vec![AssociatedPerson {
            person_bbox: [0.1, 0.1, 0.2, 0.4],
            person_score: 0.95,
            is_pseudo_body: false,
            attached_face: None,
        }];

        let matches = match_tracks_to_associated(&active_tracks, &associated, 0.30);
        assert_eq!(matches.len(), 2);
        // t0 与目标完美重合 (IoU 1.0)，优先获得该目标
        assert!(matches[0].is_some());
        // t1 虽与目标有重合，但因目标已被 t0 独占互斥，故为 None，杜绝串挂
        assert!(matches[1].is_none(), "必须保证单射性，杜绝多航迹争抢串挂！");
    }
}
