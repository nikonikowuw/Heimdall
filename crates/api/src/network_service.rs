//! 网络配置服务：封装宿主 OS 的网络管理能力
//!
//! 首版适配 NetworkManager (nmcli) + systemd-networkd (networkctl)，并在 macOS
//! 上通过 `ifconfig` / `networksetup` 提供只读网卡枚举。未识别管理服务的网卡只读展示。

#[cfg(target_os = "macos")]
use std::collections::HashMap;
use std::process::Command;

#[cfg(target_os = "macos")]
use std::net::Ipv4Addr;

use crate::error::ApiError;
use types::system::{
    InterfaceCapabilities, IpConfig, IpMethod, NetworkInterface, NetworkInterfaceState,
    NetworkInterfaceType, NetworkInterfacesResponse, NetworkManager, NetworkUpdateResult,
};

/// 网络服务
#[derive(Debug)]
pub struct NetworkService;

impl NetworkService {
    /// 列出所有网卡
    pub async fn list_interfaces() -> Result<Vec<NetworkInterface>, ApiError> {
        #[cfg(target_os = "macos")]
        {
            list_interfaces_macos().await
        }

        #[cfg(not(target_os = "macos"))]
        {
            let manager = detect_network_manager();

            match manager {
                NetworkManager::Networkmanager => list_interfaces_nm().await,
                NetworkManager::SystemdNetworkd => list_interfaces_networkd().await,
                _ => Ok(Vec::new()),
            }
        }
    }

    /// 获取单个网卡详情
    pub async fn get_interface(name: &str) -> Result<NetworkInterface, ApiError> {
        validate_interface_name(name)?;
        let interfaces = Self::list_interfaces().await?;
        interfaces
            .into_iter()
            .find(|i| i.name == name)
            .ok_or_else(|| ApiError::NetworkInterfaceNotFound(name.to_string()))
    }

    /// 修改网卡配置
    pub async fn update_interface(
        name: &str,
        config: &IpConfig,
    ) -> Result<NetworkUpdateResult, ApiError> {
        validate_interface_name(name)?;
        validate_ip_config(config)?;
        let iface = Self::get_interface(name).await?;

        if !iface.capabilities.can_modify_ip {
            return Err(ApiError::NetworkInterfaceReadOnly(
                iface
                    .capabilities
                    .reason
                    .unwrap_or_else(|| "网卡不支持修改".to_string()),
            ));
        }

        let manager = detect_network_manager();
        match manager {
            NetworkManager::Networkmanager => update_interface_nm(name, config).await,
            NetworkManager::SystemdNetworkd => update_interface_networkd(name, config).await,
            _ => Err(ApiError::NetworkInvalid("不支持的网络管理服务".to_string())),
        }
    }

    /// 构建 `NetworkInterfacesResponse`（供 handler 调用）
    pub async fn list_with_pending() -> Result<NetworkInterfacesResponse, ApiError> {
        let interfaces = Self::list_interfaces().await?;
        Ok(NetworkInterfacesResponse {
            interfaces,
            pending_operation: None, // 首版：管理网卡变更暂不实现
        })
    }
}

/// 严格校验 Linux 网卡名称，严防路径穿越与命令注入
fn validate_interface_name(name: &str) -> Result<(), ApiError> {
    if name.is_empty() || name.len() > 15 {
        return Err(ApiError::NetworkInvalid(format!(
            "无效的网卡名称长度: {name}"
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(ApiError::NetworkInvalid(format!(
            "网卡名称包含非法字符: {name}"
        )));
    }
    if name.contains("..") {
        return Err(ApiError::NetworkInvalid(format!(
            "网卡名称禁止包含路径遍历: {name}"
        )));
    }
    Ok(())
}

/// 校验 IP 配置各字段格式，防止 INI 注入与非法网络参数
fn validate_ip_config(config: &IpConfig) -> Result<(), ApiError> {
    if config.method == IpMethod::Static {
        let addr_str = config
            .address
            .as_deref()
            .ok_or_else(|| ApiError::NetworkInvalid("静态 IP 模式必须提供 IP 地址".to_string()))?;
        addr_str
            .parse::<std::net::Ipv4Addr>()
            .map_err(|e| ApiError::NetworkInvalid(format!("IP 地址格式无效: {addr_str} ({e})")))?;

        if let Some(prefix) = config.prefix {
            if !(1..=32).contains(&prefix) {
                return Err(ApiError::NetworkInvalid(format!(
                    "子网前缀必须在 1-32 之间: {prefix}"
                )));
            }
        }

        if let Some(gw) = &config.gateway {
            if !gw.is_empty() {
                gw.parse::<std::net::Ipv4Addr>().map_err(|e| {
                    ApiError::NetworkInvalid(format!("网关地址格式无效: {gw} ({e})"))
                })?;
            }
        }

        for dns_server in &config.dns {
            if !dns_server.is_empty() {
                dns_server.parse::<std::net::Ipv4Addr>().map_err(|e| {
                    ApiError::NetworkInvalid(format!("DNS 地址格式无效: {dns_server} ({e})"))
                })?;
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn list_interfaces_macos() -> Result<Vec<NetworkInterface>, ApiError> {
    let ifconfig_output = run_command_async("ifconfig", &["-a"]).await?;
    let hardware_ports = run_command_async("networksetup", &["-listallhardwareports"])
        .await
        .unwrap_or_default();
    let network_services = run_command_async("networksetup", &["-listnetworkserviceorder"])
        .await
        .unwrap_or_default();

    let hardware_by_device = parse_macos_hardware_ports(&hardware_ports);
    let service_by_device = parse_macos_network_services(&network_services);
    let records = parse_macos_ifconfig(&ifconfig_output);
    let mut interfaces = Vec::with_capacity(records.len());

    for record in records {
        let interface_type = macos_interface_type(
            &record.name,
            hardware_by_device.get(&record.name).map(String::as_str),
        );
        let ipv4 = match (&record.ipv4, service_by_device.get(&record.name)) {
            (Some(observed), Some(service_name)) => {
                Some(get_macos_ipv4_config(service_name, observed).await)
            }
            (Some(observed), None) => Some(observed_ip_config(observed)),
            (None, _) => None,
        };

        let state = macos_interface_state(&record);
        let name = record.name;
        let mac = record.mac.unwrap_or_default();
        interfaces.push(NetworkInterface {
            name,
            interface_type,
            state,
            mac,
            manager: NetworkManager::Unmanaged,
            ipv4,
            capabilities: InterfaceCapabilities {
                can_modify_ip: false,
                can_set_dhcp: false,
                can_set_static: false,
                is_management_interface: false,
                reason: Some("macOS 网络配置暂仅支持只读展示".to_string()),
            },
        });
    }

    Ok(interfaces)
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
struct MacIpv4 {
    address: String,
    prefix: Option<u32>,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct MacInterfaceRecord {
    name: String,
    flags: String,
    mac: Option<String>,
    ipv4: Option<MacIpv4>,
    status_active: Option<bool>,
}

#[cfg(target_os = "macos")]
fn parse_macos_hardware_ports(output: &str) -> HashMap<String, String> {
    let mut ports = HashMap::new();
    let mut hardware_port = None;

    for line in output.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("Hardware Port:") {
            hardware_port = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("Device:") {
            let device = value.trim();
            if !device.is_empty() {
                if let Some(port) = hardware_port.as_ref() {
                    ports.insert(device.to_string(), port.clone());
                }
            }
        }
    }

    ports
}

#[cfg(target_os = "macos")]
fn parse_macos_network_services(output: &str) -> HashMap<String, String> {
    let mut services = HashMap::new();
    let mut pending_service = None;

    for line in output.lines() {
        let line = line.trim();
        if line.starts_with('(') && !line.starts_with("(Hardware Port:") {
            if let Some(close) = line.find(')') {
                let name = line[close + 1..].trim().trim_start_matches('*').trim();
                if !name.is_empty() {
                    pending_service = Some(name.to_string());
                }
            }
            continue;
        }

        if let Some(value) = line.strip_prefix("(Hardware Port:") {
            if let Some(device_start) = value.find(", Device:") {
                let device = value[device_start + ", Device:".len()..]
                    .trim_end_matches(')')
                    .trim();
                if !device.is_empty() {
                    if let Some(service) = pending_service.take() {
                        services.insert(device.to_string(), service);
                    }
                }
            }
        }
    }

    services
}

#[cfg(target_os = "macos")]
fn parse_macos_ifconfig(output: &str) -> Vec<MacInterfaceRecord> {
    let mut records = Vec::new();
    let mut current = None;

    for line in output.lines() {
        if let Some((name, header)) = line.split_once(": flags=") {
            if let Some(record) = current.take() {
                records.push(record);
            }

            let flags = header
                .split_once('<')
                .and_then(|(_, rest)| rest.split_once('>').map(|(flags, _)| flags))
                .unwrap_or_default()
                .to_string();
            current = Some(MacInterfaceRecord {
                name: name.to_string(),
                flags,
                mac: None,
                ipv4: None,
                status_active: None,
            });
            continue;
        }

        let Some(record) = current.as_mut() else {
            continue;
        };
        let line = line.trim();
        if let Some(value) = line.strip_prefix("ether ") {
            record.mac = Some(value.trim().to_uppercase());
        } else if let Some(value) = line.strip_prefix("inet ") {
            if record.ipv4.is_none() {
                record.ipv4 = parse_macos_ipv4(value);
            }
        } else if let Some(value) = line.strip_prefix("status:") {
            record.status_active = Some(value.trim().eq_ignore_ascii_case("active"));
        }
    }

    if let Some(record) = current {
        records.push(record);
    }
    records
}

#[cfg(target_os = "macos")]
fn parse_macos_ipv4(value: &str) -> Option<MacIpv4> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let address = parts.first()?.parse::<Ipv4Addr>().ok()?.to_string();
    let netmask_index = parts.iter().position(|part| *part == "netmask")?;
    let netmask = parts.get(netmask_index + 1)?;

    Some(MacIpv4 {
        address,
        prefix: parse_macos_netmask(netmask),
    })
}

#[cfg(target_os = "macos")]
fn parse_macos_netmask(value: &str) -> Option<u32> {
    if let Some(hex) = value.strip_prefix("0x") {
        return u32::from_str_radix(hex, 16).ok().map(u32::count_ones);
    }
    value
        .parse::<Ipv4Addr>()
        .ok()
        .map(|mask| mask.octets().into_iter().map(u8::count_ones).sum())
}

#[cfg(target_os = "macos")]
fn macos_interface_type(name: &str, hardware_port: Option<&str>) -> NetworkInterfaceType {
    if name == "lo0" || name.starts_with("lo") {
        return NetworkInterfaceType::Loopback;
    }

    if let Some(port) = hardware_port {
        let port = port.to_ascii_lowercase();
        if port.contains("wi-fi") || port.contains("wifi") || port.contains("wireless") {
            return NetworkInterfaceType::Wifi;
        }
        if port.contains("ethernet") || port.contains("thunderbolt") {
            return NetworkInterfaceType::Ethernet;
        }
    }

    if name.starts_with("en") {
        NetworkInterfaceType::Ethernet
    } else {
        NetworkInterfaceType::Virtual
    }
}

#[cfg(target_os = "macos")]
fn macos_interface_state(record: &MacInterfaceRecord) -> NetworkInterfaceState {
    if record.status_active == Some(true) {
        NetworkInterfaceState::Up
    } else if record.status_active == Some(false) {
        NetworkInterfaceState::Down
    } else if has_macos_flag(&record.flags, "RUNNING") {
        NetworkInterfaceState::Up
    } else if has_macos_flag(&record.flags, "UP") {
        NetworkInterfaceState::Down
    } else {
        NetworkInterfaceState::Unknown
    }
}

#[cfg(target_os = "macos")]
fn has_macos_flag(flags: &str, expected: &str) -> bool {
    flags.split(',').any(|flag| flag == expected)
}

#[cfg(target_os = "macos")]
fn observed_ip_config(ipv4: &MacIpv4) -> IpConfig {
    IpConfig {
        method: IpMethod::Static,
        address: Some(ipv4.address.clone()),
        prefix: ipv4.prefix,
        gateway: None,
        dns: Vec::new(),
    }
}

#[cfg(target_os = "macos")]
async fn get_macos_ipv4_config(service_name: &str, observed: &MacIpv4) -> IpConfig {
    let mut config = observed_ip_config(observed);
    let info = run_command_async("networksetup", &["-getinfo", service_name])
        .await
        .unwrap_or_default();

    if info.contains("DHCP Configuration") {
        config.method = IpMethod::Dhcp;
    } else if info.contains("Manually Configured") {
        config.method = IpMethod::Static;
    }

    for line in info.lines().map(str::trim) {
        if let Some(value) = line.strip_prefix("IP address:") {
            if let Ok(address) = value.trim().parse::<Ipv4Addr>() {
                config.address = Some(address.to_string());
            }
        } else if let Some(value) = line.strip_prefix("Subnet mask:") {
            config.prefix = parse_macos_netmask(value.trim());
        } else if let Some(value) = line.strip_prefix("Router:") {
            let gateway = value.trim();
            if !gateway.is_empty() && !gateway.eq_ignore_ascii_case("none") {
                config.gateway = Some(gateway.to_string());
            }
        }
    }

    if let Ok(dns_output) =
        run_command_async("networksetup", &["-getdnsservers", service_name]).await
    {
        config.dns = dns_output
            .lines()
            .map(str::trim)
            .filter(|value| value.parse::<std::net::IpAddr>().is_ok())
            .map(str::to_string)
            .collect();
    }

    config
}

fn detect_network_manager() -> NetworkManager {
    #[cfg(target_os = "macos")]
    {
        NetworkManager::Unmanaged
    }

    #[cfg(not(target_os = "macos"))]
    {
        // 检查 nmcli 是否可用
        if Command::new("nmcli")
            .args(["general", "status"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return NetworkManager::Networkmanager;
        }

        // 检查 networkctl 是否可用
        if Command::new("networkctl")
            .args(["status"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return NetworkManager::SystemdNetworkd;
        }

        NetworkManager::Unmanaged
    }
}

#[cfg(not(target_os = "macos"))]
async fn list_interfaces_nm() -> Result<Vec<NetworkInterface>, ApiError> {
    // nmcli -t -f DEVICE,TYPE,STATE,CONNECTION device status
    let output = run_command_async(
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

    let mut interfaces = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 4 {
            continue;
        }
        let name = parts[0].to_string();
        let iface_type = match parts[1] {
            "ethernet" => NetworkInterfaceType::Ethernet,
            "wifi" | "wireless" => NetworkInterfaceType::Wifi,
            "loopback" | "lo" => NetworkInterfaceType::Loopback,
            _ => NetworkInterfaceType::Virtual,
        };
        let state = match parts[2] {
            "connected" => NetworkInterfaceState::Up,
            "disconnected" | "unavailable" => NetworkInterfaceState::Down,
            _ => NetworkInterfaceState::Unknown,
        };

        // 获取 MAC 地址
        let mac = get_mac_address(&name).await.unwrap_or_default();

        // 获取 IP 配置
        let ipv4 = get_ipv4_config_nm(&name).await.ok().flatten();

        // 检测是否为管理接口（硬编码端口 8000 作为首版实现）
        let is_mgmt = is_management_interface(&name, 8000).await;

        let can_modify = matches!(iface_type, NetworkInterfaceType::Ethernet);
        interfaces.push(NetworkInterface {
            name: name.clone(),
            interface_type: iface_type,
            state,
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

#[cfg(not(target_os = "macos"))]
async fn get_mac_address(iface: &str) -> Result<String, ApiError> {
    let path = format!("/sys/class/net/{iface}/address");
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| ApiError::NetworkFailed(format!("读取 MAC 地址失败: {e}")))?;
    Ok(content.trim().to_uppercase())
}

#[cfg(not(target_os = "macos"))]
async fn get_ipv4_config_nm(iface: &str) -> Result<Option<IpConfig>, ApiError> {
    let conn_name = get_active_connection_nm(iface).await?;

    let output = run_command_async(
        "nmcli",
        &[
            "-t",
            "-f",
            "IP4.ADDRESS,IP4.GATEWAY,IP4.DNS",
            "connection",
            "show",
            &conn_name,
        ],
    )
    .await?;

    let mut address = None;
    let mut prefix = None;
    let mut gateway = None;
    let mut dns = Vec::new();

    for line in output.lines() {
        if let Some(val) = line.strip_prefix("IP4.ADDRESS[1]:") {
            let val = val.trim();
            if let Some((addr, pfx)) = val.split_once('/') {
                address = Some(addr.to_string());
                prefix = pfx.parse().ok();
            }
        } else if let Some(val) = line.strip_prefix("IP4.GATEWAY[1]:") {
            gateway = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("IP4.DNS[1]:") {
            dns.push(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("IP4.DNS[2]:") {
            dns.push(val.trim().to_string());
        }
    }

    // 判断是 DHCP 还是 static
    let method = if address.is_some() {
        let is_dhcp = run_command_async(
            "nmcli",
            &["-t", "-f", "IP4.METHOD", "connection", "show", &conn_name],
        )
        .await
        .map(|o| o.contains("auto"))
        .unwrap_or(false);

        if is_dhcp {
            IpMethod::Dhcp
        } else {
            IpMethod::Static
        }
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
        }))
    } else {
        Ok(None)
    }
}

async fn get_active_connection_nm(iface: &str) -> Result<String, ApiError> {
    let output = run_command_async(
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

    Err(ApiError::NetworkInterfaceNotFound(format!(
        "网卡 {iface} 没有活跃连接"
    )))
}

#[cfg(not(target_os = "macos"))]
async fn is_management_interface(iface: &str, expected_port: u16) -> bool {
    // 检查此网卡是否承载 Heimdall 的监听地址
    let port_hex = format!("{:04X}", expected_port);
    if let Ok(content) = tokio::fs::read_to_string("/proc/net/tcp").await {
        for line in content.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let local_addr = parts[1];
                if let Some((ip_hex, port_hex_found)) = local_addr.split_once(':') {
                    if port_hex_found.eq_ignore_ascii_case(&port_hex) {
                        if let Some(ip) = parse_hex_ip(ip_hex) {
                            if let Ok(output) =
                                Command::new("ip").args(["addr", "show", iface]).output()
                            {
                                let stdout = String::from_utf8_lossy(&output.stdout);
                                if stdout.contains(&ip) {
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

#[cfg(not(target_os = "macos"))]
fn parse_hex_ip(hex: &str) -> Option<String> {
    if hex.len() != 8 {
        return None;
    }
    let b0 = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let b1 = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b2 = u8::from_str_radix(&hex[4..6], 16).ok()?;
    let b3 = u8::from_str_radix(&hex[6..8], 16).ok()?;
    Some(format!("{b0}.{b1}.{b2}.{b3}"))
}

async fn update_interface_nm(
    name: &str,
    config: &IpConfig,
) -> Result<NetworkUpdateResult, ApiError> {
    let conn_name = get_active_connection_nm(name).await?;

    if config.method == IpMethod::Dhcp {
        // 切换到 DHCP
        run_command_async(
            "nmcli",
            &["connection", "modify", &conn_name, "ipv4.method", "auto"],
        )
        .await?;
        // 清除静态配置
        run_command_async(
            "nmcli",
            &[
                "connection",
                "modify",
                &conn_name,
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
        // 设置静态 IP
        if let Some(addr) = &config.address {
            let prefix = config.prefix.unwrap_or(24);
            run_command_async(
                "nmcli",
                &[
                    "connection",
                    "modify",
                    &conn_name,
                    "ipv4.method",
                    "manual",
                    "ipv4.addresses",
                    &format!("{addr}/{prefix}"),
                ],
            )
            .await?;
        }
        if let Some(gw) = &config.gateway {
            run_command_async(
                "nmcli",
                &["connection", "modify", &conn_name, "ipv4.gateway", gw],
            )
            .await?;
        }
        if !config.dns.is_empty() {
            let dns_str = config.dns.join(" ");
            run_command_async(
                "nmcli",
                &["connection", "modify", &conn_name, "ipv4.dns", &dns_str],
            )
            .await?;
        }
    }

    // 激活连接
    run_command_async("nmcli", &["connection", "up", &conn_name]).await?;

    // 验证生效
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let verified = verify_ip_applied(name, config).await;

    Ok(NetworkUpdateResult {
        applied: verified,
        operation: None,
    })
}

async fn verify_ip_applied(name: &str, config: &IpConfig) -> bool {
    if let Some(expected_addr) = &config.address {
        if let Ok(output) = Command::new("ip").args(["addr", "show", name]).output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            return stdout.contains(expected_addr);
        }
    }
    true
}

// ─── systemd-networkd 适配 ───

#[cfg(not(target_os = "macos"))]
async fn list_interfaces_networkd() -> Result<Vec<NetworkInterface>, ApiError> {
    let output = run_command_async("networkctl", &["status", "--no-pager"]).await?;

    let mut interfaces = Vec::new();
    for line in output.lines() {
        if let Some(rest) = line.trim().strip_prefix(|c: char| c.is_ascii_digit()) {
            let rest = rest.trim_start_matches([':', ' ']);
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 2 {
                let name = parts[0].to_string();
                let state = if line.contains("routable") || line.contains("configured") {
                    NetworkInterfaceState::Up
                } else {
                    NetworkInterfaceState::Down
                };

                let mac = get_mac_address(&name).await.unwrap_or_default();
                let ipv4 = get_ipv4_config_networkd(&name).await.ok().flatten();
                let is_mgmt = is_management_interface(&name, 8000).await;

                interfaces.push(NetworkInterface {
                    name: name.clone(),
                    interface_type: NetworkInterfaceType::Ethernet,
                    state,
                    mac,
                    manager: NetworkManager::SystemdNetworkd,
                    ipv4,
                    capabilities: InterfaceCapabilities {
                        can_modify_ip: true,
                        can_set_dhcp: true,
                        can_set_static: true,
                        is_management_interface: is_mgmt,
                        reason: None,
                    },
                });
            }
        }
    }

    Ok(interfaces)
}

#[cfg(not(target_os = "macos"))]
async fn get_ipv4_config_networkd(iface: &str) -> Result<Option<IpConfig>, ApiError> {
    let network_dir = "/etc/systemd/network";
    let entries = tokio::fs::read_dir(network_dir).await;

    if let Ok(mut entries) = entries {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("network") {
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    if content.contains(&format!("Name={iface}")) {
                        return parse_network_file(&content);
                    }
                }
            }
        }
    }

    // 从 ip addr 获取当前配置
    let output = Command::new("ip")
        .args(["-4", "-o", "addr", "show", iface])
        .output()
        .map_err(|e| ApiError::NetworkFailed(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("inet ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            for (i, part) in parts.iter().enumerate() {
                if *part == "inet" && i + 1 < parts.len() {
                    let addr_parts: Vec<&str> = parts[i + 1].split('/').collect();
                    if addr_parts.len() == 2 {
                        return Ok(Some(IpConfig {
                            method: IpMethod::Static,
                            address: Some(addr_parts[0].to_string()),
                            prefix: addr_parts[1].parse().ok(),
                            gateway: None,
                            dns: Vec::new(),
                        }));
                    }
                }
            }
        }
    }

    Ok(None)
}

#[cfg(not(target_os = "macos"))]
fn parse_network_file(content: &str) -> Result<Option<IpConfig>, ApiError> {
    let mut address = None;
    let mut prefix = None;
    let mut gateway = None;
    let mut dns = Vec::new();
    let mut is_dhcp = false;

    for line in content.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("DHCP=") {
            is_dhcp = val == "yes";
        } else if let Some(val) = line.strip_prefix("Address=") {
            if let Some((addr, pfx)) = val.split_once('/') {
                address = Some(addr.to_string());
                prefix = pfx.parse().ok();
            }
        } else if let Some(val) = line.strip_prefix("Gateway=") {
            gateway = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("DNS=") {
            dns.push(val.to_string());
        }
    }

    let method = if is_dhcp {
        IpMethod::Dhcp
    } else if address.is_some() {
        IpMethod::Static
    } else {
        IpMethod::None
    };

    Ok(Some(IpConfig {
        method,
        address,
        prefix,
        gateway,
        dns,
    }))
}

async fn update_interface_networkd(
    name: &str,
    config: &IpConfig,
) -> Result<NetworkUpdateResult, ApiError> {
    let network_file = format!("/etc/systemd/network/10-{name}.network");

    let mut content = format!("[Match]\nName={name}\n\n[Network]\n");
    if config.method == IpMethod::Dhcp {
        content.push_str("DHCP=yes\n");
    } else {
        content.push_str("DHCP=no\n");
        if let Some(addr) = &config.address {
            let prefix = config.prefix.unwrap_or(24);
            content.push_str(&format!("Address={addr}/{prefix}\n"));
        }
        if let Some(gw) = &config.gateway {
            content.push_str(&format!("Gateway={gw}\n"));
        }
        for dns_server in &config.dns {
            content.push_str(&format!("DNS={dns_server}\n"));
        }
    }

    tokio::fs::write(&network_file, &content)
        .await
        .map_err(|e| ApiError::NetworkFailed(format!("写入网络配置文件失败: {e}")))?;

    run_command_async("networkctl", &["reconfigure", name]).await?;

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    let verified = verify_ip_applied(name, config).await;

    Ok(NetworkUpdateResult {
        applied: verified,
        operation: None,
    })
}

// ─── 工具函数 ───

async fn run_command_async(cmd: &str, args: &[&str]) -> Result<String, ApiError> {
    let output = tokio::process::Command::new(cmd)
        .args(args)
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

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;

    const IFCONFIG_SAMPLE: &str = r#"lo0: flags=8049<UP,LOOPBACK,RUNNING,MULTICAST> mtu 16384
	inet 127.0.0.1 netmask 0xff000000

en0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
	ether 72:6f:7f:c7:7b:13
	inet 192.168.5.2 netmask 0xffffff00 broadcast 192.168.5.255
	status: active
en1: flags=8963<UP,BROADCAST,SMART,PROMISC,SIMPLEX,MULTICAST> mtu 1500
	ether 36:41:5e:5a:26:c0
	status: inactive
"#;

    #[test]
    fn parses_macos_ifconfig_records() {
        let records = parse_macos_ifconfig(IFCONFIG_SAMPLE);
        assert_eq!(records.len(), 3);

        let loopback = &records[0];
        assert_eq!(loopback.name, "lo0");
        assert_eq!(
            macos_interface_type(&loopback.name, None),
            NetworkInterfaceType::Loopback
        );
        assert_eq!(loopback.ipv4.as_ref().and_then(|ip| ip.prefix), Some(8));

        let wifi = &records[1];
        assert_eq!(wifi.mac.as_deref(), Some("72:6F:7F:C7:7B:13"));
        assert_eq!(
            wifi.ipv4.as_ref().map(|ip| ip.address.as_str()),
            Some("192.168.5.2")
        );
        assert_eq!(wifi.ipv4.as_ref().and_then(|ip| ip.prefix), Some(24));
        assert_eq!(macos_interface_state(wifi), NetworkInterfaceState::Up);

        let inactive = &records[2];
        assert_eq!(macos_interface_state(inactive), NetworkInterfaceState::Down);
    }

    #[test]
    fn parses_macos_hardware_and_service_mappings() {
        let hardware = parse_macos_hardware_ports(
            "Hardware Port: Wi-Fi\nDevice: en0\n\nHardware Port: Ethernet Adapter\nDevice: en4\n",
        );
        assert_eq!(hardware.get("en0").map(String::as_str), Some("Wi-Fi"));
        assert_eq!(
            hardware.get("en4").map(String::as_str),
            Some("Ethernet Adapter")
        );
        assert_eq!(
            macos_interface_type("en0", hardware.get("en0").map(String::as_str)),
            NetworkInterfaceType::Wifi
        );

        let services = parse_macos_network_services(
            "(1) Wi-Fi\n(Hardware Port: Wi-Fi, Device: en0)\n(2) *Thunderbolt Bridge\n(Hardware Port: Thunderbolt Bridge, Device: bridge0)\n",
        );
        assert_eq!(services.get("en0").map(String::as_str), Some("Wi-Fi"));
        assert_eq!(
            services.get("bridge0").map(String::as_str),
            Some("Thunderbolt Bridge")
        );
    }

    #[tokio::test]
    async fn lists_macos_interfaces_from_host() {
        let interfaces = list_interfaces_macos()
            .await
            .expect("macOS ifconfig 网卡枚举失败");
        assert!(interfaces.iter().any(|interface| interface.name == "lo0"));
    }
}
