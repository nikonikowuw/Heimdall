//! 安全帽检测后处理工具
//!
//! 基于 `algo-sdk` 通用 YOLOv8 解码器委托解析。

use algo_sdk::cv::types::PreprocessMode;
use algo_sdk::error::AlgoError;
use algo_sdk::math::NormBox;
use algo_sdk::models::yolo::{YoloDecodeContext, YoloDecoder, Yolov8SpecDecoder};
use algo_sdk::runtime::InferenceOutput;

use crate::config::InstanceConfig;
use crate::plugin::SafetyHelmetSpec;

/// 统一入口：解析推理输出
///
/// 解码失败（如模型规格与输出结构不匹配）不是“无目标”：必须将错误上报，
/// 否则底层会退化为静默零检出。
pub fn try_parse_and_unmap_output(
    output: &InferenceOutput<'_>,
    config: &InstanceConfig,
    custom_label: Option<&'static str>,
    mode: &PreprocessMode,
    orig_w: u32,
    orig_h: u32,
) -> Result<Vec<NormBox>, AlgoError> {
    if orig_w == 0 || orig_h == 0 {
        return Ok(Vec::new());
    }

    let decoder = Yolov8SpecDecoder::<SafetyHelmetSpec>::default();
    let decode_ctx = YoloDecodeContext {
        output,
        conf_threshold: config.confidence_threshold,
        iou_threshold: config.iou_threshold,
        mode,
        orig_w,
        orig_h,
    };

    let mut boxes = decoder.decode(&decode_ctx)?;
    if let Some(label) = custom_label {
        for b in &mut boxes {
            b.label = Some(label);
        }
    }
    Ok(boxes)
}

/// 本地评测工具使用的就地调用形式（错误记入日志后返回空结果）
pub fn parse_and_unmap_output(
    output: &InferenceOutput<'_>,
    config: &InstanceConfig,
    custom_label: Option<&'static str>,
    mode: &PreprocessMode,
    orig_w: u32,
    orig_h: u32,
) -> Vec<NormBox> {
    match try_parse_and_unmap_output(output, config, custom_label, mode, orig_w, orig_h) {
        Ok(boxes) => boxes,
        Err(error) => {
            tracing::error!(
                error = %error,
                orig_w,
                orig_h,
                "安全帽检测后处理失败，本帧按无目标处理"
            );
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use algo_sdk::cv::postprocess::RknnTensorOutput;
    use algo_sdk::cv::types::LetterboxLayout;
    use algo_sdk::runtime::InferenceOutput;

    /// 单浮点输出模式的锚点总数
    const TOTAL_ANCHORS: usize = 5040;
    /// 单浮点输出模式的通道数
    const NUM_CHANNELS: usize = 84;

    /// 模型输入尺寸（与 `SafetyHelmetSpec::INPUT_DIM` 一致）
    const MODEL_W: usize = 640;
    const MODEL_H: usize = 384;

    /// 强负 logit 填充值：概率与 logits 两种语义下均远低于任何合理阈值，
    /// 保证只有显式注入的命中网格能成为候选，使断言不受 NMS 行为影响。
    const SILENT_LOGIT: i8 = -100;

    /// 构造与真实模型同形的 6-tensor 输出（P3/P4/P5 × {box, score}）。
    ///
    /// 仅 P3 首个网格注入 `score_raw`（scale=1.0 / zp=0，即原始值直接等于 logit），
    /// 其余全部填 `SILENT_LOGIT`，boxes 置零（DFL 均匀分布 → 稳定非退化框）。
    fn build_six_tensor_output(score_raw: i8) -> [Vec<i8>; 6] {
        let grid_p3 = (MODEL_W / 8) * (MODEL_H / 8);
        let grid_p4 = (MODEL_W / 16) * (MODEL_H / 16);
        let grid_p5 = (MODEL_W / 32) * (MODEL_H / 32);

        let mut score_p3 = vec![SILENT_LOGIT; 2 * grid_p3];
        score_p3[0] = score_raw;

        [
            vec![0i8; 64 * grid_p3],
            score_p3,
            vec![0i8; 64 * grid_p4],
            vec![SILENT_LOGIT; 2 * grid_p4],
            vec![0i8; 64 * grid_p5],
            vec![SILENT_LOGIT; 2 * grid_p5],
        ]
    }

    /// 将 6 个张量包装为与模型声明一致的 `RknnTensorOutput` 列表
    fn six_tensor_branches<'a>(data: &'a [Vec<i8>; 6]) -> Vec<RknnTensorOutput<'a>> {
        let dims = [
            [1, 64, 48, 80],
            [1, 2, 48, 80],
            [1, 64, 24, 40],
            [1, 2, 24, 40],
            [1, 64, 12, 20],
            [1, 2, 12, 20],
        ];
        dims.iter()
            .enumerate()
            .map(|(index, &dims)| RknnTensorOutput {
                index: index as u32,
                dims,
                scale: 1.0,
                zp: 0,
                data: &data[index],
            })
            .collect()
    }

    fn test_mode() -> PreprocessMode {
        PreprocessMode::Letterbox(LetterboxLayout {
            scaled_w: 640,
            scaled_h: 360,
            pad_left: 0,
            pad_top: 12,
            scale: 1.0 / 3.0,
            dst_w: 640,
            dst_h: 384,
        })
    }

    /// 端到端锁定 `SafetyHelmetSpec::CLS_IS_LOGITS = true` 的行为语义。
    ///
    /// 该用例是**判别性**的：命中网格的原始值为 0（scale=1.0），两种语义给出相反结论。
    ///
    /// - logits 语义：`sigmoid(0) = 0.5 > 0.45` → 保留，置信度 0.5；
    /// - 概率语义（即声明被误改为 `false`）：`0.0 < 0.45` → 丢弃，零检出。
    ///
    /// 因此若声明被翻转，本用例必然失败。它经由 `parse_and_unmap_output` 走
    /// `Yolov8SpecDecoder` → `from_spec(S::…)` 的生产路径，而非本地另建配置。
    #[test]
    fn test_logits_semantics_restore_confidence_from_sigmoid() {
        let data = build_six_tensor_output(0);
        let branches = six_tensor_branches(&data);
        let config = InstanceConfig::default();
        let out = InferenceOutput::MultiBranch(branches);

        let boxes = parse_and_unmap_output(&out, &config, None, &test_mode(), 1920, 1080);

        assert_eq!(
            boxes.len(),
            1,
            "logit 0.0 经 sigmoid 还原为 0.5 应通过阈值；若为 0 说明声明被当作概率语义"
        );
        assert!(
            (boxes[0].confidence - 0.5).abs() < 1e-5,
            "置信度必须为 sigmoid(0)=0.5，实际 {}",
            boxes[0].confidence
        );
    }

    /// 近阈值负 logit 必须按 logit 空间判定，而非与概率阈值直接比较。
    ///
    /// 默认阈值 0.45 对应 logit 空间 ln(0.45/0.55) ≈ -0.2007，
    /// 故 logit -1.0（sigmoid ≈ 0.269）应被过滤。
    #[test]
    fn test_negative_logit_below_threshold_is_filtered() {
        let data = build_six_tensor_output(-1);
        let branches = six_tensor_branches(&data);
        let config = InstanceConfig::default();
        let out = InferenceOutput::MultiBranch(branches);

        let boxes = parse_and_unmap_output(&out, &config, None, &test_mode(), 1920, 1080);

        assert!(
            boxes.is_empty(),
            "logit -1.0 (sigmoid 0.269) 低于默认阈值 0.45，应被过滤"
        );
    }

    #[test]
    fn test_single_float_fallback_decoding() {
        let mut net_out = vec![0.0f32; NUM_CHANNELS * TOTAL_ANCHORS];

        // 锚点 10: Hardhat (class 0), 中心 (320, 192), 宽高 (100, 80), 得分 0.95
        let a = 10;
        net_out[a] = 320.0;
        net_out[TOTAL_ANCHORS + a] = 192.0;
        net_out[2 * TOTAL_ANCHORS + a] = 100.0;
        net_out[3 * TOTAL_ANCHORS + a] = 80.0;
        net_out[4 * TOTAL_ANCHORS + a] = 0.95;

        let config = InstanceConfig::default();
        let out = InferenceOutput::SingleFloat(&net_out);
        let boxes = parse_and_unmap_output(&out, &config, None, &test_mode(), 1920, 1080);

        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].class_id, 0);
        assert_eq!(boxes[0].label, Some("Hardhat"));
        assert!((boxes[0].confidence - 0.95).abs() < 1e-5);
    }

    #[test]
    fn test_single_float_two_classes() {
        let mut net_out = vec![0.0f32; NUM_CHANNELS * TOTAL_ANCHORS];

        // 锚点 5: NO-Hardhat (class 1), 置信度 0.87
        let a = 5;
        net_out[a] = 100.0;
        net_out[TOTAL_ANCHORS + a] = 200.0;
        net_out[2 * TOTAL_ANCHORS + a] = 60.0;
        net_out[3 * TOTAL_ANCHORS + a] = 50.0;
        net_out[(4 + 1) * TOTAL_ANCHORS + a] = 0.87;

        let config = InstanceConfig::default();
        let out = InferenceOutput::SingleFloat(&net_out);
        let boxes = parse_and_unmap_output(&out, &config, None, &test_mode(), 1920, 1080);

        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].class_id, 1);
        assert_eq!(boxes[0].label, Some("NO-Hardhat"));
    }

    #[test]
    fn test_custom_label_override() {
        let mut net_out = vec![0.0f32; NUM_CHANNELS * TOTAL_ANCHORS];
        let a = 10;
        net_out[a] = 320.0;
        net_out[TOTAL_ANCHORS + a] = 192.0;
        net_out[2 * TOTAL_ANCHORS + a] = 100.0;
        net_out[3 * TOTAL_ANCHORS + a] = 80.0;
        net_out[4 * TOTAL_ANCHORS + a] = 0.92;

        let config = InstanceConfig {
            custom_alarm_label: Some("安全检查".to_string()),
            ..Default::default()
        };
        let out = InferenceOutput::SingleFloat(&net_out);
        let boxes =
            parse_and_unmap_output(&out, &config, Some("安全检查"), &test_mode(), 1920, 1080);

        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].label, Some("安全检查"));
    }
}
