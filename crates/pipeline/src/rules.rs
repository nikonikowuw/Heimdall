//! 统一几何规则引擎 (Geometric Rules Engine)
//!
//! 1. 区域遮罩 (Mask): 静默过滤屏蔽区域内的目标；
//! 2. 多边形入侵 (Roi): 检测目标底面中心点是否侵入布防多边形；
//! 3. 绊线越界 (Line): 结合航迹历史移动向量检测是否越界（单向 A->B, B->A 或双向）；
//! 4. 防刷屏机制: 每一个 (track_id, rule_id) 具备独立的 5 秒防重复报警冷却。

use types::{DetectionRule, DetectionRuleRole, TrackedObject};

use crate::geometry::{check_line_crossing, point_in_polygon};
use crate::tracker::SimpleTracker;

/// 触发的告警详情
#[derive(Debug, Clone)]
pub struct TriggeredAlarm {
    pub rule_index: usize,
    pub role: DetectionRuleRole,
    pub tracked_object: TrackedObject,
    pub occurred_at_ms: i64,
}

/// 几何规则引擎
#[derive(Debug, Default)]
pub struct RuleEvaluator;

impl RuleEvaluator {
    pub fn new() -> Self {
        Self
    }

    /// 执行规则判定并返回触发的告警集合
    pub fn evaluate(
        &self,
        rules: &[DetectionRule],
        tracked_objects: &[TrackedObject],
        tracker: &mut SimpleTracker,
        current_time_ms: i64,
        cooldown_ms: i64,
    ) -> Vec<TriggeredAlarm> {
        let mut alarms = Vec::new();

        for obj in tracked_objects {
            let bottom_center = obj.bbox.bottom_center();

            // 1. 检查是否落在任一 Mask 遮罩区域内；若是则静默过滤（零堆分配）
            let is_masked = rules
                .iter()
                .filter(|r| r.role == DetectionRuleRole::Mask)
                .any(|r| point_in_polygon(bottom_center, &r.points));
            if is_masked {
                continue;
            }

            // 2. 对非遮罩目标遍历几何规则
            for (rule_idx, rule) in rules.iter().enumerate() {
                let triggered = match rule.role {
                    DetectionRuleRole::Mask => false,
                    DetectionRuleRole::Roi => point_in_polygon(bottom_center, &rule.points),
                    DetectionRuleRole::Line => {
                        obj.trajectory.len() >= 2
                            && rule.points.len() >= 2
                            && check_line_crossing(
                                obj.trajectory[obj.trajectory.len() - 2],
                                obj.trajectory[obj.trajectory.len() - 1],
                                rule.points[0],
                                rule.points[1],
                                rule.line_direction,
                            )
                    }
                };

                if triggered
                    && tracker.check_and_mark_rule(
                        obj.track_id,
                        rule_idx,
                        rule.role,
                        current_time_ms,
                        cooldown_ms,
                    )
                {
                    alarms.push(TriggeredAlarm {
                        rule_index: rule_idx,
                        role: rule.role,
                        tracked_object: obj.clone(),
                        occurred_at_ms: current_time_ms,
                    });
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
        let mut tracker = SimpleTracker::new();

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
            bbox: BoundingBox::new(0.1, 0.1, 0.3, 0.3),
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
            bbox: BoundingBox::new(0.6, 0.6, 0.8, 0.8),
            trajectory: vec![(0.7, 0.8)],
        };

        let alarms = evaluator.evaluate(&rules, &[obj_outside], &mut tracker, 1000, 5000);
        assert_eq!(alarms.len(), 1, "遮罩外的目标应正常触发 ROI 报警！");
    }

    #[test]
    fn test_tripwire_crossing_and_cooldown() {
        let evaluator = RuleEvaluator::new();
        let mut tracker = SimpleTracker::new();

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
            bbox: BoundingBox::new(0.4, 0.6, 0.6, 0.8),
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
}
