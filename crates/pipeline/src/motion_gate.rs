use types::{
    DetectionRule, DetectionRuleRole, FrameHandle, FrameRef, MotionGateConfig, PixelFormat,
};

/// 网格宏块尺寸 (8x8 像素)
const CELL_SIZE: usize = 8;
/// 宏块激活所需的最小变动像素数 (64 像素中至少 25% 变动，彻底过滤散粒热噪)
const CELL_ACTIVE_MIN_PIXELS: u32 = 16;
/// 瞬时剧变抑制阈值比例 (全图变动超过 85% 判定为场景瞬变，如昼夜 IRCUT 切换或开关灯)
const SCENE_CHANGE_RATIO: f32 = 0.85;
/// 热度计算归一化敏感度系数 (全图有效变动达到 15% 时热度满格 1.0)
const MOTION_SCORE_ALPHA: f32 = 0.15;

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

/// 空间掩模与 ROI 栅格化位图缓存
#[derive(Debug, Clone, Default)]
pub struct MaskBitmap {
    pub width: usize,
    pub height: usize,
    /// 1 表示被遮罩屏蔽，0 表示正常计算
    mask: Vec<u8>,
    /// 是否配置了 ROI 正向防区规则
    has_roi: bool,
    /// 1 表示在 ROI 内部，0 表示在 ROI 外部
    roi: Vec<u8>,
}

impl MaskBitmap {
    /// 根据空间规则创建或更新掩模位图
    pub fn new(rules: &[DetectionRule], width: usize, height: usize) -> Self {
        if width == 0 || height == 0 {
            return Self::default();
        }

        let Some(total_pixels) = width.checked_mul(height) else {
            return Self::default();
        };
        let mut mask = vec![0u8; total_pixels];
        let mut roi = vec![0u8; total_pixels];
        let mut has_roi = false;

        for rule in rules {
            if rule.points.len() < 3 {
                continue;
            }

            // 转换并钳制归一化多边形到绝对像素坐标；非法顶点不参与光栅化。
            let poly_pixels: Vec<(f32, f32)> = rule
                .points
                .iter()
                .filter_map(|pt| {
                    if !pt.x.is_finite() || !pt.y.is_finite() {
                        return None;
                    }
                    Some((
                        pt.x.clamp(0.0, 1.0) as f32 * width as f32,
                        pt.y.clamp(0.0, 1.0) as f32 * height as f32,
                    ))
                })
                .collect();
            if poly_pixels.len() < 3 {
                continue;
            }

            // 计算多边形的外接包围盒 (AABB)，仅在包围盒范围内执行光栅化
            let mut min_x = width as f32;
            let mut max_x = 0.0f32;
            let mut min_y = height as f32;
            let mut max_y = 0.0f32;

            for &(px, py) in &poly_pixels {
                min_x = min_x.min(px);
                max_x = max_x.max(px);
                min_y = min_y.min(py);
                max_y = max_y.max(py);
            }

            let start_x = (min_x.floor() as usize).min(width);
            let end_x = (max_x.ceil() as usize).min(width);
            let start_y = (min_y.floor() as usize).min(height);
            let end_y = (max_y.ceil() as usize).min(height);

            match rule.role {
                DetectionRuleRole::Mask => {
                    for y in start_y..end_y {
                        let row_offset = y * width;
                        let py = y as f32 + 0.5;
                        for x in start_x..end_x {
                            let px = x as f32 + 0.5;
                            if Self::point_in_polygon(px, py, &poly_pixels) {
                                mask[row_offset + x] = 1;
                            }
                        }
                    }
                }
                DetectionRuleRole::Roi => {
                    has_roi = true;
                    for y in start_y..end_y {
                        let row_offset = y * width;
                        let py = y as f32 + 0.5;
                        for x in start_x..end_x {
                            let px = x as f32 + 0.5;
                            if Self::point_in_polygon(px, py, &poly_pixels) {
                                roi[row_offset + x] = 1;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Self {
            width,
            height,
            mask,
            has_roi,
            roi,
        }
    }

    /// 射线交叉法判定点是否在多边形内部
    #[inline]
    fn point_in_polygon(x: f32, y: f32, poly: &[(f32, f32)]) -> bool {
        let mut inside = false;
        let n = poly.len();
        if n < 3 {
            return false;
        }
        let mut j = n - 1;
        for i in 0..n {
            let (xi, yi) = poly[i];
            let (xj, yj) = poly[j];
            let intersect =
                ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi + 1e-6) + xi);
            if intersect {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    /// 检查指定像素点是否被 Mask 屏蔽
    #[inline(always)]
    pub fn is_masked(&self, x: usize, y: usize) -> bool {
        if self.mask.is_empty() || x >= self.width || y >= self.height {
            return false;
        }
        self.mask[y * self.width + x] != 0
    }

    /// 检查指定像素点是否在有效 ROI 内部（若未配置 ROI 则恒视为在有效防区内）
    #[inline(always)]
    pub fn is_in_roi(&self, x: usize, y: usize) -> bool {
        if !self.has_roi {
            return true;
        }
        if self.roi.is_empty() || x >= self.width || y >= self.height {
            return false;
        }
        self.roi[y * self.width + x] != 0
    }
}

#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferLockBaseAddress(pixel_buffer: *mut std::ffi::c_void, lock_flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pixel_buffer: *mut std::ffi::c_void, lock_flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddressOfPlane(
        pixel_buffer: *mut std::ffi::c_void,
        plane_index: usize,
    ) -> *mut std::ffi::c_void;
    fn CVPixelBufferGetBytesPerRowOfPlane(
        pixel_buffer: *mut std::ffi::c_void,
        plane_index: usize,
    ) -> usize;
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
        }
    }

    /// 更新空间遮罩规则，重新光栅化生成 Mask 与 ROI 位图
    pub fn update_rules(&mut self, rules: &[DetectionRule], width: usize, height: usize) {
        self.rules = rules.to_vec();
        self.mask_bitmap = Some(MaskBitmap::new(&self.rules, width, height));
        self.cached_width = width;
        self.cached_height = height;
    }

    fn ensure_mask_dimensions(&mut self, width: usize, height: usize) {
        if self.cached_width == width && self.cached_height == height {
            return;
        }

        self.mask_bitmap = if self.rules.is_empty() {
            None
        } else {
            Some(MaskBitmap::new(&self.rules, width, height))
        };
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
            return MotionGateDecision {
                should_skip: false,
                motion_score: 0.0,
                is_keepalive: true,
            };
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
            return MotionGateDecision {
                should_skip: false,
                motion_score: 0.0,
                is_keepalive: true,
            };
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

        let mask_ref = self.mask_bitmap.as_ref();
        let (mask_slice, roi_slice, has_roi) = match mask_ref {
            Some(m) if m.width == width && m.height == height => (
                (!m.mask.is_empty()).then_some(&m.mask[..]),
                (!m.roi.is_empty()).then_some(&m.roi[..]),
                m.has_roi,
            ),
            _ => (None, None, false),
        };

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

                        let diff = (cur_val as i16 - ref_val as i16).unsigned_abs() as u8;
                        if diff >= threshold {
                            total_diff_pixels += 1;

                            // Mask 与 ROI 都是像素级有效性约束；直接通过线性索引查表，
                            // 消除重复的坐标转换乘法、bounds check 与 Option 闭包开销。
                            let is_masked = match mask_slice {
                                Some(m) => m[idx] != 0,
                                None => false,
                            };
                            let is_in_roi = if has_roi {
                                match roi_slice {
                                    Some(r) => r[idx] != 0,
                                    None => true,
                                }
                            } else {
                                true
                            };
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
            return MotionGateDecision {
                should_skip: false,
                motion_score: 1.0,
                is_keepalive: true,
            };
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
        if has_raw_motion {
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
        }
    }

    /// 针对 `FrameRef` 评估跳帧决策。
    ///
    /// 对于 Host 内存切片帧，零拷贝提取 Y 分量并执行差分；
    /// 对于 macOS ApplePixelBuffer，在 Unified Memory 下零拷贝直接读取 Y 平面；
    /// 对于物理设备帧 (DMA-BUF / DeviceMemory)，在没有硬件微缩器接入时保守放行，严格保护设备直通零拷贝边界。
    pub fn evaluate_frame(&mut self, frame: &FrameRef, timestamp_ms: i64) -> MotionGateDecision {
        if !self.config.enabled {
            return MotionGateDecision {
                should_skip: false,
                motion_score: 0.0,
                is_keepalive: false,
            };
        }

        let width = frame.width as usize;
        let height = frame.height as usize;
        let Some(total_pixels) = width.checked_mul(height) else {
            return MotionGateDecision {
                should_skip: false,
                motion_score: 0.0,
                is_keepalive: false,
            };
        };
        if width == 0 || height == 0 {
            return MotionGateDecision {
                should_skip: false,
                motion_score: 0.0,
                is_keepalive: false,
            };
        }

        match frame.handle() {
            FrameHandle::Host(bytes) => {
                let hor_stride = frame.stride.hor_stride.max(frame.width) as usize;

                match frame.format {
                    PixelFormat::Nv12 | PixelFormat::Yuv420p => {
                        if hor_stride == width {
                            if bytes.len() >= total_pixels {
                                return self.evaluate(
                                    &bytes[..total_pixels],
                                    width,
                                    height,
                                    timestamp_ms,
                                );
                            }
                            return MotionGateDecision {
                                should_skip: false,
                                motion_score: 0.0,
                                is_keepalive: false,
                            };
                        }

                        // 带水平虚宽 (hor_stride > width) 提取连续可见区域至预分配 Scratchpad
                        if self.scratch_y.len() != total_pixels {
                            self.scratch_y.resize(total_pixels, 0);
                        }

                        for y in 0..height {
                            let src_start = y * hor_stride;
                            let src_end = src_start + width;
                            let dst_start = y * width;
                            let dst_end = dst_start + width;
                            if src_end > bytes.len() {
                                return MotionGateDecision {
                                    should_skip: false,
                                    motion_score: 0.0,
                                    is_keepalive: false,
                                };
                            }
                            self.scratch_y[dst_start..dst_end]
                                .copy_from_slice(&bytes[src_start..src_end]);
                        }

                        self.evaluate_scratch(width, height, timestamp_ms)
                    }
                    PixelFormat::Rgb24 | PixelFormat::Bgr24 => {
                        // 3 通道像素帧 (测试/Mock 环境)，快速加权提取灰度 Y ≈ (R + 2G + B) >> 2
                        let required_bytes = total_pixels.saturating_mul(3);
                        if bytes.len() < required_bytes {
                            return MotionGateDecision {
                                should_skip: false,
                                motion_score: 0.0,
                                is_keepalive: false,
                            };
                        }
                        if self.scratch_y.len() != total_pixels {
                            self.scratch_y.resize(total_pixels, 0);
                        }

                        for i in 0..total_pixels {
                            let b0 = bytes[i * 3] as u16;
                            let b1 = bytes[i * 3 + 1] as u16;
                            let b2 = bytes[i * 3 + 2] as u16;
                            self.scratch_y[i] = ((b0 + (b1 << 1) + b2) >> 2) as u8;
                        }

                        self.evaluate_scratch(width, height, timestamp_ms)
                    }
                    _ => MotionGateDecision {
                        should_skip: false,
                        motion_score: 0.0,
                        is_keepalive: false,
                    },
                }
            }
            #[cfg(target_os = "macos")]
            FrameHandle::ApplePixelBuffer { ptr } => {
                let raw_ptr = ptr.as_ptr();
                // SAFETY: 锁定 CVPixelBuffer 基础地址以只读方式访问 (1 = kCVPixelBufferLock_ReadOnly)
                let lock_status = unsafe { CVPixelBufferLockBaseAddress(raw_ptr, 1) };
                if lock_status != 0 {
                    return MotionGateDecision {
                        should_skip: false,
                        motion_score: 1.0,
                        is_keepalive: false,
                    };
                }

                struct BufferUnlockGuard(*mut std::ffi::c_void);
                impl Drop for BufferUnlockGuard {
                    fn drop(&mut self) {
                        // SAFETY: 解锁 CVPixelBuffer 基础地址
                        unsafe {
                            CVPixelBufferUnlockBaseAddress(self.0, 1);
                        }
                    }
                }
                let _unlock_guard = BufferUnlockGuard(raw_ptr);

                // SAFETY: 在锁定生命周期内读取 Y 平面（Plane 0）基址与行步长
                let (y_ptr, hor_stride) = unsafe {
                    let y = CVPixelBufferGetBaseAddressOfPlane(raw_ptr, 0) as *const u8;
                    let ys = CVPixelBufferGetBytesPerRowOfPlane(raw_ptr, 0);
                    (y, ys)
                };

                if y_ptr.is_null() || hor_stride < width {
                    return MotionGateDecision {
                        should_skip: false,
                        motion_score: 1.0,
                        is_keepalive: false,
                    };
                }

                let total_plane_bytes = hor_stride * height;
                // SAFETY: CVPixelBuffer 已锁定且 y_ptr 非空，内存区在 _unlock_guard 作用域内有效
                let plane_slice = unsafe { std::slice::from_raw_parts(y_ptr, total_plane_bytes) };

                if hor_stride == width {
                    return self.evaluate(
                        &plane_slice[..total_pixels],
                        width,
                        height,
                        timestamp_ms,
                    );
                }

                if self.scratch_y.len() != total_pixels {
                    self.scratch_y.resize(total_pixels, 0);
                }

                for y in 0..height {
                    let src_start = y * hor_stride;
                    let src_end = src_start + width;
                    let dst_start = y * width;
                    let dst_end = dst_start + width;
                    if src_end <= plane_slice.len() {
                        self.scratch_y[dst_start..dst_end]
                            .copy_from_slice(&plane_slice[src_start..src_end]);
                    }
                }

                self.evaluate_scratch(width, height, timestamp_ms)
            }
            _ => {
                // Linux DMA-BUF / Ascend DeviceMemory 暂未接入硬件缩略图生成，保守放行推理
                MotionGateDecision {
                    should_skip: false,
                    motion_score: 1.0,
                    is_keepalive: false,
                }
            }
        }
    }

    /// 内部辅助：利用预分配的 `scratch_y` 执行差分评估，并在评估结束后将缓冲安全归还。
    fn evaluate_scratch(
        &mut self,
        width: usize,
        height: usize,
        timestamp_ms: i64,
    ) -> MotionGateDecision {
        let scratch = std::mem::take(&mut self.scratch_y);
        let decision = self.evaluate(&scratch, width, height, timestamp_ms);
        self.scratch_y = scratch;
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
    use types::{DetectionPoint, StrideInfo};

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
        gate.update_rules(&[mask_rule], 64, 64);

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
    fn test_evaluate_frame_with_host_nv12() {
        let mut gate = MotionGate::new(MotionGateConfig::default());
        let width = 64;
        let height = 64;
        let yuv_bytes = vec![128u8; width * height * 3 / 2];

        let frame = FrameRef::new(
            "cam_01".into(),
            1000,
            width as u32,
            height as u32,
            StrideInfo::new(width as u32, height as u32),
            PixelFormat::Nv12,
            FrameHandle::Host(yuv_bytes.into()),
        );

        let d1 = gate.evaluate_frame(&frame, 1000);
        assert!(!d1.should_skip);
        assert!(d1.is_keepalive);

        let d2 = gate.evaluate_frame(&frame, 1040);
        assert!(d2.should_skip);
    }

    #[test]
    fn test_evaluate_frame_with_strided_nv12_ignores_padding() {
        let mut gate = MotionGate::new(MotionGateConfig {
            threshold: 25,
            contour_area: 16,
            motion_hold_frames: 0,
            ..Default::default()
        });
        let width = 8usize;
        let height = 8usize;
        let stride = 16usize;

        let mut first = vec![0u8; stride * height + stride * height / 2];
        for y in 0..height {
            first[y * stride..y * stride + width].fill(100);
        }
        let frame1 = FrameRef::new(
            "cam_stride".into(),
            1000,
            width as u32,
            height as u32,
            StrideInfo::new(stride as u32, height as u32),
            PixelFormat::Nv12,
            FrameHandle::Host(first.into()),
        );
        assert!(!gate.evaluate_frame(&frame1, 1000).should_skip);

        let mut padding_only = vec![0u8; stride * height + stride * height / 2];
        for y in 0..height {
            padding_only[y * stride..y * stride + width].fill(100);
            padding_only[y * stride + width..(y + 1) * stride].fill(255);
        }
        let frame2 = FrameRef::new(
            "cam_stride".into(),
            1040,
            width as u32,
            height as u32,
            StrideInfo::new(stride as u32, height as u32),
            PixelFormat::Nv12,
            FrameHandle::Host(padding_only.into()),
        );
        assert!(
            gate.evaluate_frame(&frame2, 1040).should_skip,
            "stride padding changes must not be treated as motion"
        );

        let mut visible_motion = vec![0u8; stride * height + stride * height / 2];
        for y in 0..height {
            visible_motion[y * stride..y * stride + 4].fill(200);
            visible_motion[y * stride + 4..(y + 1) * stride].fill(100);
        }
        let frame3 = FrameRef::new(
            "cam_stride".into(),
            1080,
            width as u32,
            height as u32,
            StrideInfo::new(stride as u32, height as u32),
            PixelFormat::Nv12,
            FrameHandle::Host(visible_motion.into()),
        );
        assert!(!gate.evaluate_frame(&frame3, 1080).should_skip);
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
        gate.update_rules(&[roi_rule], 64, 64);

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

    #[cfg(target_os = "macos")]
    #[test]
    fn test_evaluate_frame_with_apple_pixel_buffer() {
        extern "C" {
            fn CVPixelBufferCreate(
                allocator: *mut std::ffi::c_void,
                width: usize,
                height: usize,
                pixel_format_type: u32,
                pixel_buffer_attributes: *mut std::ffi::c_void,
                pixel_buffer_out: *mut *mut std::ffi::c_void,
            ) -> i32;
        }

        // kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange = '420v' = 0x34323076
        const PIXEL_FORMAT_NV12: u32 = 0x34323076;

        let width = 64usize;
        let height = 64usize;
        let mut pixel_buffer: *mut std::ffi::c_void = std::ptr::null_mut();
        // SAFETY: 仅用于测试环境中分配一张标准的 64x64 NV12 CoreVideo 内存帧
        let status = unsafe {
            CVPixelBufferCreate(
                std::ptr::null_mut(),
                width,
                height,
                PIXEL_FORMAT_NV12,
                std::ptr::null_mut(),
                &mut pixel_buffer,
            )
        };
        assert_eq!(status, 0, "CVPixelBufferCreate 必须成功");

        let frame = FrameRef::new(
            "cam_mac".into(),
            1000,
            width as u32,
            height as u32,
            StrideInfo::new(width as u32, height as u32),
            PixelFormat::Nv12,
            FrameHandle::ApplePixelBuffer {
                ptr: std::ptr::NonNull::new(pixel_buffer)
                    .expect("CVPixelBuffer 创建成功后指针非空"),
            },
        );

        let mut gate = MotionGate::new(MotionGateConfig::default());
        let d1 = gate.evaluate_frame(&frame, 1000);
        assert!(!d1.should_skip);
        assert!(d1.is_keepalive);

        let d2 = gate.evaluate_frame(&frame, 1040);
        assert!(d2.should_skip, "静止 ApplePixelBuffer 帧应当被门控跳过");
    }
}
