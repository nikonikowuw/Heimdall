//! INT8 量化与反量化工具函数（适用于任何 RKNN / ONNX 量化模型）

/// INT8 反量化：`val * scale + zp` 的逆运算
#[inline(always)]
pub fn dequant_i8(val: i8, zp: i32, scale: f32) -> f32 {
    if !scale.is_finite() || scale <= 0.0 {
        return 0.0;
    }
    (val as i32 - zp) as f32 * scale
}

/// INT8 量化：将 f32 值映射到 INT8 量化空间
#[inline(always)]
pub fn quant_f32(val: f32, zp: i32, scale: f32) -> i8 {
    if !scale.is_finite() || scale <= 0.0 || !val.is_finite() {
        return zp.clamp(-128, 127) as i8;
    }
    let v = val / scale + zp as f32;
    if !v.is_finite() {
        return zp.clamp(-128, 127) as i8;
    }
    v.clamp(-128.0, 127.0).round() as i8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let q = quant_f32(0.5, 0, 0.01);
        let dq = dequant_i8(q, 0, 0.01);
        assert!((dq - 0.5).abs() < 1e-5);
    }

    #[test]
    fn test_dequant_zero_scale_returns_zero() {
        assert_eq!(dequant_i8(10, 0, 0.0), 0.0);
    }

    #[test]
    fn test_dequant_nan_scale_returns_zero() {
        assert_eq!(dequant_i8(10, 0, f32::NAN), 0.0);
    }

    #[test]
    fn test_quant_zero_scale_returns_zp() {
        assert_eq!(quant_f32(0.5, 0, 0.0), 0);
    }

    #[test]
    fn test_quant_negative_scale_returns_zp() {
        assert_eq!(quant_f32(0.5, 0, -1.0), 0);
    }

    #[test]
    fn test_quant_nan_input_returns_zp() {
        assert_eq!(quant_f32(f32::NAN, 0, 0.1), 0);
    }
}
