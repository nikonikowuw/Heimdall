//! RFC 5227 地址冲突检测 (ACD / ARP Probe) 与免费 ARP (Gratuitous ARP) 宣告

use super::detector::run_command_with_c_locale;
use crate::error::ApiError;

/// 执行 RFC 5227 标准 ARP 探测检测静态 IP 是否冲突
///
/// 返回值：
/// - `Ok(Some(mac))`：检测到冲突，返回冲突主机的 MAC 地址
/// - `Ok(None)`：探测通过，无冲突
/// - `Err(ApiError)`：探测工具执行失败且不可恢复
pub async fn probe_ipv4_conflict(iface: &str, target_ip: &str) -> Result<Option<String>, ApiError> {
    // 检查 arping 是否存在
    let has_arping = tokio::process::Command::new("which")
        .arg("arping")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !has_arping {
        tracing::warn!("系统未发现 arping 工具，跳过 RFC 5227 地址冲突预检");
        return Ok(None);
    }

    // 优先尝试 RFC 5227 DAD 模式：arping -c 3 -w 2 -D -I <iface> <target_ip> (广播 3 次，超时 2s)
    let output = tokio::process::Command::new("arping")
        .args(["-c", "3", "-w", "2", "-D", "-I", iface, target_ip])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .await;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let combined = format!("{stdout}\n{stderr}");

            // 如果 arping 不支持 -D 选项，回退到常规 arping
            if combined.contains("invalid option") || combined.contains("unrecognized option") {
                return fallback_arping(iface, target_ip).await;
            }

            // 在 DAD 模式下，如果收到回复，退出码非 0，且会有应答行
            if !out.status.success() {
                if let Some(mac) = extract_mac_from_arping(&combined) {
                    return Ok(Some(mac));
                }
                // 如果退出码非 0 且包含 Received 1 response 等字样
                if combined.contains("Received 1") || combined.contains("Received 2") {
                    return Ok(Some("未知 MAC (已占用)".to_string()));
                }
            }
            Ok(None)
        }
        Err(e) => {
            tracing::warn!("执行 arping 失败: {e}，跳过冲突检测");
            Ok(None)
        }
    }
}

/// 常规模式探测回退
async fn fallback_arping(iface: &str, target_ip: &str) -> Result<Option<String>, ApiError> {
    let output = tokio::process::Command::new("arping")
        .args(["-c", "3", "-w", "2", "-I", iface, target_ip])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .await;

    if let Ok(out) = output {
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if out.status.success() || text.contains("reply from") || text.contains("Unicast reply") {
            let mac =
                extract_mac_from_arping(&text).unwrap_or_else(|| "未知 MAC (已响应)".to_string());
            return Ok(Some(mac));
        }
    }
    Ok(None)
}

/// 从 arping 输出解析冲突主机的 MAC 地址
fn extract_mac_from_arping(output: &str) -> Option<String> {
    for line in output.lines() {
        for (open, close) in [('[', ']'), ('(', ')')] {
            if let Some(start) = line.find(open) {
                if let Some(end) = line[start + 1..].find(close) {
                    let candidate = &line[start + 1..start + 1 + end];
                    if is_valid_mac(candidate) {
                        return Some(candidate.to_uppercase());
                    }
                }
            }
        }
    }
    None
}

fn is_valid_mac(s: &str) -> bool {
    let delim = if s.contains(':') {
        ':'
    } else if s.contains('-') {
        '-'
    } else {
        return false;
    };
    let mut count = 0;
    for part in s.split(delim) {
        if part.len() != 2 || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
        count += 1;
    }
    count == 6
}

/// 广播免费 ARP (Gratuitous ARP) 强制刷新交换机与网关缓存
pub async fn announce_gratuitous_arp(iface: &str, ip: &str) {
    // 异步后台广播 2 次免费 ARP
    let iface = iface.to_string();
    let ip = ip.to_string();
    tokio::spawn(async move {
        let _ = run_command_with_c_locale("arping", &["-c", "2", "-A", "-I", &iface, &ip]).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_mac() {
        let sample = "Unicast reply from 192.168.1.100 [AA:BB:CC:DD:EE:FF]  0.892ms\nSent 1 probes (1 broadcast(s))";
        assert_eq!(
            extract_mac_from_arping(sample),
            Some("AA:BB:CC:DD:EE:FF".to_string())
        );

        let sample2 = "40 bytes from 192.168.1.1 (00:11:22:33:44:55): index=0 time=1.2 ms";
        assert_eq!(
            extract_mac_from_arping(sample2),
            Some("00:11:22:33:44:55".to_string())
        );
    }
}
