//! 网络探测器：物理链路载波、协商速率、双工及管理口动态识别

use crate::error::ApiError;

/// 执行外部命令，强制注入 `LC_ALL=C` 和 `LANG=C`，防止本地化语言破坏解析
pub async fn run_command_with_c_locale(cmd: &str, args: &[&str]) -> Result<String, ApiError> {
    let output = tokio::process::Command::new(cmd)
        .args(args)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .await
        .map_err(|e| ApiError::NetworkFailed(format!("执行 {cmd} 失败: {e}")))?;

    if output.status.success() {
        String::from_utf8(output.stdout)
            .map_err(|e| ApiError::NetworkFailed(format!("命令输出编码错误: {e}")))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(ApiError::NetworkFailed(format!("{cmd} 执行失败: {stderr}")))
    }
}

/// 读取物理网线插入载波状态 (Carrier Detect)
pub async fn detect_carrier(iface: &str) -> Option<bool> {
    let path = format!("/sys/class/net/{iface}/carrier");
    tokio::fs::read_to_string(&path)
        .await
        .ok()
        .map(|s| s.trim() == "1")
}

/// 读取网口协商速率 (Mbps) 与双工模式 (full / half)
pub async fn detect_link_speed_and_duplex(iface: &str) -> (Option<u32>, Option<String>) {
    let speed_path = format!("/sys/class/net/{iface}/speed");
    let speed = tokio::fs::read_to_string(&speed_path)
        .await
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());

    let duplex_path = format!("/sys/class/net/{iface}/duplex");
    let duplex = tokio::fs::read_to_string(&duplex_path)
        .await
        .ok()
        .map(|s| s.trim().to_lowercase())
        .filter(|d| d == "full" || d == "half");

    (speed, duplex)
}

/// 读取网卡物理 MAC 地址（大写冒号格式）
pub async fn get_mac_address(iface: &str) -> Result<String, ApiError> {
    let path = format!("/sys/class/net/{iface}/address");
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| ApiError::NetworkFailed(format!("读取 MAC 地址失败: {e}")))?;
    Ok(content.trim().to_uppercase())
}

/// 动态判定网卡是否为当前管理网卡（Management Interface）
///
/// 判定逻辑：
/// 1. 检查网卡是否承载系统主默认路由 (`default via ... dev <iface>`)
/// 2. 检查此网卡所拥有的 IP 是否承载本地处于 LISTEN 状态的 Web 服务端口
pub async fn is_management_interface(iface: &str) -> bool {
    // 1. 检查默认路由 (异步执行外部命令，不阻塞 Tokio worker)
    if let Ok(output) = tokio::process::Command::new("ip")
        .args(["route", "show", "default"])
        .env("LC_ALL", "C")
        .output()
        .await
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.windows(2).any(|w| w[0] == "dev" && w[1] == iface) {
                return true;
            }
        }
    }

    // 2. 检查本机 /proc/net/tcp 活跃监听端口
    if let Ok(content) = tokio::fs::read_to_string("/proc/net/tcp").await {
        for line in content.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // 状态 0A 为 TCP_LISTEN
            if parts.len() >= 4 && parts[3] == "0A" {
                let local_addr = parts[1];
                if let Some((ip_hex, _)) = local_addr.split_once(':') {
                    if let Some(ip) = parse_hex_ip(ip_hex) {
                        // 排除 0.0.0.0 与 127.0.0.1
                        if ip != "0.0.0.0" && ip != "127.0.0.1" {
                            if let Ok(output) = tokio::process::Command::new("ip")
                                .args(["addr", "show", iface])
                                .env("LC_ALL", "C")
                                .output()
                                .await
                            {
                                let stdout = String::from_utf8_lossy(&output.stdout);
                                if stdout.contains(&format!("inet {ip}/")) {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    false
}

/// 解析 /proc/net/tcp 中的十六进制 IP 地址
///
/// Linux 内核使用网络字节序 (Big-Endian) 存储 IP，在小端序架构以 `%08X` 输出时：
/// - hex[0..2] 对应第 4 字节（IP 最右端）
/// - hex[2..4] 对应第 3 字节
/// - hex[4..6] 对应第 2 字节
/// - hex[6..8] 对应第 1 字节（IP 最左端）
pub fn parse_hex_ip(hex: &str) -> Option<String> {
    if hex.len() != 8 {
        return None;
    }
    let b3 = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let b2 = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b1 = u8::from_str_radix(&hex[4..6], 16).ok()?;
    let b0 = u8::from_str_radix(&hex[6..8], 16).ok()?;
    Some(format!("{b0}.{b1}.{b2}.{b3}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_ip() {
        // 127.0.0.1 -> 0100007F in Linux /proc/net/tcp
        assert_eq!(parse_hex_ip("0100007F"), Some("127.0.0.1".to_string()));

        // 192.168.20.156 -> 9C14A8C0
        assert_eq!(parse_hex_ip("9C14A8C0"), Some("192.168.20.156".to_string()));

        // 0.0.0.0 -> 00000000
        assert_eq!(parse_hex_ip("00000000"), Some("0.0.0.0".to_string()));
    }
}
