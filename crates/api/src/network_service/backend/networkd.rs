//! systemd-networkd 底层驱动适配 (掉电原子持久化 + 精准验证)

use super::nm::verify_ip_applied;
use crate::error::ApiError;
use crate::network_service::detector::{
    detect_carrier, detect_default_route_interfaces, detect_link_speed_and_duplex, get_mac_address,
    is_management_interface_with_defaults, is_virtual_interface, run_command_with_c_locale,
};
use tokio::io::AsyncWriteExt;
use types::system::{
    InterfaceCapabilities, IpConfig, IpMethod, NetworkInterface, NetworkInterfaceState,
    NetworkInterfaceType, NetworkManager, NetworkUpdateResult,
};

/// 枚举由 systemd-networkd 管理的所有网卡
pub async fn list_interfaces_networkd() -> Result<Vec<NetworkInterface>, ApiError> {
    let output = run_command_with_c_locale("networkctl", &["status", "--no-pager"]).await?;
    let default_route_ifaces = detect_default_route_interfaces().await;

    let mut interfaces = Vec::new();
    for line in output.lines() {
        if let Some(rest) = line.trim().strip_prefix(|c: char| c.is_ascii_digit()) {
            let rest = rest.trim_start_matches([':', ' ']);
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 2 {
                let name = parts[0].to_string();

                // 过滤回环、网桥、虚拟接口及容器虚拟网卡
                if is_virtual_interface(&name) {
                    continue;
                }

                let state = if line.contains("routable") || line.contains("configured") {
                    NetworkInterfaceState::Up
                } else {
                    NetworkInterfaceState::Down
                };

                let mac = get_mac_address(&name).await.unwrap_or_default();
                let ipv4 = get_ipv4_config_networkd(&name).await.ok().flatten();
                let is_mgmt =
                    is_management_interface_with_defaults(&name, &default_route_ifaces).await;
                let carrier = detect_carrier(&name).await;
                let (speed, duplex) = detect_link_speed_and_duplex(&name).await;

                interfaces.push(NetworkInterface {
                    name: name.clone(),
                    interface_type: NetworkInterfaceType::Ethernet,
                    state,
                    carrier,
                    speed,
                    duplex,
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

/// 获取 networkd 接口的 IPv4 配置
pub async fn get_ipv4_config_networkd(iface: &str) -> Result<Option<IpConfig>, ApiError> {
    let network_dir = "/etc/systemd/network";
    if let Ok(mut entries) = tokio::fs::read_dir(network_dir).await {
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

    // 从 ip addr 读取当前生效配置
    let output = tokio::process::Command::new("ip")
        .args(["-4", "-o", "addr", "show", iface])
        .env("LC_ALL", "C")
        .output()
        .await
        .map_err(|e| ApiError::NetworkFailed(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("inet ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            for pair in parts.windows(2) {
                if pair[0] == "inet" {
                    if let Some((addr, pfx)) = pair[1].split_once('/') {
                        return Ok(Some(IpConfig {
                            method: IpMethod::Static,
                            address: Some(addr.to_string()),
                            prefix: pfx.parse().ok(),
                            gateway: None,
                            dns: Vec::new(),
                            metric: None,
                        }));
                    }
                }
            }
        }
    }

    Ok(None)
}

fn parse_network_file(content: &str) -> Result<Option<IpConfig>, ApiError> {
    let mut address = None;
    let mut prefix = None;
    let mut gateway = None;
    let mut dns = Vec::new();
    let mut metric = None;
    let mut is_dhcp = false;

    for line in content.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("DHCP=") {
            is_dhcp = val.eq_ignore_ascii_case("yes") || val.eq_ignore_ascii_case("ipv4");
        } else if let Some(val) = line.strip_prefix("Address=") {
            if let Some((addr, pfx)) = val.split_once('/') {
                address = Some(addr.to_string());
                prefix = pfx.parse().ok();
            }
        } else if let Some(val) = line.strip_prefix("Gateway=") {
            gateway = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("DNS=") {
            dns.push(val.to_string());
        } else if let Some(val) = line.strip_prefix("Metric=") {
            metric = val.parse::<u32>().ok();
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
        metric,
    }))
}

/// 掉电原子安全应用配置至 systemd-networkd (write .tmp -> sync_all -> rename -> .bak)
pub async fn update_interface_networkd(
    name: &str,
    config: &IpConfig,
) -> Result<NetworkUpdateResult, ApiError> {
    let network_file = format!("/etc/systemd/network/10-{name}.network");
    let tmp_file = format!("{network_file}.tmp.{}", std::process::id());
    let bak_file = format!("{network_file}.bak");

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
        if let Some(metric) = config.metric {
            content.push_str(&format!("\n[Route]\nMetric={metric}\n"));
        }
    }

    // 1. 如果已有原文件，保留 .bak 备份
    if tokio::fs::metadata(&network_file).await.is_ok() {
        let _ = tokio::fs::copy(&network_file, &bak_file).await;
    }

    // 2. 写入临时文件并强刷物理介质 (fsync)
    let mut file = tokio::fs::File::create(&tmp_file)
        .await
        .map_err(|e| ApiError::NetworkFailed(format!("创建临时配置文件失败: {e}")))?;
    file.write_all(content.as_bytes())
        .await
        .map_err(|e| ApiError::NetworkFailed(format!("写入临时配置文件失败: {e}")))?;
    file.sync_all()
        .await
        .map_err(|e| ApiError::NetworkFailed(format!("刷盘临时配置文件失败: {e}")))?;
    drop(file);

    // 3. 原子的 rename 覆盖
    tokio::fs::rename(&tmp_file, &network_file)
        .await
        .map_err(|e| ApiError::NetworkFailed(format!("原子替换配置文件失败: {e}")))?;

    // 4. 重载网卡
    run_command_with_c_locale("networkctl", &["reconfigure", name]).await?;

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    let verified = verify_ip_applied(name, config).await;

    Ok(NetworkUpdateResult {
        applied: verified,
        operation: None,
    })
}
