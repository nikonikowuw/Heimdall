//! 工业级运动门控引擎 (Motion Gate)：差分判定本体与调度状态机
//!
//! 职责边界：
//! - 本文件：引擎状态、保活/余晖状态机、$8 \times 8$ 宏块网格差分与场景瞬变抑制；
//! - `mask`：Mask / ROI 规则在评估栅格上的惰性光栅化；
//! - `y_plane`：Host / CVPixelBuffer / DMA-BUF 载体的 Y 平面提取与平台 FFI；
//! - `telemetry`：绕过计数、按类首次告警与限速日志。
//!
//! 本模块不含异步逻辑：门控对象的线程独占性由调用方保证。生产路径把门控绑定到每路一个的
//! 专用 OS 线程（见 [`crate::motion_gate_worker`]），因为 DMA-BUF 载体会调用 RGA 平台 FFI。

mod mask;
mod telemetry;
mod y_plane;

pub use mask::MaskBitmap;

use tracing::warn;
use types::{DetectionRule, FrameRef, MotionGateConfig};

#[cfg(all(target_os = "linux", feature = "rga"))]
use media::motion_thumb::MotionThumbnailScaler;

use self::telemetry::{DecisionLog, GateTelemetry};

/// 门控缩略图目标高度（像素）
///
/// 门控只需回答"画面是否变化"，不需要定位精度。缩略图的面积平均会抹平压缩噪声与传感器热噪，
/// 使静止场景的有效差分像素数趋于 0，从而让 `contour_area` 能稳定区分"静止"与"真实运动"；
/// 直接在全分辨率帧上差分则相反：像素数随分辨率放大，阈值口径彻底失真。
///
/// 320×180（16:9 源）与主流 NVR 的检测分辨率惯例一致（如 Frigate 的 `detect` 默认 320×180），
/// 该尺度下每帧 Y 平面回读 57.6KB，与源分辨率无关。
const THUMBNAIL_HEIGHT: u32 = 180;
/// 缩略图宽度夹紧区间（16:9 源为 320；超宽/竖屏相机按同高换算后夹紧）
const THUMBNAIL_MIN_WIDTH: u32 = 64;
const THUMBNAIL_MAX_WIDTH: u32 = 640;
/// 网格宏块尺寸 (8x8 像素)
const CELL_SIZE: usize = 8;
/// 宏块激活所需的最小变动像素数 (64 像素中至少 25% 变动，彻底过滤散粒热噪)
const CELL_ACTIVE_MIN_PIXELS: u32 = 16;
/// 瞬时剧变抑制阈值比例 (全图变动超过 85% 判定为场景瞬变，如昼夜 IRCUT 切换或开关灯)
const SCENE_CHANGE_RATIO: f32 = 0.85;
/// 热度计算归一化敏感度系数 (全图有效变动达到 15% 时热度满格 1.0)
const MOTION_SCORE_ALPHA: f32 = 0.15;

/// 由源帧尺寸推导门控缩略图尺寸（同高换算，宽度 4 像素对齐、高度 2 像素对齐）。
///
/// 固定目标高度而非固定宽度：相机存在 16:9 / 4:3 / 竖屏多种画幅，按高换算可保证各画幅的
/// 垂直采样密度一致，避免竖屏相机被压成一条细带。
///
/// 宽度会被夹紧到 `THUMBNAIL_MIN_WIDTH..=THUMBNAIL_MAX_WIDTH`：夹紧意味着**放弃宽高比**
/// （超宽全景与极端竖屏会被 RGA 拉伸），但 RGA 每帧写入量与差分开销因此恒定；
/// 门控只判定“画面是否变化”，不定位目标，拉伸带来的几何误差可忽略。
///
/// 返回 `None` 表示该源尺寸无法生成合法缩略图（源小于目标，拒绝放大）。
pub fn thumbnail_target(src_w: u32, src_h: u32) -> Option<(u32, u32)> {
    if src_w == 0 || src_h == 0 {
        return None;
    }

    let height = THUMBNAIL_HEIGHT.min(src_h) & !1;
    if height == 0 {
        return None;
    }

    let scaled = (src_w as u64 * height as u64 / src_h as u64) as u32;
    let width = scaled.clamp(THUMBNAIL_MIN_WIDTH, THUMBNAIL_MAX_WIDTH) & !3;
    if width == 0 || width > src_w || height > src_h {
        return None;
    }

    Some((width, height))
}

/// 运动检测与门控评估结果
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionGateDecision {
    /// 是否跳过本次推理 (true: 静止跳过; false: 有活动或保活放行)
    pub should_skip: bool,
    /// 当前帧归一化运动热度 (0.0 ~ 1.0)
    pub motion_score: f32,
    /// 本次放行是否由保活心跳触发
    pub is_keepalive: bool,
}

impl MotionGateDecision {
    /// 无法判定时的保守放行决策（热度按“有活动”上报，与放行动作保持一致）
    #[inline]
    pub const fn passthrough() -> Self {
        Self {
            should_skip: false,
            motion_score: 1.0,
            is_keepalive: false,
        }
    }
}

/// 工业级运动门控引擎
#[derive(Debug)]
pub struct MotionGate {
    pub config: MotionGateConfig,
    last_infer_time_ms: i64,
    motion_hold_counter: u32,
    reference_frame: Option<Vec<u8>>,
    reference_width: usize,
    reference_height: usize,
    mask_bitmap: Option<MaskBitmap>,
    rules: Vec<DetectionRule>,
    cached_width: usize,
    cached_height: usize,
    scratch_y: Vec<u8>,
    /// 门控缩略图缩放器（DMA-BUF 硬解帧的 CPU 可读视图，首次用到时建立并常驻）
    #[cfg(all(target_os = "linux", feature = "rga"))]
    thumb_scaler: Option<MotionThumbnailScaler>,
    /// 缩略图链路连续失败次数（成功一帧即清零）
    #[cfg(all(target_os = "linux", feature = "rga"))]
    thumb_fail_streak: u32,
    /// 缩略图链路已熔断：不再逐帧重试硬件，一律保守放行
    #[cfg(all(target_os = "linux", feature = "rga"))]
    thumb_link_down: bool,
    /// 防区死区告警是否已发出（每路只告警一次）
    mask_dead_zone_warned: bool,
    /// 遥测：绕过计数、按类首次告警与限速日志（由门控线程独占）
    telemetry: GateTelemetry,
}

impl MotionGate {
    pub fn new(config: MotionGateConfig) -> Self {
        Self {
            config,
            last_infer_time_ms: 0,
            motion_hold_counter: 0,
            reference_frame: None,
            reference_width: 0,
            reference_height: 0,
            mask_bitmap: None,
            rules: Vec::new(),
            cached_width: 0,
            cached_height: 0,
            scratch_y: Vec::new(),
            #[cfg(all(target_os = "linux", feature = "rga"))]
            thumb_scaler: None,
            #[cfg(all(target_os = "linux", feature = "rga"))]
            thumb_fail_streak: 0,
            #[cfg(all(target_os = "linux", feature = "rga"))]
            thumb_link_down: false,
            mask_dead_zone_warned: false,
            telemetry: GateTelemetry::default(),
        }
    }

    /// 绑定相机标识（影响日志归属，不参与判定）
    pub fn with_camera_id(mut self, camera_id: impl Into<String>) -> Self {
        self.telemetry = GateTelemetry::with_camera_id(camera_id);
        self
    }

    /// 已累计的“未参与门控判定”帧数（载体不支持或缩略图链路失败）
    #[inline]
    pub fn bypassed_frames(&self) -> u64 {
        self.telemetry.bypassed_frames()
    }

    /// 更新空间遮罩规则；Mask / ROI 位图在下一帧评估时按**评估栅格**惰性光栅化
    ///
    /// 不接收源分辨率：硬件门控在设备侧缩略图上差分，若在源分辨率上预光栅化，
    /// 既白白做一次全分辨率光栅化，又会让 Mask/ROI 与差分栅格错位。
    pub fn update_rules(&mut self, rules: &[DetectionRule]) {
        self.rules = rules.to_vec();
        self.mask_bitmap = None;
        self.cached_width = 0;
        self.cached_height = 0;
        self.mask_dead_zone_warned = false;
    }

    fn ensure_mask_dimensions(&mut self, width: usize, height: usize) {
        if self.cached_width == width && self.cached_height == height {
            return;
        }

        let bitmap = if self.rules.is_empty() {
            None
        } else {
            Some(MaskBitmap::new(&self.rules, width, height))
        };

        // 防区覆盖数低于 `contour_area` 时，有效变动像素数永远达不到阈值：
        // 该路只靠保活心跳放行，业务上等价于门控失效。这是可推导的死区，必须告警而不是静默跳过。
        if let Some(bitmap) = bitmap.as_ref() {
            if bitmap.declared_roi && !self.mask_dead_zone_warned {
                let roi_pixels = bitmap.roi_pixels();
                if roi_pixels < self.config.contour_area as usize {
                    self.mask_dead_zone_warned = true;
                    warn!(
                        camera = %self.telemetry.camera_id(),
                        roi_pixels,
                        contour_area = self.config.contour_area,
                        grid_width = width,
                        grid_height = height,
                        "防区在运动门控栅格上的覆盖像素数低于 contour_area，本路运动将永远无法触发；请扩大防区或在源分辨率上标定"
                    );
                }
            }
        }

        self.mask_bitmap = bitmap;
        self.cached_width = width;
        self.cached_height = height;
    }

    /// 核心评估函数：针对当前帧的亮度 Y 平面切片计算运动差分与跳帧决策。
    ///
    /// # 核心算法：
    /// 1. 保活周期检查：超过 `keepalive_interval_ms` 强制放行一帧刷新跟踪器状态；
    /// 2. $8\times 8$ 宏块网格差分聚类：过滤飞虫、高频 Sensor 散粒噪点；
    /// 3. 空间规则掩模抑制：忽略配置了 Mask 的区域，非 ROI 防区变动不触发；
    /// 4. 场景瞬态抑制：全图变动超过 85%（如昼夜 IRCUT 切换）重置背景，避免长时误报警；
    /// 5. 余晖保活平滑：检测到动作后维持 `motion_hold_frames` 帧连续放行，防止迈步停顿造成航迹断裂。
    pub fn evaluate(
        &mut self,
        current_y: &[u8],
        width: usize,
        height: usize,
        timestamp_ms: i64,
    ) -> MotionGateDecision {
        if !self.config.enabled {
            return MotionGateDecision {
                should_skip: false,
                motion_score: 0.0,
                is_keepalive: false,
            };
        }

        let Some(total_pixels) = width.checked_mul(height) else {
            return MotionGateDecision {
                should_skip: false,
                motion_score: 0.0,
                is_keepalive: false,
            };
        };
        if current_y.len() < total_pixels || total_pixels == 0 {
            // 异常输入尺寸，保守放行
            return MotionGateDecision {
                should_skip: false,
                motion_score: 0.0,
                is_keepalive: false,
            };
        }

        self.ensure_mask_dimensions(width, height);

        // 首帧建立参考背景
        let Some(ref_frame) = self.reference_frame.as_mut() else {
            self.reference_frame = Some(current_y[..total_pixels].to_vec());
            self.reference_width = width;
            self.reference_height = height;
            self.last_infer_time_ms = timestamp_ms;
            self.cached_width = width;
            self.cached_height = height;
            let decision = MotionGateDecision {
                should_skip: false,
                motion_score: 0.0,
                is_keepalive: true,
            };
            self.telemetry.log_decision(
                timestamp_ms,
                DecisionLog {
                    grid_width: width,
                    grid_height: height,
                    active_pixels: 0,
                    total_diff_pixels: 0,
                    decision,
                },
            );
            return decision;
        };

        // 若分辨率发生变化，重置参考帧
        if ref_frame.len() != total_pixels
            || self.reference_width != width
            || self.reference_height != height
        {
            *ref_frame = current_y[..total_pixels].to_vec();
            self.reference_width = width;
            self.reference_height = height;
            self.motion_hold_counter = 0;
            self.last_infer_time_ms = timestamp_ms;
            self.cached_width = width;
            self.cached_height = height;
            let decision = MotionGateDecision {
                should_skip: false,
                motion_score: 0.0,
                is_keepalive: true,
            };
            self.telemetry.log_decision(
                timestamp_ms,
                DecisionLog {
                    grid_width: width,
                    grid_height: height,
                    active_pixels: 0,
                    total_diff_pixels: 0,
                    decision,
                },
            );
            return decision;
        }

        // 1. 保活周期判定 (Keepalive)
        let is_keepalive = timestamp_ms.saturating_sub(self.last_infer_time_ms)
            >= self.config.keepalive_interval_ms as i64;

        // 2. 网格化差分聚类计算
        let threshold = self.config.threshold;
        let grid_w = width.div_ceil(CELL_SIZE);
        let grid_h = height.div_ceil(CELL_SIZE);

        let mut total_diff_pixels: u32 = 0;
        let mut active_cluster_pixels: u32 = 0;

        // 规则位图必须与差分栅格同尺寸：不匹配时 `slices_for` 退化为“无规则”（保守放行语义）
        let (mask_slice, roi_slice, declared_roi) = self
            .mask_bitmap
            .as_ref()
            .map_or((None, None, false), |bitmap| {
                bitmap.slices_for(width, height)
            });

        for gy in 0..grid_h {
            let y_start = gy * CELL_SIZE;
            let y_end = (y_start + CELL_SIZE).min(height);

            for gx in 0..grid_w {
                let x_start = gx * CELL_SIZE;
                let x_end = (x_start + CELL_SIZE).min(width);

                let mut cell_diff_count: u32 = 0;

                for y in y_start..y_end {
                    let row_offset = y * width;
                    for x in x_start..x_end {
                        let idx = row_offset + x;
                        let cur_val = current_y[idx];
                        let ref_val = ref_frame[idx];

                        let diff = cur_val.abs_diff(ref_val);
                        if diff >= threshold {
                            total_diff_pixels += 1;

                            // Mask 与 ROI 都是像素级有效性约束；直接通过线性索引查表，
                            // 消除重复的坐标转换乘法、bounds check 与 Option 闭包开销。
                            let is_masked = mask_slice.is_some_and(|m| m[idx] != 0);
                            let is_in_roi = !declared_roi || roi_slice.is_none_or(|r| r[idx] != 0);
                            if !is_masked && is_in_roi {
                                cell_diff_count += 1;
                            }
                        }
                    }
                }

                // 宏块网格变动超过 25% (16 像素) 且处于 ROI 防区内，判定为有效活跃宏块
                if cell_diff_count >= CELL_ACTIVE_MIN_PIXELS {
                    active_cluster_pixels += cell_diff_count;
                }
            }
        }

        // 3. 场景瞬态剧变抑制 (Scene Change / Light Shock)
        // 若全图变动面积超过 85% (如昼夜红外滤光片切换 IRCUT 或开灯瞬间)，重置参考帧并触发单帧保活
        if total_diff_pixels as f32 > (total_pixels as f32 * SCENE_CHANGE_RATIO) {
            ref_frame.copy_from_slice(&current_y[..total_pixels]);
            self.motion_hold_counter = 0;
            self.last_infer_time_ms = timestamp_ms;
            let decision = MotionGateDecision {
                should_skip: false,
                motion_score: 1.0,
                is_keepalive: true,
            };
            self.telemetry.log_decision(
                timestamp_ms,
                DecisionLog {
                    grid_width: width,
                    grid_height: height,
                    active_pixels: active_cluster_pixels,
                    total_diff_pixels,
                    decision,
                },
            );
            return decision;
        }

        // 4. 原始运动判定与热度归一化
        let has_raw_motion = active_cluster_pixels >= self.config.contour_area;
        let alpha_area = (total_pixels as f32 * MOTION_SCORE_ALPHA).max(1.0);
        let motion_score = (active_cluster_pixels as f32 / alpha_area).clamp(0.0, 1.0);

        // 5. 动态背景平滑吸收 (Background Smoothing)
        if !has_raw_motion {
            // 画面静止时，按加权移动平均微调参考帧，吸收日出日落等光线慢速渐变 (β ≈ 0.03125)
            for (r, &c) in ref_frame.iter_mut().zip(current_y[..total_pixels].iter()) {
                *r = ((*r as u16 * 31 + c as u16 + 16) >> 5) as u8;
            }
        } else {
            // 有显著运动时，直接将当前活跃帧作为下一帧的前序参考基准
            ref_frame.copy_from_slice(&current_y[..total_pixels]);
        }

        // 6. 决策与余晖保活状态机
        let decision = if has_raw_motion {
            self.motion_hold_counter = self.config.motion_hold_frames;
            self.last_infer_time_ms = timestamp_ms;
            MotionGateDecision {
                should_skip: false,
                motion_score,
                is_keepalive: false,
            }
        } else if self.motion_hold_counter > 0 {
            self.motion_hold_counter -= 1;
            self.last_infer_time_ms = timestamp_ms;
            MotionGateDecision {
                should_skip: false,
                motion_score,
                is_keepalive: false,
            }
        } else if is_keepalive {
            self.last_infer_time_ms = timestamp_ms;
            MotionGateDecision {
                should_skip: false,
                motion_score,
                is_keepalive: true,
            }
        } else {
            MotionGateDecision {
                should_skip: true,
                motion_score,
                is_keepalive: false,
            }
        };

        // 7. 限速遥测：把差分口径（有效差分像素数 / 热度 / 决策）写入日志，
        //    现场无需额外探针即可标定 `threshold` 与 `contour_area`。
        self.telemetry.log_decision(
            timestamp_ms,
            DecisionLog {
                grid_width: width,
                grid_height: height,
                active_pixels: active_cluster_pixels,
                total_diff_pixels,
                decision,
            },
        );

        decision
    }

    /// 兼容旧 API：评估当前帧是否应当跳过
    pub fn should_skip_frame(&mut self, frame: &FrameRef) -> bool {
        self.evaluate_frame(frame, frame.timestamp).should_skip
    }

    /// 兼容旧 API：直接传入时间戳与外部运动布尔值进行保活与跳帧评估
    pub fn should_skip(&mut self, current_time_ms: i64, has_motion: bool) -> bool {
        if !self.config.enabled {
            return false;
        }

        let is_keepalive = current_time_ms.saturating_sub(self.last_infer_time_ms)
            >= self.config.keepalive_interval_ms as i64;

        if is_keepalive {
            self.last_infer_time_ms = current_time_ms;
            false
        } else if has_motion {
            self.motion_hold_counter = self.config.motion_hold_frames;
            self.last_infer_time_ms = current_time_ms;
            false
        } else if self.motion_hold_counter > 0 {
            self.motion_hold_counter -= 1;
            self.last_infer_time_ms = current_time_ms;
            false
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{DetectionPoint, DetectionRuleRole};

    #[test]
    fn test_gate_disabled_never_skips() {
        let mut gate = MotionGate::new(MotionGateConfig {
            enabled: false,
            ..Default::default()
        });

        let img = vec![128u8; 100 * 100];
        let decision = gate.evaluate(&img, 100, 100, 1000);
        assert!(!decision.should_skip);
    }
    #[test]
    fn test_first_frame_is_keepalive() {
        let gate_cfg = MotionGateConfig::default();
        let mut gate = MotionGate::new(gate_cfg);

        let img = vec![128u8; 64 * 64];
        let decision = gate.evaluate(&img, 64, 64, 1000);
        assert!(!decision.should_skip);
        assert!(decision.is_keepalive);
    }
    #[test]
    fn test_thumbnail_target_keeps_fixed_grid_and_never_upscales() {
        // 16:9 恒为 320x180：1080p / 1440p / 4K 同一口径，阈值语义与源分辨率解耦
        assert_eq!(thumbnail_target(1920, 1080), Some((320, 180)));
        assert_eq!(thumbnail_target(2560, 1440), Some((320, 180)));
        assert_eq!(thumbnail_target(3840, 2160), Some((320, 180)));
        assert_eq!(thumbnail_target(640, 360), Some((320, 180)));
        // 4:3 与竖屏按同高换算，宽度 4 像素对齐
        assert_eq!(thumbnail_target(640, 480), Some((240, 180)));
        assert_eq!(thumbnail_target(1080, 1920), Some((100, 180)));
        // 源本身已足够小：保持 1:1，绝不放大
        assert_eq!(thumbnail_target(320, 180), Some((320, 180)));
        assert_eq!(thumbnail_target(160, 90), Some((160, 90)));
        // 宽度夹紧到 640 上限：超宽全景接受几何拉伸（写入量与差分开销恒定）
        assert_eq!(thumbnail_target(3840, 960), Some((640, 180)));
        // 源比目标还窄：拒绝生成缩略图
        assert_eq!(thumbnail_target(32, 240), None);
        assert_eq!(thumbnail_target(0, 1080), None);
    }
    #[test]
    fn test_passthrough_decision_is_conservative() {
        let decision = MotionGateDecision::passthrough();
        assert!(!decision.should_skip, "无法判定时必须保守放行");
        assert!(!decision.is_keepalive);
        assert_eq!(decision.motion_score, 1.0);
    }
    #[test]
    fn test_roi_below_contour_area_is_flagged_as_dead_zone() {
        let tiny_roi = DetectionRule {
            role: DetectionRuleRole::Roi,
            line_direction: types::DetectionLineDirection::Both,
            points: vec![
                DetectionPoint { x: 0.5, y: 0.5 },
                DetectionPoint { x: 0.5625, y: 0.5 },
                DetectionPoint {
                    x: 0.5625,
                    y: 0.5625,
                },
                DetectionPoint { x: 0.5, y: 0.5625 },
            ],
        };

        // 4x4=16 像素防区 vs contour_area=500：有效变动像素数永远达不到阈值，
        // 该路除保活外永不推理，必须留下可观测记录
        let mut dead = MotionGate::new(MotionGateConfig {
            threshold: 25,
            contour_area: 500,
            motion_hold_frames: 0,
            ..Default::default()
        });
        dead.update_rules(std::slice::from_ref(&tiny_roi));
        let img = vec![100u8; 64 * 64];
        dead.evaluate(&img, 64, 64, 1000);
        assert!(dead.mask_dead_zone_warned, "死区防区必须被标记");

        // 防区足够大时不告警
        let mut healthy = MotionGate::new(MotionGateConfig {
            threshold: 25,
            contour_area: 16,
            motion_hold_frames: 0,
            ..Default::default()
        });
        healthy.update_rules(&[tiny_roi]);
        healthy.evaluate(&img, 64, 64, 1000);
        assert!(!healthy.mask_dead_zone_warned);
    }
    #[test]
    fn test_rules_rasterize_at_evaluation_grid() {
        // 规则不绑定源分辨率：改评估栅格后按新栅格重新光栅化，不会沿用旧位图
        let mut gate = MotionGate::new(MotionGateConfig {
            threshold: 25,
            contour_area: 16,
            motion_hold_frames: 0,
            ..Default::default()
        });
        let half_mask = DetectionRule {
            role: DetectionRuleRole::Mask,
            line_direction: types::DetectionLineDirection::Both,
            points: vec![
                DetectionPoint { x: 0.0, y: 0.0 },
                DetectionPoint { x: 0.5, y: 0.0 },
                DetectionPoint { x: 0.5, y: 0.5 },
                DetectionPoint { x: 0.0, y: 0.5 },
            ],
        };
        gate.update_rules(std::slice::from_ref(&half_mask));

        // 64x64 栅格：左上 32x32 被遮罩，遮罩区内运动必须被抑制
        let mut img = vec![100u8; 64 * 64];
        assert!(
            !gate.evaluate(&img, 64, 64, 1000).should_skip,
            "首帧保活放行"
        );
        for y in 8..24 {
            for x in 8..24 {
                img[y * 64 + x] = 200;
            }
        }
        assert!(
            gate.evaluate(&img, 64, 64, 1040).should_skip,
            "遮罩区内运动必须被抑制"
        );

        // 32x32 栅格：同一份规则必须按新栅格重新光栅化（左上 16x16）
        let mut small = vec![100u8; 32 * 32];
        assert!(
            !gate.evaluate(&small, 32, 32, 1080).should_skip,
            "栅格切换视为重建基准"
        );
        for y in 4..12 {
            for x in 4..12 {
                small[y * 32 + x] = 200;
            }
        }
        assert!(
            gate.evaluate(&small, 32, 32, 1120).should_skip,
            "栅格切换后 Mask 必须按新栅格重新光栅化并继续生效"
        );
    }
    #[test]
    fn test_static_scene_skips_and_absorbs_background() {
        let mut gate = MotionGate::new(MotionGateConfig::default());
        let img = vec![100u8; 64 * 64];

        // Frame 1: baseline
        gate.evaluate(&img, 64, 64, 1000);

        // Frame 2: identical static image at +40ms
        let decision = gate.evaluate(&img, 64, 64, 1040);
        assert!(decision.should_skip);
        assert_eq!(decision.motion_score, 0.0);
    }
    #[test]
    fn test_noise_resilience_not_triggered() {
        let mut gate = MotionGate::new(MotionGateConfig {
            threshold: 25,
            contour_area: 100,
            ..Default::default()
        });
        let img = vec![100u8; 64 * 64];
        gate.evaluate(&img, 64, 64, 1000);

        // Inject ±5 noise across the entire image (sensor thermal noise)
        let mut noisy_img = img.clone();
        for (i, p) in noisy_img.iter_mut().enumerate() {
            if i % 2 == 0 {
                *p = 104;
            } else {
                *p = 96;
            }
        }

        let decision = gate.evaluate(&noisy_img, 64, 64, 1040);
        assert!(
            decision.should_skip,
            "Sensor thermal noise (±4) must not trigger motion gate"
        );
        assert_eq!(decision.motion_score, 0.0);
    }
    #[test]
    fn test_motion_triggers_and_hold_frames() {
        let mut gate = MotionGate::new(MotionGateConfig {
            threshold: 25,
            contour_area: 100,
            motion_hold_frames: 3,
            keepalive_interval_ms: 5000,
            ..Default::default()
        });

        let mut img = vec![100u8; 64 * 64];
        gate.evaluate(&img, 64, 64, 1000);

        // Create a moving block of 20x20 pixels (400 pixels > contour_area 100)
        // that spans across multiple 8x8 cells
        for y in 10..30 {
            for x in 10..30 {
                img[y * 64 + x] = 200; // diff = 100 > threshold 25
            }
        }

        // Frame 2: motion triggers!
        let d2 = gate.evaluate(&img, 64, 64, 1040);
        assert!(!d2.should_skip);
        assert!(d2.motion_score > 0.0);

        // Frame 3: image stays the same (relative to Frame 2, diff = 0), but motion_hold_frames (3) active!
        let d3 = gate.evaluate(&img, 64, 64, 1080);
        assert!(!d3.should_skip, "Hold frame 1 must be let through");

        let d4 = gate.evaluate(&img, 64, 64, 1120);
        assert!(!d4.should_skip, "Hold frame 2 must be let through");

        let d5 = gate.evaluate(&img, 64, 64, 1160);
        assert!(!d5.should_skip, "Hold frame 3 must be let through");

        // Frame 6: hold expires, should skip!
        let d6 = gate.evaluate(&img, 64, 64, 1200);
        assert!(d6.should_skip, "After hold expires, static frame must skip");
    }
    #[test]
    fn test_keepalive_triggers() {
        let mut gate = MotionGate::new(MotionGateConfig {
            keepalive_interval_ms: 1000,
            ..Default::default()
        });

        let img = vec![100u8; 64 * 64];
        gate.evaluate(&img, 64, 64, 1000);

        // At +500ms, static -> skip
        let d2 = gate.evaluate(&img, 64, 64, 1500);
        assert!(d2.should_skip);

        // At +1050ms, keepalive period reached -> let through!
        let d3 = gate.evaluate(&img, 64, 64, 2050);
        assert!(!d3.should_skip);
        assert!(d3.is_keepalive);
    }
    #[test]
    fn test_scene_change_suppression() {
        let mut gate = MotionGate::new(MotionGateConfig::default());
        let img1 = vec![50u8; 64 * 64];
        gate.evaluate(&img1, 64, 64, 1000);

        // 90% of screen changes drastically (e.g. IRCUT daylight switch)
        let img2 = vec![220u8; 64 * 64];
        let d2 = gate.evaluate(&img2, 64, 64, 1040);
        assert!(!d2.should_skip);
        assert!(
            d2.is_keepalive,
            "Scene change should trigger a single keepalive frame and reset background"
        );

        // Next frame identical -> skips immediately without false multi-frame alarms!
        let d3 = gate.evaluate(&img2, 64, 64, 1080);
        assert!(d3.should_skip);
    }
    #[test]
    fn test_mask_filtering() {
        let mut gate = MotionGate::new(MotionGateConfig {
            threshold: 25,
            contour_area: 50,
            motion_hold_frames: 0,
            ..Default::default()
        });

        let mut img = vec![100u8; 64 * 64];
        gate.evaluate(&img, 64, 64, 1000);

        // Add a Mask rule covering the top-left region [0.0, 0.0] -> [0.5, 0.5] (32x32 pixels)
        let mask_rule = DetectionRule {
            role: DetectionRuleRole::Mask,
            line_direction: types::DetectionLineDirection::Both,
            points: vec![
                DetectionPoint { x: 0.0, y: 0.0 },
                DetectionPoint { x: 0.5, y: 0.0 },
                DetectionPoint { x: 0.5, y: 0.5 },
                DetectionPoint { x: 0.0, y: 0.5 },
            ],
        };
        gate.update_rules(&[mask_rule]);

        // Cause motion ONLY inside the masked region [10..26, 10..26] (16x16 = 256 pixels)
        for y in 10..26 {
            for x in 10..26 {
                img[y * 64 + x] = 200;
            }
        }

        let d2 = gate.evaluate(&img, 64, 64, 1040);
        assert!(
            d2.should_skip,
            "Motion occurring entirely within the Mask polygon must be suppressed"
        );
        assert_eq!(d2.motion_score, 0.0);

        // Now cause motion OUTSIDE the masked region (e.g. x in [35..55], y in [35..55])
        for y in 35..55 {
            for x in 35..55 {
                img[y * 64 + x] = 200;
            }
        }

        let d3 = gate.evaluate(&img, 64, 64, 1080);
        assert!(
            !d3.should_skip,
            "Motion outside the mask must trigger motion gate"
        );
        assert!(d3.motion_score > 0.0);
    }
    #[test]
    fn test_roi_filtering() {
        let mut gate = MotionGate::new(MotionGateConfig {
            threshold: 25,
            contour_area: 50,
            motion_hold_frames: 0,
            ..Default::default()
        });

        let mut img = vec![100u8; 64 * 64];
        gate.evaluate(&img, 64, 64, 1000);

        // Define an ROI covering [0.5, 0.5] -> [1.0, 1.0] (bottom-right)
        let roi_rule = DetectionRule {
            role: DetectionRuleRole::Roi,
            line_direction: types::DetectionLineDirection::Both,
            points: vec![
                DetectionPoint { x: 0.5, y: 0.5 },
                DetectionPoint { x: 1.0, y: 0.5 },
                DetectionPoint { x: 1.0, y: 1.0 },
                DetectionPoint { x: 0.5, y: 1.0 },
            ],
        };
        gate.update_rules(&[roi_rule]);

        // Motion outside ROI (top-left [10..26, 10..26]) -> should be suppressed by ROI gate
        for y in 10..26 {
            for x in 10..26 {
                img[y * 64 + x] = 200;
            }
        }

        let d2 = gate.evaluate(&img, 64, 64, 1040);
        assert!(
            d2.should_skip,
            "Motion outside ROI must not trigger motion gate"
        );
        assert_eq!(d2.motion_score, 0.0);

        // Motion inside ROI ([40..56, 40..56]) -> should trigger!
        for y in 40..56 {
            for x in 40..56 {
                img[y * 64 + x] = 200;
            }
        }

        let d3 = gate.evaluate(&img, 64, 64, 1080);
        assert!(
            !d3.should_skip,
            "Motion inside ROI must trigger motion gate"
        );
        assert!(d3.motion_score > 0.0);
    }
    #[test]
    fn test_backward_compat_should_skip() {
        let mut gate = MotionGate::new(MotionGateConfig {
            keepalive_interval_ms: 2000,
            motion_hold_frames: 2,
            ..Default::default()
        });

        // Frame 1: motion detected
        assert!(!gate.should_skip(1000, true));
        // Frame 2: no motion, hold frame 1
        assert!(!gate.should_skip(1040, false));
        // Frame 3: no motion, hold frame 2
        assert!(!gate.should_skip(1080, false));
        // Frame 4: no motion, hold expired -> skip!
        assert!(gate.should_skip(1120, false));
        // Frame 5: keepalive reached (+2000ms from last inference at 1080ms)
        assert!(!gate.should_skip(3100, false));
    }
}
