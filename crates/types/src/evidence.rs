//! 证据图片来源标识（跨层契约）。
//!
//! 业务证据三支柱（Alarms / Captures / Recognitions）落库时都必须写清「这张图到底怎么来的」：
//! 只有如此，事后才能分辨一张低清或缺帧的凭据是**取帧降级**还是**质量模型尚未成熟**。
//! 两者正交，因此拆成路径（[`EvidenceImageSource`]）与码流（[`EvidenceImageStream`]）两个维度，
//! 不做笛卡尔积枚举。

use serde::{Deserialize, Serialize};

/// 证据图产生路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceImageSource {
    /// 结算峰值候选帧：分析过程在结算窗口内留存的最优帧，结算时唯一一次落盘。
    ///
    /// 分辨率等于**分析码流**分辨率（双流模式即子码流），这是 M3 主流同 PTS 回溯接入前的已知上限。
    PeakCandidate,
    /// 靶向快拍：结算或告警触发时按 PTS 现场取证（主码流回溯/环复用，失败再回退子码流）。
    Targeted,
}

impl EvidenceImageSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PeakCandidate => "peak_candidate",
            Self::Targeted => "targeted",
        }
    }

    /// 解析持久化取值；未知取值返回 `None`，由消费方按「未标注」降级展示。
    ///
    /// 新增取值必须先同时升级宿主与前端 DTO，不能依赖旧消费者猜测语义。
    pub fn from_wire(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "peak_candidate" => Some(Self::PeakCandidate),
            "targeted" => Some(Self::Targeted),
            _ => None,
        }
    }
}

/// 证据图所属码流。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceImageStream {
    /// 主码流帧（高分辨率）。
    Main,
    /// 子码流帧（低分辨率；双流模式下的峰值候选帧常态，或主码流不可用时的回退帧）。
    Sub,
}

impl EvidenceImageStream {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Sub => "sub",
        }
    }

    pub fn from_wire(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "main" => Some(Self::Main),
            "sub" => Some(Self::Sub),
            _ => None,
        }
    }

    pub fn is_sub(&self) -> bool {
        matches!(self, Self::Sub)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_values_round_trip_and_reject_unknown() {
        for source in [
            EvidenceImageSource::PeakCandidate,
            EvidenceImageSource::Targeted,
        ] {
            assert_eq!(
                EvidenceImageSource::from_wire(source.as_str()),
                Some(source),
                "路径取值必须可往返"
            );
        }
        for stream in [EvidenceImageStream::Main, EvidenceImageStream::Sub] {
            assert_eq!(
                EvidenceImageStream::from_wire(stream.as_str()),
                Some(stream),
                "码流取值必须可往返"
            );
        }
        // 历史记录的空串与未来新增取值都必须降级为「未标注」，不能猜测成语义。
        for unknown in ["", " ", "main_replay", "MAIN-STREAM"] {
            assert_eq!(EvidenceImageSource::from_wire(unknown), None);
            assert_eq!(EvidenceImageStream::from_wire(unknown), None);
        }
        assert!(EvidenceImageStream::Sub.is_sub());
        assert!(!EvidenceImageStream::Main.is_sub());
    }
}
