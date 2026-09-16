//! 统一几何规则引擎 (Geometric Rules Engine)
//!
//! 1. 区域遮罩 (Mask): 静默过滤屏蔽区域内的目标；
//! 2. 多边形入侵 (Roi): 检测目标底面中心点是否侵入布防多边形；
//! 3. 绊线越界 (Line): 结合航迹历史移动向量检测是否越界（单向 A->B, B->A 或双向）；
//! 4. 防刷屏机制: 每一个 (track_id, rule_id) 具备独立的 5 秒防重复报警冷却；
//! 5. 识别类通行抓拍的空间触发判定 (`is_capture_triggering`)，结算时机与冷却由
//!    `capture_settle` 模块统一承担。

use types::{DetectionRule, DetectionRuleRole, TrackedObject};

use crate::geometry::{check_line_crossing, point_in_polygon};
use crate::tracker::ByteTrack;

/// 默认全屏布防规则索引标识（区别于用户显式配置的具体几何规则索引 0, 1, 2...）
pub const DEFAULT_FULLSCREEN_RULE_INDEX: usize = usize::MAX;

/// 规则标识枚举，避免直接依赖裸数值哨兵
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleIdentifier {
    /// 用户配置的具体几何规则索引 (0, 1, 2...)
    Custom(usize),
    /// 未配置任何正向几何规则时的默认全屏 ROI 兜底感知
    FullScreenDefault,
}

impl RuleIdentifier {
    /// 获取规则索引数值
    pub fn index(self) -> usize {
        match self {
            RuleIdentifier::Custom(idx) => idx,
            RuleIdentifier::FullScreenDefault => DEFAULT_FULLSCREEN_RULE_INDEX,
        }
    }

    /// 转换为稳定规则标识符
    pub fn stable_id(self) -> String {
        match self {
            RuleIdentifier::Custom(idx) => format!("rule_{idx}"),
            RuleIdentifier::FullScreenDefault => "fullscreen_default".to_string(),
        }
    }

    /// 从索引构造
    pub fn from_index(idx: usize) -> Self {
        if idx == DEFAULT_FULLSCREEN_RULE_INDEX {
            RuleIdentifier::FullScreenDefault
        } else {
            RuleIdentifier::Custom(idx)
        }
    }
}

/// 触发的告警详情
#[derive(Debug, Clone)]
pub struct TriggeredAlarm {
    pub rule_index: usize,
    pub role: DetectionRuleRole,
    pub tracked_object: TrackedObject,
    pub occurred_at_ms: i64,
}

impl TriggeredAlarm {
    /// 获取对应的规则标识符枚举
    pub fn rule_identifier(&self) -> RuleIdentifier {
        RuleIdentifier::from_index(self.rule_index)
    }

    /// 获取稳定规则标识字符串（用于落库与跨层通知，杜绝裸数值歧义）
    pub fn stable_rule_id(&self) -> String {
        self.rule_identifier().stable_id()
    }
}

#[inline]
fn is_object_masked(rules: &[DetectionRule], point: (f64, f64)) -> bool {
    rules
        .iter()
        .any(|r| r.role == DetectionRuleRole::Mask && point_in_polygon(point, &r.points))
}

#[inline]
fn check_rule_triggered(
    rule: &DetectionRule,
    point: (f64, f64),
    trajectory: &[(f64, f64)],
) -> bool {
    match rule.role {
        DetectionRuleRole::Mask | DetectionRuleRole::Precrop => false,
        DetectionRuleRole::Roi => point_in_polygon(point, &rule.points),
        DetectionRuleRole::Line => {
            trajectory.len() >= 2
                && rule.points.len() >= 2
                && check_line_crossing(
                    trajectory[trajectory.len() - 2],
                    trajectory[trajectory.len() - 1],
                    rule.points[0],
                    rule.points[1],
                    rule.line_direction,
                )
        }
    }
}

/// 识别类通行抓拍的空间触发判定（不含冷却）。
///
/// 语义与告警一致：Mask 遮罩内静默过滤；配置了正向规则（ROI/Line）时仅命中时触发；
/// 未配置正向规则时默认全屏感应布防。抓拍冷却与结算时机由
/// `crate::capture_settle::CaptureSettleController` 统一管理，本函数保持纯几何语义。
#[inline]
pub(crate) fn is_capture_triggering(rules: &[DetectionRule], obj: &TrackedObject) -> bool {
    // 人脸识别类任务的人员目标：若当前帧未检测到人脸（如背身、低头），
    // 暂不进行抓拍判定，等待其转正脸时再触发。
    if obj.label == "person" && obj.face.is_none() {
        return false;
    }

    let bottom_center = obj.bbox.bottom_center();
    if is_object_masked(rules, bottom_center) {
        return false;
    }

    let has_positive_rules = rules
        .iter()
        .any(|r| matches!(r.role, DetectionRuleRole::Roi | DetectionRuleRole::Line));

    if !has_positive_rules {
        return true;
    }

    rules
        .iter()
        .any(|rule| check_rule_triggered(rule, bottom_center, &obj.trajectory))
}

/// 几何规则引擎
#[derive(Debug, Default)]
pub struct RuleEvaluator;

impl RuleEvaluator {
    pub fn new() -> Self {
        Self
    }

    /// 执行规则判定并返回触发的告警集合
    ///
    /// 工业级默认感知规范：
    /// 若未配置任何正向几何检测规则 (无 ROI 区域与 Line 绊线)，系统按安防监控心智默认启用
    /// 全屏 ROI 感知布防 (Full-screen Coverage 兜底)，确保任务在未划定局部区域时仍可正常识别并抓拍。
    pub fn evaluate(
        &self,
        rules: &[DetectionRule],
        tracked_objects: &[TrackedObject],
        tracker: &mut ByteTrack,
        current_time_ms: i64,
        cooldown_ms: i64,
    ) -> Vec<TriggeredAlarm> {
        let mut alarms = Vec::new();

        let has_positive_rules = rules
            .iter()
            .any(|r| matches!(r.role, DetectionRuleRole::Roi | DetectionRuleRole::Line));

        for obj in tracked_objects {
            let bottom_center = obj.bbox.bottom_center();

            if is_object_masked(rules, bottom_center) {
                continue;
            }

            // 规则评估：
            // - 未配置正向规则时：默认全屏感应布防，目标只要在有效画面非遮罩区域内即触发告警与抓拍（受冷却保护）
            // - 已配置正向规则时：严格根据用户配置的 ROI 多边形或 Line 绊线逐一评估
            let mut try_record_alarm = |rule_index: usize, role: DetectionRuleRole| {
                if tracker.check_and_mark_rule(
                    obj.track_id,
                    rule_index,
                    role,
                    current_time_ms,
                    cooldown_ms,
                ) {
                    alarms.push(TriggeredAlarm {
                        rule_index,
                        role,
                        tracked_object: obj.clone(),
                        occurred_at_ms: current_time_ms,
                    });
                }
            };

            if !has_positive_rules {
                try_record_alarm(DEFAULT_FULLSCREEN_RULE_INDEX, DetectionRuleRole::Roi);
            } else {
                for (rule_idx, rule) in rules.iter().enumerate() {
                    if check_rule_triggered(rule, bottom_center, &obj.trajectory) {
                        try_record_alarm(rule_idx, rule.role);
                    }
                }
            }
        }

        alarms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{BoundingBox, DetectionLineDirection, DetectionPoint};

    #[test]
    fn test_mask_filtering() {
        let evaluator = RuleEvaluator::new();
        let mut tracker = ByteTrack::new();

        let rules = vec![
            DetectionRule {
                role: DetectionRuleRole::Mask,
                line_direction: DetectionLineDirection::Both,
                points: vec![
                    DetectionPoint::new(0.0, 0.0),
                    DetectionPoint::new(0.5, 0.0),
                    DetectionPoint::new(0.5, 0.5),
                    DetectionPoint::new(0.0, 0.5),
                ],
            },
            DetectionRule {
                role: DetectionRuleRole::Roi,
                line_direction: DetectionLineDirection::Both,
                points: vec![
                    DetectionPoint::new(0.0, 0.0),
                    DetectionPoint::new(1.0, 0.0),
                    DetectionPoint::new(1.0, 1.0),
                    DetectionPoint::new(0.0, 1.0),
                ],
            },
        ];

        // 目标位于遮罩区域内 [0.2, 0.2]
        let obj_in_mask = TrackedObject {
            track_id: 1,
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.9,
            quality_score: None,
            embedding: None,
            bbox: BoundingBox::new(0.1, 0.1, 0.3, 0.3),
            face: None,
            trajectory: vec![(0.2, 0.3)],
        };

        let alarms = evaluator.evaluate(&rules, &[obj_in_mask], &mut tracker, 1000, 5000);
        assert!(alarms.is_empty(), "处于遮罩区域内的目标必须被静默过滤！");

        // 目标位于遮罩区域外 [0.7, 0.7]
        let obj_outside = TrackedObject {
            track_id: 2,
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.9,
            quality_score: None,
            embedding: None,
            bbox: BoundingBox::new(0.6, 0.6, 0.8, 0.8),
            face: None,
            trajectory: vec![(0.7, 0.8)],
        };

        let alarms = evaluator.evaluate(&rules, &[obj_outside], &mut tracker, 1000, 5000);
        assert_eq!(alarms.len(), 1, "遮罩外的目标应正常触发 ROI 报警！");
    }

    #[test]
    fn test_tripwire_crossing_and_cooldown() {
        let evaluator = RuleEvaluator::new();
        let mut tracker = ByteTrack::new();

        let rules = vec![DetectionRule {
            role: DetectionRuleRole::Line,
            line_direction: DetectionLineDirection::Both,
            points: vec![DetectionPoint::new(0.0, 0.5), DetectionPoint::new(1.0, 0.5)],
        }];

        // 目标跨越 Y = 0.5 绊线: (0.5, 0.3) -> (0.5, 0.7)
        let crossing_obj = TrackedObject {
            track_id: 10,
            class_id: 0,
            label: "car".to_string(),
            confidence: 0.95,
            quality_score: None,
            embedding: None,
            bbox: BoundingBox::new(0.4, 0.6, 0.6, 0.8),
            face: None,
            trajectory: vec![(0.5, 0.3), (0.5, 0.7)],
        };

        // 第一次穿越：触发
        let alarms1 = evaluator.evaluate(
            &rules,
            std::slice::from_ref(&crossing_obj),
            &mut tracker,
            2000,
            5000,
        );
        assert_eq!(alarms1.len(), 1);

        // 3 秒后目标再次跨越 (同一 track_id)：被 5 秒冷却拦截
        let alarms2 = evaluator.evaluate(
            &rules,
            std::slice::from_ref(&crossing_obj),
            &mut tracker,
            5000,
            5000,
        );
        assert!(alarms2.is_empty(), "5秒内同一目标同一规则不可重复告警");

        // 8 秒后目标再次跨越：允许告警
        let alarms3 = evaluator.evaluate(&rules, &[crossing_obj], &mut tracker, 7001, 5000);
        assert_eq!(alarms3.len(), 1);
    }

    #[test]
    fn test_default_fullscreen_roi_when_no_positive_rules() {
        let evaluator = RuleEvaluator::new();
        let mut tracker = ByteTrack::new();

        let obj = TrackedObject {
            track_id: 1,
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.95,
            quality_score: None,
            embedding: None,
            bbox: BoundingBox::new(0.4, 0.4, 0.6, 0.6),
            face: None,
            trajectory: vec![(0.5, 0.6)],
        };

        // 1. 当未配置任何规则时，默认按全屏 ROI 触发告警，索引使用全局统一全屏标识
        let alarms = evaluator.evaluate(&[], std::slice::from_ref(&obj), &mut tracker, 1000, 5000);
        assert_eq!(alarms.len(), 1, "未配置规则时默认应全屏感应告警");
        assert_eq!(alarms[0].role, DetectionRuleRole::Roi);
        assert_eq!(alarms[0].rule_index, DEFAULT_FULLSCREEN_RULE_INDEX);

        // 2. 5 秒防刷屏冷却生效
        let alarms_cooldown =
            evaluator.evaluate(&[], std::slice::from_ref(&obj), &mut tracker, 2000, 5000);
        assert!(alarms_cooldown.is_empty(), "防刷屏冷却期内不重复告警");

        // 3. 冷却期过后再次触发
        let alarms_after = evaluator.evaluate(&[], &[obj], &mut tracker, 7000, 5000);
        assert_eq!(alarms_after.len(), 1, "冷却期后应能再次触发");
    }

    #[test]
    fn test_default_fullscreen_with_mask_filtering() {
        let evaluator = RuleEvaluator::new();
        let mut tracker = ByteTrack::new();

        // 配置单一 Mask 遮罩，未配置任何正向几何规则
        let mask_rule = DetectionRule {
            role: DetectionRuleRole::Mask,
            line_direction: types::DetectionLineDirection::Both,
            points: vec![
                types::DetectionPoint::new(0.0, 0.0),
                types::DetectionPoint::new(0.3, 0.0),
                types::DetectionPoint::new(0.3, 0.3),
                types::DetectionPoint::new(0.0, 0.3),
            ],
        };
        let rules = vec![mask_rule];

        // 目标 A 落在 Mask 区域内 -> 必须被静默过滤
        let masked_obj = TrackedObject {
            track_id: 10,
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.9,
            quality_score: None,
            embedding: None,
            bbox: BoundingBox::new(0.05, 0.05, 0.2, 0.2),
            face: None,
            trajectory: vec![(0.125, 0.2)],
        };

        // 目标 B 落在非遮罩区域 -> 触发默认全屏告警，索引不能被错误赋予 0 (避免指代 mask_rule)
        let unmasked_obj = TrackedObject {
            track_id: 11,
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.92,
            quality_score: None,
            embedding: None,
            bbox: BoundingBox::new(0.5, 0.5, 0.7, 0.7),
            face: None,
            trajectory: vec![(0.6, 0.7)],
        };

        let alarms = evaluator.evaluate(
            &rules,
            &[masked_obj, unmasked_obj],
            &mut tracker,
            1000,
            5000,
        );
        assert_eq!(alarms.len(), 1, "遮罩区内目标过滤，非遮罩区触发告警");
        assert_eq!(alarms[0].tracked_object.track_id, 11);
        assert_eq!(alarms[0].role, DetectionRuleRole::Roi);
        assert_eq!(
            alarms[0].rule_index, DEFAULT_FULLSCREEN_RULE_INDEX,
            "全屏兜底告警索引必须与用户遮罩规则索引解耦"
        );
    }

    fn capture_obj(label: &str, bottom_y: f64, trajectory: Vec<(f64, f64)>) -> TrackedObject {
        TrackedObject {
            track_id: 1,
            class_id: 0,
            label: label.to_string(),
            confidence: 0.95,
            quality_score: Some(0.9),
            bbox: BoundingBox::new(0.4, (bottom_y - 0.3) as f32, 0.6, bottom_y as f32),
            face: None,
            embedding: None,
            trajectory,
        }
    }

    #[test]
    fn test_is_capture_triggering_fullscreen_mask_roi_line() {
        // 1. 未配置任何区域：识别类目标默认全屏触发。
        let obj = capture_obj("face", 0.6, vec![(0.5, 0.6)]);
        assert!(is_capture_triggering(&[], &obj));

        // 2. 人员目标当帧无脸：不触发（等待转正脸，避免无脸帧空转）。
        let person = capture_obj("person", 0.6, vec![(0.5, 0.6)]);
        assert!(!is_capture_triggering(&[], &person));

        // 3. Mask 遮罩内静默过滤（bottom_center 位于遮罩多边形内）。
        let mask = DetectionRule {
            role: DetectionRuleRole::Mask,
            line_direction: DetectionLineDirection::Both,
            points: vec![
                DetectionPoint::new(0.0, 0.0),
                DetectionPoint::new(0.3, 0.0),
                DetectionPoint::new(0.3, 0.3),
                DetectionPoint::new(0.0, 0.3),
            ],
        };
        let masked = TrackedObject {
            bbox: BoundingBox::new(0.05, 0.0, 0.15, 0.2),
            ..capture_obj("face", 0.2, vec![(0.1, 0.2)])
        };
        assert!(!is_capture_triggering(&[mask], &masked));

        // 4. 配置 ROI 时仅命中区域触发。
        let roi = DetectionRule {
            role: DetectionRuleRole::Roi,
            line_direction: DetectionLineDirection::Both,
            points: vec![
                DetectionPoint::new(0.0, 0.0),
                DetectionPoint::new(0.3, 0.0),
                DetectionPoint::new(0.3, 0.3),
                DetectionPoint::new(0.0, 0.3),
            ],
        };
        let inside = TrackedObject {
            bbox: BoundingBox::new(0.05, 0.0, 0.15, 0.2),
            ..capture_obj("face", 0.2, vec![(0.1, 0.2)])
        };
        let outside = capture_obj("face", 0.7, vec![(0.6, 0.7)]);
        assert!(is_capture_triggering(std::slice::from_ref(&roi), &inside));
        assert!(!is_capture_triggering(&[roi], &outside));

        // 5. 绊线：跨越触发，未跨越不触发。
        let line = DetectionRule {
            role: DetectionRuleRole::Line,
            line_direction: DetectionLineDirection::Both,
            points: vec![DetectionPoint::new(0.0, 0.5), DetectionPoint::new(1.0, 0.5)],
        };
        let crossing = capture_obj("face", 0.6, vec![(0.5, 0.4), (0.5, 0.6)]);
        let staying = capture_obj("face", 0.3, vec![(0.5, 0.2), (0.5, 0.3)]);
        assert!(is_capture_triggering(
            std::slice::from_ref(&line),
            &crossing
        ));
        assert!(!is_capture_triggering(&[line], &staying));
    }
}
