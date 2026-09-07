//! macOS 开发环境适配（只读回退）

#[cfg(target_os = "macos")]
use std::collections::HashMap;
#[cfg(target_os = "macos")]
use std::net::Ipv4Addr;

use super::detector::run_command_with_c_locale;
use crate::error::ApiError;
use types::system::{
    InterfaceCapabilities, IpConfig, IpMethod, NetworkInterface, NetworkInterfaceState,
    NetworkInterfaceType, NetworkManager,
};

#[cfg(target_os = "macos")]
pub async fn list_interfaces_macos() -> Result<Vec<NetworkInterface>, ApiError> {
    let ifconfig_output = run_command_with_c_locale("ifconfig", &["-a"]).await?;
    let hardware_ports = run_command_with_c_locale("networksetup", &["-listallhardwareports"])
        .await
        .unwrap_or_default();
    let network_services = run_command_with_c_locale("networksetup", &["-listnetworkserviceorder"])
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
            carrier: None,
            speed: None,
            duplex: None,
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
        let trimmed = line.trim();

        if let Some(value) = trimmed.strip_prefix("ether ") {
            let mac = value.split_whitespace().next().unwrap_or_default();
            if !mac.is_empty() {
                record.mac = Some(mac.to_uppercase());
            }
        } else if let Some(value) = trimmed.strip_prefix("inet ") {
            let mut parts = value.split_whitespace();
            let address = parts.next().map(str::to_string);
            let mut prefix = None;

            while let Some(part) = parts.next() {
                if part == "netmask" {
                    if let Some(netmask) = parts.next() {
                        prefix = parse_macos_netmask(netmask);
                    }
                    break;
                }
            }

            if let Some(address) = address {
                record.ipv4 = Some(MacIpv4 { address, prefix });
            }
        } else if let Some(value) = trimmed.strip_prefix("status: ") {
            record.status_active = Some(value.eq_ignore_ascii_case("active"));
        }
    }

    if let Some(record) = current.take() {
        records.push(record);
    }

    records
}

#[cfg(target_os = "macos")]
fn macos_interface_type(name: &str, hardware_port: Option<&str>) -> NetworkInterfaceType {
    if name == "lo0" || name.starts_with("lo") {
        return NetworkInterfaceType::Loopback;
    }

    if let Some(port) = hardware_port {
        let lower = port.to_ascii_lowercase();
        if lower.contains("wi-fi") || lower.contains("wlan") {
            return NetworkInterfaceType::Wifi;
        }
        if lower.contains("ethernet") || lower.contains("lan") {
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
    let has_up_flag = record
        .flags
        .split(',')
        .any(|flag| flag.eq_ignore_ascii_case("UP"));

    match (has_up_flag, record.status_active) {
        (true, Some(false)) => NetworkInterfaceState::Down,
        (true, _) => NetworkInterfaceState::Up,
        (false, _) => NetworkInterfaceState::Down,
    }
}

#[cfg(target_os = "macos")]
fn observed_ip_config(observed: &MacIpv4) -> IpConfig {
    IpConfig {
        method: IpMethod::Static,
        address: Some(observed.address.clone()),
        prefix: observed.prefix,
        gateway: None,
        dns: Vec::new(),
        metric: None,
    }
}

#[cfg(target_os = "macos")]
async fn get_macos_ipv4_config(service_name: &str, observed: &MacIpv4) -> IpConfig {
    let mut config = observed_ip_config(observed);
    let info = run_command_c("networksetup", &["-getinfo", service_name])
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

    if let Ok(dns_output) = run_command_c("networksetup", &["-getdnsservers", service_name]).await {
        config.dns = dns_output
            .lines()
            .map(str::trim)
            .filter(|value| value.parse::<std::net::IpAddr>().is_ok())
            .map(str::to_string)
            .collect();
    }

    config
}

#[cfg(target_os = "macos")]
fn parse_macos_netmask(netmask: &str) -> Option<u32> {
    if let Some(hex) = netmask.strip_prefix("0x") {
        let raw = u32::from_str_radix(hex, 16).ok()?;
        return Some(raw.count_ones());
    }

    let parsed = netmask.parse::<Ipv4Addr>().ok()?;
    Some(u32::from(parsed).count_ones())
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
