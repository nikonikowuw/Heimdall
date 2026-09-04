use url::Url;

/// 子码流推导候选
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubStreamCandidate {
    pub brand: String,
    pub sub_url: String,
    pub description: String,
}

/// 根据主流摄像机 RTSP URL 规则自动推导子码流候选地址
pub fn deduce_sub_stream(main_url: &str) -> Vec<SubStreamCandidate> {
    if main_url.trim().is_empty() {
        return Vec::new();
    }

    let Ok(u) = Url::parse(main_url) else {
        return Vec::new();
    };

    let path = u.path();
    let raw_query = u.query().unwrap_or("");
    let mut candidates = Vec::new();

    // 1. 海康威视 / 天地伟业 (Hikvision / Tiandy / ISAPI)
    // 主码流通常为 /Streaming/Channels/101 或 /h264/ch1/main/av_stream 或 /ch1/main/av_stream
    if path.contains("/Streaming/Channels/") {
        // 匹配 .../Streaming/Channels/101 -> 102, 201 -> 202
        if let Some(pos) = path.find("/Streaming/Channels/") {
            let suffix = &path[pos + "/Streaming/Channels/".len()..];
            if suffix.len() >= 3 && suffix.ends_with("01") {
                let channel_prefix = &suffix[..suffix.len() - 2];
                let sub_path = format!("{}/Streaming/Channels/{channel_prefix}02", &path[..pos]);
                let mut sub_u = u.clone();
                sub_u.set_path(&sub_path);
                candidates.push(SubStreamCandidate {
                    brand: "Hikvision".to_string(),
                    sub_url: sub_u.to_string(),
                    description: "海康威视标准子码流 (Channel 102)".to_string(),
                });
            }
        }
    } else if path.contains("/main/av_stream") {
        let sub_path = path.replacen("/main/av_stream", "/sub/av_stream", 1);
        let mut sub_u = u.clone();
        sub_u.set_path(&sub_path);
        let brand = if path.contains("/ch1/main/av_stream") && !path.contains("/h264/") {
            "Tiandy/Hikvision"
        } else {
            "Hikvision"
        };
        candidates.push(SubStreamCandidate {
            brand: brand.to_string(),
            sub_url: sub_u.to_string(),
            description: "海康威视/天地伟业子码流 (/sub/av_stream)".to_string(),
        });
    }

    // 2. 大华 (Dahua)
    // 主码流通常为 /cam/realmonitor?channel=1&subtype=0
    if path.contains("/cam/realmonitor") || raw_query.contains("subtype=0") {
        let mut sub_u = u.clone();
        if raw_query.contains("subtype=0") {
            let sub_query = raw_query.replacen("subtype=0", "subtype=1", 1);
            sub_u.set_query(Some(&sub_query));
        } else if !raw_query.contains("subtype=") {
            let sub_query = if raw_query.is_empty() {
                "subtype=1".to_string()
            } else {
                format!("{raw_query}&subtype=1")
            };
            sub_u.set_query(Some(&sub_query));
        }
        candidates.push(SubStreamCandidate {
            brand: "Dahua".to_string(),
            sub_url: sub_u.to_string(),
            description: "大华标准子码流 (subtype=1)".to_string(),
        });
    }

    // 3. 宇视 (Uniview)
    // 主码流通常为 /video1 或 /unicast/c1/s0/live
    if path.contains("/video1") {
        let sub_path = path.replacen("/video1", "/video2", 1);
        let mut sub_u = u.clone();
        sub_u.set_path(&sub_path);
        candidates.push(SubStreamCandidate {
            brand: "Uniview".to_string(),
            sub_url: sub_u.to_string(),
            description: "宇视标准子码流 (/video2)".to_string(),
        });
    } else if path.contains("/s0/live") {
        let sub_path = path.replacen("/s0/live", "/s1/live", 1);
        let mut sub_u = u.clone();
        sub_u.set_path(&sub_path);
        candidates.push(SubStreamCandidate {
            brand: "Uniview".to_string(),
            sub_url: sub_u.to_string(),
            description: "宇视标准子码流 (/s1/live)".to_string(),
        });
    }

    // 4. TP-Link / 水星 (Mercury)
    // 主码流通常为 /stream1
    if path.contains("/stream1") {
        let sub_path = path.replacen("/stream1", "/stream2", 1);
        let mut sub_u = u.clone();
        sub_u.set_path(&sub_path);
        candidates.push(SubStreamCandidate {
            brand: "TP-Link".to_string(),
            sub_url: sub_u.to_string(),
            description: "TP-Link/水星标准子码流 (/stream2)".to_string(),
        });
    }

    // 5. 通用后缀推导 (/main -> /sub, /ch0 -> /ch1, /0 -> /1)
    if candidates.is_empty() {
        if path.contains("/main") {
            let sub_path = path.replacen("/main", "/sub", 1);
            let mut sub_u = u.clone();
            sub_u.set_path(&sub_path);
            candidates.push(SubStreamCandidate {
                brand: "Generic".to_string(),
                sub_url: sub_u.to_string(),
                description: "通用子码流 (/sub 替换)".to_string(),
            });
        } else if path.ends_with('0') {
            let mut sub_path = path.to_string();
            sub_path.pop();
            sub_path.push('1');
            let mut sub_u = u.clone();
            sub_u.set_path(&sub_path);
            candidates.push(SubStreamCandidate {
                brand: "Generic".to_string(),
                sub_url: sub_u.to_string(),
                description: "通用子码流 (通道 0 -> 1)".to_string(),
            });
        }
    }

    candidates
}

/// 获取第一顺位推荐的子码流地址（若无可推导则返回 None）
pub fn deduce_primary_sub_stream(main_url: &str) -> Option<String> {
    deduce_sub_stream(main_url)
        .into_iter()
        .next()
        .map(|c| c.sub_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deduce_hikvision_channels() {
        let main = "rtsp://admin:12345@192.168.1.64:554/Streaming/Channels/101";
        let list = deduce_sub_stream(main);
        assert!(!list.is_empty());
        assert_eq!(
            list[0].sub_url,
            "rtsp://admin:12345@192.168.1.64:554/Streaming/Channels/102"
        );
    }

    #[test]
    fn test_deduce_hikvision_av_stream() {
        let main = "rtsp://admin:12345@192.168.1.64:554/h264/ch1/main/av_stream";
        let sub = deduce_primary_sub_stream(main);
        assert_eq!(
            sub.as_deref(),
            Some("rtsp://admin:12345@192.168.1.64:554/h264/ch1/sub/av_stream")
        );
    }

    #[test]
    fn test_deduce_dahua() {
        let main = "rtsp://admin:admin123@192.168.1.108:554/cam/realmonitor?channel=1&subtype=0";
        let sub = deduce_primary_sub_stream(main);
        assert_eq!(
            sub.as_deref(),
            Some("rtsp://admin:admin123@192.168.1.108:554/cam/realmonitor?channel=1&subtype=1")
        );
    }

    #[test]
    fn test_deduce_tplink() {
        let main = "rtsp://admin:123456@192.168.1.50:554/stream1";
        let sub = deduce_primary_sub_stream(main);
        assert_eq!(
            sub.as_deref(),
            Some("rtsp://admin:123456@192.168.1.50:554/stream2")
        );
    }

    #[test]
    fn test_deduce_generic() {
        let main = "rtsp://127.0.0.1:8554/live/ch0";
        let sub = deduce_primary_sub_stream(main);
        assert_eq!(sub.as_deref(), Some("rtsp://127.0.0.1:8554/live/ch1"));
    }
}
