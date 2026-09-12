//! Distribution Focal Loss (DFL) 解码
//!
//! 将 DFL 的 N-bin softmax 分布还原为单个回归坐标值。
//! YOLOv8 标准使用 16 bins，但本实现支持任意 bin 数量。

/// DFL 解码：将 N-bin softmax 分布还原为单个回归值
///
/// 计算公式：`out = Σ softmax(bin_i) * i`
/// 使用 log-sum-exp 技巧避免数值溢出。
pub fn decode_dfl(slice: &[f32], out: &mut f32) {
    let n = slice.len();
    if n == 0 {
        *out = 0.0;
        return;
    }
    let mut max_v = slice[0];
    for &v in &slice[1..] {
        if v > max_v {
            max_v = v;
        }
    }
    let mut exp_sum = 0.0f32;
    for &val in slice {
        exp_sum += (val - max_v).exp();
    }
    let inv_exp_sum = 1.0 / exp_sum;
    let mut weighted = 0.0f32;
    for (i, &val) in slice.iter().enumerate() {
        weighted += ((val - max_v).exp() * inv_exp_sum) * (i as f32);
    }
    *out = weighted;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uniform_distribution() {
        // 均匀分布时 DFL 应返回中间值 (n-1)/2
        let bins = [1.0f32; 16];
        let mut result = 0.0f32;
        decode_dfl(&bins, &mut result);
        assert!((result - 7.5).abs() < 0.01);
    }

    #[test]
    fn test_peak_at_12() {
        // 峰值分布应返回接近峰值位置
        let mut bins = [0.0f32; 16];
        bins[12] = 10.0;
        let mut result = 0.0f32;
        decode_dfl(&bins, &mut result);
        assert!((result - 12.0).abs() < 0.01);
    }

    #[test]
    fn test_empty_slice() {
        let mut result = -1.0f32;
        decode_dfl(&[], &mut result);
        assert_eq!(result, 0.0);
    }

    #[test]
    fn test_single_bin() {
        let bins = [0.0, 5.0, 0.0];
        let mut result = 0.0f32;
        decode_dfl(&bins, &mut result);
        assert!((result - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_two_equal_bins() {
        let bins = [1.0, 1.0];
        let mut result = 0.0f32;
        decode_dfl(&bins, &mut result);
        assert!((result - 0.5).abs() < 1e-5);
    }
}
