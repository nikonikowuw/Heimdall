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

/// 解析 /proc/net/route 文本内容，提取承载默认路由的网卡列表（按 Metric 升序排列）
pub fn parse_default_route_interfaces(content: &str) -> Vec<String> {
    let mut routes: Vec<(String, u32)> = Vec::new();
    for line in content.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // fields: [0: Iface, 1: Destination, 2: Gateway, 3: Flags, 4: RefCnt, 5: Use, 6: Metric, 7: Mask]
        // Destination 为 00000000 代表默认路由 (0.0.0.0)
        if fields.len() >= 7 && fields[1] == "00000000" {
            let flags = u32::from_str_radix(fields[3], 16).unwrap_or(0);
            // RTF_UP = 0x0001 (活跃状态路由)
            if flags & 0x0001 != 0 {
                let metric = fields[6].parse::<u32>().unwrap_or(u32::MAX);
                routes.push((fields[0].to_string(), metric));
            }
        }
    }
    // 按 Metric 从小到大排序（Metric 越小优先级越高）
    routes.sort_by_key(|(_, metric)| *metric);

    // 去重保持优先级顺序
    let mut result = Vec::new();
    for (iface, _) in routes {
        if !result.contains(&iface) {
            result.push(iface);
        }
    }
    result
}

/// 快速读取系统所有承载默认路由的网卡 (按 Metric 升序，耗时 <0.05ms，零子进程创建)
pub async fn detect_default_route_interfaces() -> Vec<String> {
    if let Ok(content) = tokio::fs::read_to_string("/proc/net/route").await {
        return parse_default_route_interfaces(&content);
    }
    Vec::new()
}

/// 快速读取系统主默认路由网卡 (最低 Metric，耗时 <0.05ms，零子进程创建)
pub async fn detect_default_route_interface() -> Option<String> {
    detect_default_route_interfaces().await.into_iter().next()
}

/// 读取系统默认网关与对应网卡 (耗时 <0.05ms，直接读取 /proc/net/route)
pub async fn detect_default_gateway() -> Option<(String, String)> {
    if let Ok(content) = tokio::fs::read_to_string("/proc/net/route").await {
        for line in content.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() >= 3 && fields[1] == "00000000" {
                let iface = fields[0].to_string();
                if let Some(gw_ip) = parse_hex_ip(fields[2]) {
                    if gw_ip != "0.0.0.0" {
                        return Some((iface, gw_ip));
                    }
                }
            }
        }
    }
    None
}

/// 读取宿主系统 DNS 配置列表
pub async fn read_system_dns() -> Vec<String> {
    let candidate_paths = ["/run/systemd/resolve/resolv.conf", "/etc/resolv.conf"];
    let mut dns = Vec::new();
    for path in candidate_paths {
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            for line in content.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("nameserver") {
                    let ip = rest.trim();
                    if !ip.is_empty() && !ip.starts_with("127.") && !dns.contains(&ip.to_string()) {
                        dns.push(ip.to_string());
                    }
                }
            }
            if !dns.is_empty() {
                break;
            }
        }
    }
    dns
}

/// 从内核直接读取网卡当前实时生效的 IPv4 与掩码（作为 NetworkManager 等 Profile 缺失时的安全兜底）
pub async fn get_runtime_ipv4_config(iface: &str) -> Option<types::system::IpConfig> {
    let output = tokio::process::Command::new("ip")
        .args(["-4", "-o", "addr", "show", iface])
        .env("LC_ALL", "C")
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("inet ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            for pair in parts.windows(2) {
                if pair[0] == "inet" {
                    if let Some((addr, pfx)) = pair[1].split_once('/') {
                        let is_dhcp = line.contains(" dynamic");
                        let gateway =
                            if let Some((gw_iface, gw_ip)) = detect_default_gateway().await {
                                if gw_iface == iface {
                                    Some(gw_ip)
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                        return Some(types::system::IpConfig {
                            method: if is_dhcp {
                                types::system::IpMethod::Dhcp
                            } else {
                                types::system::IpMethod::Static
                            },
                            address: Some(addr.to_string()),
                            prefix: pfx.parse().ok(),
                            gateway,
                            dns: read_system_dns().await,
                            metric: None,
                        });
                    }
                }
            }
        }
    }
    None
}

/// 快速识别系统当前主网卡网络身份（网卡名、IP、MAC 地址）
pub async fn detect_primary_network_identity() -> (Option<String>, Option<String>, Option<String>) {
    let primary_iface = detect_default_route_interface().await;

    if let Some(ref iface) = primary_iface {
        let mac = get_mac_address(iface).await.ok();
        let ip = get_runtime_ipv4_config(iface)
            .await
            .and_then(|cfg| cfg.address);
        return (Some(iface.clone()), ip, mac);
    }

    // 若无默认路由，尝试在 /sys/class/net 寻找第一个非虚拟且处于 UP 状态的物理网卡
    if let Ok(mut entries) = tokio::fs::read_dir("/sys/class/net").await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if !is_virtual_interface(&name) {
                let operstate =
                    tokio::fs::read_to_string(format!("/sys/class/net/{name}/operstate"))
                        .await
                        .unwrap_or_default();
                if operstate.trim() == "up" {
                    let mac = get_mac_address(&name).await.ok();
                    let ip = get_runtime_ipv4_config(&name)
                        .await
                        .and_then(|cfg| cfg.address);
                    return (Some(name), ip, mac);
                }
            }
        }
    }

    (None, None, None)
}

/// 动态判定网卡是否为当前管理网卡（Management Interface）
///
/// 判定逻辑：
/// 1. 检查网卡是否承载系统默认路由（优先 /proc/net/route 极速直读，失败时回退 ip route show default）
/// 2. 检查此网卡所拥有的 IP 是否承载本地处于 LISTEN 状态的 Web 服务端口
pub async fn is_management_interface(iface: &str) -> bool {
    let default_ifaces = detect_default_route_interfaces().await;
    is_management_interface_with_defaults(iface, &default_ifaces).await
}

/// 接收已预读取的默认路由网卡列表，消除循环中重复读取 /proc/net/route 的开销
pub async fn is_management_interface_with_defaults(iface: &str, default_ifaces: &[String]) -> bool {
    // 1. 优先检查预读取的所有承载默认路由的网卡
    if default_ifaces.iter().any(|dev| dev == iface) {
        return true;
    }

    // 2. 若 /proc/net/route 读取为空（如特殊容器环境），回退检查 ip route show default
    if default_ifaces.is_empty() {
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
    }

    // 3. 检查本机 /proc/net/tcp 活跃监听端口
    if let Ok(content) = tokio::fs::read_to_string("/proc/net/tcp").await {
        let mut listen_ips = Vec::new();
        for line in content.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // 状态 0A 为 TCP_LISTEN
            if parts.len() >= 4 && parts[3] == "0A" {
                let local_addr = parts[1];
                if let Some((ip_hex, _)) = local_addr.split_once(':') {
                    if let Some(ip) = parse_hex_ip(ip_hex) {
                        // 排除 0.0.0.0 与 127.0.0.1，且去重避免重复查询
                        if ip != "0.0.0.0" && ip != "127.0.0.1" && !listen_ips.contains(&ip) {
                            listen_ips.push(ip);
                        }
                    }
                }
            }
        }

        if !listen_ips.is_empty() {
            // 至多执行一次命令获取该网卡的所有 IPv4 地址
            if let Ok(output) = tokio::process::Command::new("ip")
                .args(["-o", "-4", "addr", "show", iface])
                .env("LC_ALL", "C")
                .output()
                .await
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for ip in listen_ips {
                    if stdout.contains(&format!("inet {ip}/")) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// 判定网卡是否为软件虚拟网卡或本地回环（如 lo, docker0, veth*, br-*, virbr*, tun*, tap*, wg* 等）
///
/// 判定标准：
/// 1. 本地回环：`lo`, `lo0`
/// 2. 经典虚拟网卡命名模式：
///    - 容器与虚拟化网桥：`docker*`, `br-*`, `veth*`, `virbr*`, `vmnet*`, `cni*`, `flannel*`
///    - VPN 与隧道：`tun*`, `tap*`, `wg*`, `tailscale*`, `zt*`, `sit*`, `ip6tnl*`
///    - 虚拟设备与 P2P：`dummy*`, `p2p-dev-*`
/// 3. Linux sysfs 设备总线链接权威判定：
///    若在 `/sys/class/net/{name}` 存在，检查其是否存在物理硬件设备符号链接 `device`（挂载于 PCI/USB/Platform 总线）。
///    若无 `device` 符号链接且不是以常规物理前缀命名（`eth`, `en`, `wl`, `ww`），判定为纯软件虚拟接口。
pub fn is_virtual_interface(name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return true;
    }

    // 1. 标准本地回环
    if name == "lo" || name == "lo0" {
        return true;
    }

    // 2. 常见虚拟网卡名字特征防御
    if name.starts_with("docker")
        || name.starts_with("br-")
        || name.starts_with("veth")
        || name.starts_with("virbr")
        || name.starts_with("vmnet")
        || name.starts_with("cni")
        || name.starts_with("flannel")
        || name.starts_with("tun")
        || name.starts_with("tap")
        || name.starts_with("wg")
        || name.starts_with("tailscale")
        || name.starts_with("zt")
        || name.starts_with("dummy")
        || name.starts_with("bond")
        || name.starts_with("sit")
        || name.starts_with("ip6tnl")
        || name.starts_with("p2p-dev-")
    {
        return true;
    }

    // 3. Linux sysfs 设备总线判定
    #[cfg(target_os = "linux")]
    {
        let net_dir = std::path::Path::new("/sys/class/net").join(name);
        if net_dir.exists() {
            let device_link = net_dir.join("device");
            let is_standard_physical = name.starts_with("eth")
                || name.starts_with("en")
                || name.starts_with("wl")
                || name.starts_with("ww");
            if !device_link.exists() && !is_standard_physical {
                return true;
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
    fn test_parse_default_route_interfaces() {
        // 模拟包含多默认路由且乱序的 /proc/net/route
        let sample = "\
Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
eth1\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
docker0\t000011AC\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0\n\
eth0\t00000000\t0114A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
eth2\t00000000\t0102A8C0\t0002\t0\t0\t50\t00000000\t0\t0\t0\n";

        // eth2 Flags=0002 (无 RTF_UP 0x0001，处于 Down 状态，不应被采纳)
        // eth0 Metric=100，eth1 Metric=600，应按 Metric 升序排列：[eth0, eth1]
        let ifaces = parse_default_route_interfaces(sample);
        assert_eq!(ifaces, vec!["eth0", "eth1"]);
    }

    #[test]
    fn test_parse_hex_ip() {
        // 127.0.0.1 -> 0100007F in Linux /proc/net/tcp
        assert_eq!(parse_hex_ip("0100007F"), Some("127.0.0.1".to_string()));

        // 192.168.20.156 -> 9C14A8C0
        assert_eq!(parse_hex_ip("9C14A8C0"), Some("192.168.20.156".to_string()));

        // 0.0.0.0 -> 00000000
        assert_eq!(parse_hex_ip("00000000"), Some("0.0.0.0".to_string()));
    }

    #[test]
    fn test_is_virtual_interface() {
        assert!(is_virtual_interface("lo"));
        assert!(is_virtual_interface("lo0"));
        assert!(is_virtual_interface("docker0"));
        assert!(is_virtual_interface("br-a937ad6ccef6"));
        assert!(is_virtual_interface("veth0773fb4"));
        assert!(is_virtual_interface("virbr0"));
        assert!(is_virtual_interface("tun0"));
        assert!(is_virtual_interface("tap0"));
        assert!(is_virtual_interface("wg0"));
        assert!(is_virtual_interface("tailscale0"));
        assert!(is_virtual_interface("p2p-dev-wlp0s20f3"));
        assert!(is_virtual_interface("dummy0"));

        assert!(!is_virtual_interface("eth0"));
        assert!(!is_virtual_interface("eth1"));
        assert!(!is_virtual_interface("enp3s0"));
        assert!(!is_virtual_interface("ens33"));
        assert!(!is_virtual_interface("wlan0"));
        assert!(!is_virtual_interface("wlp0s20f3"));
        assert!(!is_virtual_interface("wwan0"));
    }
}
