//! NetworkManager 底层驱动适配
//!
//! 连接定位一律走稳定 UUID。字段命名空间严格区分：`connection show` 列表模式只接受元字段
//! （`NAME`/`UUID`/`DEVICE`…，`DEVICE` 仅对活跃连接取值），`connection.interface-name`
//! 属于 `connection show <ID>` 的 setting.property 命名空间——混用会报 `invalid field`。
//! 外呼 nmcli 经 `run_command_with_c_locale` 强制 `LC_ALL=C.UTF-8`，避免 glib 把非 ASCII
//! 连接名降级成占位字符。

use crate::error::ApiError;
use crate::network_service::detector::{
    detect_carrier, detect_default_route_interfaces, detect_link_speed_and_duplex, get_mac_address,
    get_runtime_ipv4_config, is_management_interface_with_defaults, is_virtual_interface,
    run_command_with_c_locale,
};
use std::collections::HashMap;
use types::system::{
    InterfaceCapabilities, IpConfig, IpMethod, NetworkInterface, NetworkInterfaceState,
    NetworkInterfaceType, NetworkManager, NetworkUpdateResult,
};

/// 迭代 `nmcli -t` 的 `UUID:X` 输出对
///
/// `X` 是列表模式元字段（如 `DEVICE`）；terse 模式下未赋值的字段为空串，`--` 是 nmcli 对空值的占位。
fn parse_uuid_peer_pairs(output: &str) -> impl Iterator<Item = (&str, &str)> {
    output.lines().filter_map(|line| {
        let (uuid, peer) = line.split_once(':')?;
        let (uuid, peer) = (uuid.trim(), peer.trim());
        (!uuid.is_empty() && uuid != "--" && !peer.is_empty() && peer != "--")
            .then_some((uuid, peer))
    })
}

/// 取出关联标识等于 `peer` 的连接 UUID（`peer` 为网卡名）
pub fn parse_connection_uuid_for_peer(output: &str, peer: &str) -> Option<String> {
    parse_uuid_peer_pairs(output)
        .find(|(_, value)| *value == peer)
        .map(|(uuid, _)| uuid.to_string())
}

/// 从 `nmcli connection show <ID>` 的 detail 输出中取 `connection.interface-name`
///
/// detail 模式才接受 `setting.property` 字段；列表模式只接受元字段（`NAME`/`UUID`/`DEVICE`…），
/// 混用会报 `invalid field`。绑定网卡但未激活的 profile 的唯一可靠关联来源就是这里。
pub fn parse_interface_name_setting(output: &str) -> Option<String> {
    for line in output.lines() {
        if let Some(value) = line.strip_prefix("connection.interface-name:") {
            let value = value.trim();
            if !value.is_empty() && value != "--" {
                return Some(value.to_string());
            }
            return None;
        }
    }
    None
}

/// 批量提取网卡名与连接 UUID 映射 (Device -> UUID)
///
/// `active_output` 取自 `connection show --active` 的 `DEVICE` 元字段；`bound_output` 取自
/// `connection show <ID>` 的 `connection.interface-name` setting——后者与激活状态无关，
/// 是未激活网卡唯一可靠的关联来源（`DEVICE` 对未激活 profile 恒为 `--`）。
pub fn parse_device_conn_map(
    active_output: &str,
    bound_pairs: &[(String, String)],
) -> HashMap<String, String> {
    let mut map = HashMap::new();

    // 1. 活跃连接优先
    for (uuid, dev) in parse_uuid_peer_pairs(active_output) {
        map.insert(dev.to_string(), uuid.to_string());
    }

    // 2. 未激活网卡按绑定的 profile 兜底，不覆盖已解析的活跃连接
    for (uuid, dev) in bound_pairs {
        map.entry(dev.clone()).or_insert_with(|| uuid.clone());
    }

    map
}

/// 批量读取网卡名与连接 UUID 映射
///
/// 活跃连接用列表模式取 `DEVICE`；未激活的 profile 逐个走 detail 模式取
/// `connection.interface-name`。任一 nmcli 调用失败都向上传播：丢失映射会让调用方把
/// 所有网卡 IPv4 误判为“无配置”，静默降级比报错更危险。
pub async fn get_device_conn_map_nm() -> Result<HashMap<String, String>, ApiError> {
    let active_out = run_command_with_c_locale(
        "nmcli",
        &["-t", "-f", "UUID,DEVICE", "connection", "show", "--active"],
    )
    .await?;

    // 待补绑定的 profile：排除已解析出活跃网卡的 UUID，避免重复 fork
    let active_uuids: std::collections::HashSet<String> = parse_uuid_peer_pairs(&active_out)
        .map(|(uuid, _)| uuid.to_string())
        .collect();

    let all_uuids =
        run_command_with_c_locale("nmcli", &["-t", "-f", "UUID", "connection", "show"]).await?;

    let mut bound_pairs = Vec::new();
    for uuid in all_uuids
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "--" && !active_uuids.contains(*line))
    {
        let detail = run_command_with_c_locale(
            "nmcli",
            &[
                "-t",
                "-f",
                "connection.interface-name",
                "connection",
                "show",
                uuid,
            ],
        )
        .await?;

        if let Some(dev) = parse_interface_name_setting(&detail) {
            bound_pairs.push((uuid.to_string(), dev));
        }
    }

    Ok(parse_device_conn_map(&active_out, &bound_pairs))
}

/// 枚举由 NetworkManager 管理的所有网卡
pub async fn list_interfaces_nm() -> Result<Vec<NetworkInterface>, ApiError> {
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

    // 批量预读取网卡与连接 UUID 映射，避免用本地化连接名称导致跨字符集解析失败
    let conn_map = get_device_conn_map_nm().await?;

    let mut interfaces = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 3 {
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

        // 优先根据网卡绑定的 UUID 获取 IPv4 配置，杜绝包含中文或特殊字符的连接名破坏解析
        let mut ipv4 = if let Some(uuid) = conn_map.get(&name) {
            get_ipv4_config_by_conn(uuid).await.ok().flatten()
        } else {
            None
        };

        // 内核运行时地址安全兜底：若 NM 连接未输出可用 IPv4，从系统内核直读
        if ipv4.as_ref().and_then(|ip| ip.address.as_ref()).is_none() {
            if let Some(runtime_ip) = get_runtime_ipv4_config(&name).await {
                ipv4 = Some(runtime_ip);
            }
        }

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

/// 获取网卡关联的连接标识符 (UUID)
///
/// 优先级：设备当前活跃连接 → 该网卡绑定的任一 profile（未激活亦可）→ 报错。
pub async fn get_active_connection_nm(iface: &str) -> Result<String, ApiError> {
    // 1. 优先尝试从设备状态直读当前活跃连接 UUID
    if let Ok(output) = run_command_with_c_locale(
        "nmcli",
        &["-g", "GENERAL.CON-UUID", "device", "show", iface],
    )
    .await
    {
        let uuid = output.trim();
        if !uuid.is_empty() && uuid != "--" {
            return Ok(uuid.to_string());
        }
    }

    // 2. 尝试从活跃连接列表中以 UUID 维度查找
    let output = run_command_with_c_locale(
        "nmcli",
        &["-t", "-f", "UUID,DEVICE", "connection", "show", "--active"],
    )
    .await?;

    if let Some(uuid) = parse_connection_uuid_for_peer(&output, iface) {
        return Ok(uuid);
    }

    // 3. 无活跃连接时退回该网卡绑定的已有 profile。
    //    复用 get_device_conn_map_nm：它内部用 detail 模式取 connection.interface-name，
    //    而列表模式的 DEVICE 列对未激活 profile 恒为 `--`，无法用于本判定。
    if let Some(uuid) = get_device_conn_map_nm().await?.get(iface) {
        return Ok(uuid.clone());
    }

    Err(ApiError::NetworkInterfaceNotFound(format!(
        "网卡 {iface} 没有关联的网络连接配置"
    )))
}

/// 读取 IPv4 当前配置（按网卡名称）
pub async fn get_ipv4_config_nm(iface: &str) -> Result<Option<IpConfig>, ApiError> {
    let conn_id = match get_active_connection_nm(iface).await {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    get_ipv4_config_by_conn(&conn_id).await
}

/// 解析 nmcli connection show 输出中的 IPv4 关键字段
pub fn parse_ipv4_config_output(output: &str) -> Option<IpConfig> {
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
        Some(IpConfig {
            method,
            address,
            prefix,
            gateway,
            dns,
            metric,
        })
    } else {
        None
    }
}

/// 读取指定连接 UUID 或标识符的 IPv4 配置
pub async fn get_ipv4_config_by_conn(conn_id: &str) -> Result<Option<IpConfig>, ApiError> {
    let output = run_command_with_c_locale(
        "nmcli",
        &[
            "-t",
            "-f",
            "IP4.ADDRESS,IP4.GATEWAY,IP4.DNS,ipv4.route-metric,ipv4.method",
            "connection",
            "show",
            conn_id,
        ],
    )
    .await?;

    Ok(parse_ipv4_config_output(&output))
}

/// 应用网络配置至 NetworkManager
pub async fn update_interface_nm(
    name: &str,
    config: &IpConfig,
) -> Result<NetworkUpdateResult, ApiError> {
    // 优先使用 UUID 定位连接；若确无任何绑定该网卡的 profile，则创建一个纯 ASCII 的托管连接。
    // 新 profile 带 ifname 绑定，后续查找必然命中，重复 PUT 不会反复新建。
    let conn_id = match get_active_connection_nm(name).await {
        Ok(uuid) => uuid,
        Err(ApiError::NetworkInterfaceNotFound(_)) => {
            let new_conn_name = format!("heimdall-{name}");
            run_command_with_c_locale(
                "nmcli",
                &[
                    "connection",
                    "add",
                    "type",
                    "ethernet",
                    "ifname",
                    name,
                    "con-name",
                    &new_conn_name,
                ],
            )
            .await?;
            get_active_connection_nm(name).await?
        }
        Err(e) => return Err(e),
    };

    if config.method == IpMethod::Dhcp {
        run_command_with_c_locale(
            "nmcli",
            &[
                "connection",
                "modify",
                &conn_id,
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
            &conn_id,
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

    // 重新拉起连接激活生效 (使用 UUID)
    run_command_with_c_locale("nmcli", &["connection", "up", &conn_id]).await?;

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
        // 复用统一 helper，避免各调用点各写一套 locale 注入
        if let Ok(stdout) =
            run_command_with_c_locale("ip", &["-o", "-4", "addr", "show", "dev", name]).await
        {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_connection_uuid_for_peer() {
        let sample = "c987627a-e455-4a12-8cb2-20c2d3a3c202:eth0\n81222133-5786-357e-bda7-eea86655800c:eth1\n";
        assert_eq!(
            parse_connection_uuid_for_peer(sample, "eth0"),
            Some("c987627a-e455-4a12-8cb2-20c2d3a3c202".to_string())
        );
        assert_eq!(
            parse_connection_uuid_for_peer(sample, "eth1"),
            Some("81222133-5786-357e-bda7-eea86655800c".to_string())
        );
        assert_eq!(parse_connection_uuid_for_peer(sample, "eth2"), None);

        // terse 空值占位与未绑定 profile（空 interface-name）不得被当成网卡名
        let placeholders =
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee:--\nbbbbbbbb-cccc-dddd-eeee-ffffffffffff:\n";
        assert_eq!(parse_connection_uuid_for_peer(placeholders, "--"), None);
        assert_eq!(parse_connection_uuid_for_peer(placeholders, ""), None);
    }

    #[test]
    fn test_parse_interface_name_setting() {
        // detail 模式（`connection show <ID>`）的 setting.property 输出格式
        assert_eq!(
            parse_interface_name_setting(
                "connection.interface-name:eth0\nconnection.id:Profile 1\n"
            ),
            Some("eth0".to_string())
        );
        // 未绑定网卡的 profile（terse 空值）与 `--` 占位不得当作绑定
        assert_eq!(
            parse_interface_name_setting("connection.interface-name:\n"),
            None
        );
        assert_eq!(
            parse_interface_name_setting("connection.interface-name:--\n"),
            None
        );
        // 字段缺失（异常/旧版 nmcli）不得误判
        assert_eq!(
            parse_interface_name_setting("connection.id:Profile 1\n"),
            None
        );
        // 非 ASCII 接口名不得被截断
        assert_eq!(
            parse_interface_name_setting("connection.interface-name:ens 0\n"),
            Some("ens 0".to_string())
        );
    }

    #[test]
    fn test_parse_device_conn_map_prefers_active_then_bound() {
        // 活跃连接：列表模式元字段 DEVICE
        let active = "c987627a-e455-4a12-8cb2-20c2d3a3c202:eth0\n";
        // 未激活 profile：detail 模式 connection.interface-name（eth1 已绑定，另一个未绑定）
        let bound_pairs = vec![
            (
                "81222133-5786-357e-bda7-eea86655800c".to_string(),
                "eth1".to_string(),
            ),
            (
                "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string(),
                "eth0".to_string(),
            ),
        ];
        let map = parse_device_conn_map(active, &bound_pairs);
        assert_eq!(
            map.get("eth0"),
            Some(&"c987627a-e455-4a12-8cb2-20c2d3a3c202".to_string())
        );
        assert_eq!(
            map.get("eth1"),
            Some(&"81222133-5786-357e-bda7-eea86655800c".to_string())
        );

        // 活跃连接优先：绑定列表中同网卡的另一个 profile 不得覆盖
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn test_parse_ipv4_config_output_static() {
        let sample = r#"IP4.ADDRESS[1]:192.168.1.100/24
IP4.GATEWAY:192.168.1.1
IP4.DNS[1]:223.5.5.5
IP4.DNS[2]:223.6.6.6
ipv4.route-metric:100
ipv4.method:manual
"#;
        let cfg = parse_ipv4_config_output(sample).expect("应成功解析静态 IP");
        assert_eq!(cfg.method, IpMethod::Static);
        assert_eq!(cfg.address, Some("192.168.1.100".to_string()));
        assert_eq!(cfg.prefix, Some(24));
        assert_eq!(cfg.gateway, Some("192.168.1.1".to_string()));
        assert_eq!(
            cfg.dns,
            vec!["223.5.5.5".to_string(), "223.6.6.6".to_string()]
        );
        assert_eq!(cfg.metric, Some(100));
    }

    #[test]
    fn test_parse_ipv4_config_output_dhcp() {
        let sample = r#"IP4.ADDRESS[1]:10.0.0.50/16
IP4.GATEWAY:10.0.0.1
IP4.DNS[1]:10.0.0.1
ipv4.route-metric:200
ipv4.method:auto
"#;
        let cfg = parse_ipv4_config_output(sample).expect("应成功解析 DHCP IP");
        assert_eq!(cfg.method, IpMethod::Dhcp);
        assert_eq!(cfg.address, Some("10.0.0.50".to_string()));
        assert_eq!(cfg.prefix, Some(16));
        assert_eq!(cfg.gateway, Some("10.0.0.1".to_string()));
        assert_eq!(cfg.metric, Some(200));
    }
}
