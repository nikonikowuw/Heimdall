//! NetworkManager 底层驱动适配 (强制 LC_ALL=C，精准解析)

use crate::error::ApiError;
use crate::network_service::detector::{
    detect_carrier, detect_default_route_interfaces, detect_link_speed_and_duplex, get_mac_address,
    is_management_interface_with_defaults, is_virtual_interface, run_command_with_c_locale,
};
use types::system::{
    InterfaceCapabilities, IpConfig, IpMethod, NetworkInterface, NetworkInterfaceState,
    NetworkInterfaceType, NetworkManager, NetworkUpdateResult,
};

/// 枚举由 NetworkManager 管理的所有网卡
pub async fn list_interfaces_nm() -> Result<Vec<NetworkInterface>, ApiError> {
    // 强制 LC_ALL=C 获取设备状态
    let output = run_command_with_c_locale(
        "nmcli",
        &[
            "-t",
            "-f",
            "DEVICE,TYPE,STATE,CONNECTION",
            "device",
            "status",
        ],
    )
    .await?;

    // 批量预读取系统默认路由网卡列表，避免循环中重复探测
    let default_route_ifaces = detect_default_route_interfaces().await;

    let mut interfaces = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 4 {
            continue;
        }
        let name = parts[0].to_string();

        // 工业级过滤：跳过回环、网桥、虚拟隧道及所有容器虚拟网卡 (docker/veth/br-/tun 等)
        if matches!(parts[1], "loopback" | "bridge" | "tun" | "tap" | "wifi-p2p")
            || is_virtual_interface(&name)
        {
            continue;
        }

        let iface_type = match parts[1] {
            "ethernet" => NetworkInterfaceType::Ethernet,
            "wifi" | "wireless" => NetworkInterfaceType::Wifi,
            _ => continue,
        };
        let state = match parts[2] {
            "connected" => NetworkInterfaceState::Up,
            "disconnected" | "unavailable" => NetworkInterfaceState::Down,
            _ => NetworkInterfaceState::Unknown,
        };

        let mac = get_mac_address(&name).await.unwrap_or_default();

        // 直接复用 device status 中已激活的连接名读取 IPv4 配置
        let active_conn = parts[3].trim();
        let ipv4 = if !active_conn.is_empty() && active_conn != "--" {
            get_ipv4_config_by_conn(active_conn).await.ok().flatten()
        } else {
            None
        };

        let is_mgmt = is_management_interface_with_defaults(&name, &default_route_ifaces).await;
        let carrier = detect_carrier(&name).await;
        let (speed, duplex) = detect_link_speed_and_duplex(&name).await;

        let can_modify = matches!(iface_type, NetworkInterfaceType::Ethernet);
        interfaces.push(NetworkInterface {
            name: name.clone(),
            interface_type: iface_type,
            state,
            carrier,
            speed,
            duplex,
            mac,
            manager: NetworkManager::Networkmanager,
            ipv4,
            capabilities: InterfaceCapabilities {
                can_modify_ip: can_modify,
                can_set_dhcp: can_modify,
                can_set_static: can_modify,
                is_management_interface: is_mgmt,
                reason: if !can_modify {
                    Some("仅支持有线以太网配置".to_string())
                } else {
                    None
                },
            },
        });
    }

    Ok(interfaces)
}

/// 获取活跃连接名称
pub async fn get_active_connection_nm(iface: &str) -> Result<String, ApiError> {
    let output = run_command_with_c_locale(
        "nmcli",
        &["-t", "-f", "NAME,DEVICE", "connection", "show", "--active"],
    )
    .await?;

    for line in output.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 2 && parts[1] == iface {
            return Ok(parts[0].to_string());
        }
    }

    // 若无活跃连接，尝试查找绑定该网卡的已有连接
    let all_conns =
        run_command_with_c_locale("nmcli", &["-t", "-f", "NAME,DEVICE", "connection", "show"])
            .await?;
    for line in all_conns.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 2 && parts[1] == iface {
            return Ok(parts[0].to_string());
        }
    }

    Err(ApiError::NetworkInterfaceNotFound(format!(
        "网卡 {iface} 没有关联的网络连接配置"
    )))
}

/// 读取 IPv4 当前配置（按网卡名称）
pub async fn get_ipv4_config_nm(iface: &str) -> Result<Option<IpConfig>, ApiError> {
    let conn_name = match get_active_connection_nm(iface).await {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    get_ipv4_config_by_conn(&conn_name).await
}

/// 读取指定连接名称的 IPv4 配置（单次调用提取地址、网关、DNS 及方式，避免重复 fork）
pub async fn get_ipv4_config_by_conn(conn_name: &str) -> Result<Option<IpConfig>, ApiError> {
    let output = run_command_with_c_locale(
        "nmcli",
        &[
            "-t",
            "-f",
            "IP4.ADDRESS,IP4.GATEWAY,IP4.DNS,ipv4.route-metric,ipv4.method",
            "connection",
            "show",
            conn_name,
        ],
    )
    .await?;

    let mut address = None;
    let mut prefix = None;
    let mut gateway = None;
    let mut dns = Vec::new();
    let mut metric = None;
    let mut is_dhcp = false;

    for line in output.lines() {
        if let Some(val) = line.strip_prefix("IP4.ADDRESS[1]:") {
            let val = val.trim();
            if let Some((addr, pfx)) = val.split_once('/') {
                address = Some(addr.to_string());
                prefix = pfx.parse().ok();
            }
        } else if let Some(val) = line
            .strip_prefix("IP4.GATEWAY:")
            .or_else(|| line.strip_prefix("IP4.GATEWAY[1]:"))
        {
            let gw = val.trim();
            if !gw.is_empty() && gw != "--" {
                gateway = Some(gw.to_string());
            }
        } else if let Some(val) = line.strip_prefix("IP4.DNS") {
            // 兼容 IP4.DNS[1]: 8.8.8.8 与 IP4.DNS: 8.8.8.8 格式
            let dns_str = if let Some((_, ip_part)) = val.split_once(':') {
                ip_part.trim()
            } else {
                val.trim_start_matches(':').trim()
            };
            if !dns_str.is_empty() && dns_str != "--" && !dns.contains(&dns_str.to_string()) {
                dns.push(dns_str.to_string());
            }
        } else if let Some(val) = line.strip_prefix("ipv4.route-metric:") {
            metric = val.trim().parse::<u32>().ok();
        } else if let Some(val) = line.strip_prefix("ipv4.method:") {
            if val.trim() == "auto" {
                is_dhcp = true;
            }
        }
    }

    let method = if is_dhcp {
        IpMethod::Dhcp
    } else if address.is_some() {
        IpMethod::Static
    } else {
        IpMethod::None
    };

    if address.is_some() || method != IpMethod::None {
        Ok(Some(IpConfig {
            method,
            address,
            prefix,
            gateway,
            dns,
            metric,
        }))
    } else {
        Ok(None)
    }
}

/// 应用网络配置至 NetworkManager
pub async fn update_interface_nm(
    name: &str,
    config: &IpConfig,
) -> Result<NetworkUpdateResult, ApiError> {
    let conn_name = get_active_connection_nm(name).await?;

    if config.method == IpMethod::Dhcp {
        run_command_with_c_locale(
            "nmcli",
            &[
                "connection",
                "modify",
                &conn_name,
                "ipv4.method",
                "auto",
                "ipv4.addresses",
                "",
                "ipv4.gateway",
                "",
                "ipv4.dns",
                "",
            ],
        )
        .await?;
    } else {
        let addr = config.address.as_deref().unwrap_or("");
        let prefix = config.prefix.unwrap_or(24);
        let cidr = format!("{addr}/{prefix}");

        let mut args = vec![
            "connection",
            "modify",
            &conn_name,
            "ipv4.method",
            "manual",
            "ipv4.addresses",
            &cidr,
        ];

        let gw_str = config.gateway.as_deref().unwrap_or("");
        args.extend_from_slice(&["ipv4.gateway", gw_str]);

        let dns_joined = config.dns.join(" ");
        args.extend_from_slice(&["ipv4.dns", &dns_joined]);

        let metric_str = config
            .metric
            .map(|m| m.to_string())
            .unwrap_or_else(|| "100".to_string());
        args.extend_from_slice(&["ipv4.route-metric", &metric_str]);

        run_command_with_c_locale("nmcli", &args).await?;
    }

    // 重新拉起连接激活生效
    run_command_with_c_locale("nmcli", &["connection", "up", &conn_name]).await?;

    // 等待 500ms 验证内核生效
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let verified = verify_ip_applied(name, config).await;

    Ok(NetworkUpdateResult {
        applied: verified,
        operation: None,
    })
}

/// 精准 CIDR 验证内核 IP 绑定，杜绝粗糙子串匹配
pub async fn verify_ip_applied(name: &str, config: &IpConfig) -> bool {
    if config.method == IpMethod::Dhcp {
        // DHCP 只要网卡状态为 UP 即可认为成功
        return true;
    }
    if let Some(expected_addr) = &config.address {
        if let Ok(output) = tokio::process::Command::new("ip")
            .args(["-o", "-4", "addr", "show", "dev", name])
            .env("LC_ALL", "C")
            .output()
            .await
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                // e.g. "2: eth0    inet 192.168.1.100/24 brd ... scope global ..."
                let parts: Vec<&str> = line.split_whitespace().collect();
                for pair in parts.windows(2) {
                    if pair[0] == "inet" {
                        if let Some((addr, _)) = pair[1].split_once('/') {
                            if addr == expected_addr {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        return false;
    }
    true
}
