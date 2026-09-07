//! 工业多网口策略路由与 Metric 规划器

use crate::error::ApiError;
use std::net::Ipv4Addr;

/// 校验 IP 与网关的子网从属关系，防止非法不可达网关
pub fn validate_gateway_reachable(ip: &str, prefix: u32, gateway: &str) -> Result<(), ApiError> {
    let ip_addr: Ipv4Addr = ip
        .parse()
        .map_err(|e| ApiError::NetworkInvalid(format!("非法 IP: {e}")))?;
    let gw_addr: Ipv4Addr = gateway
        .parse()
        .map_err(|e| ApiError::NetworkInvalid(format!("非法网关: {e}")))?;

    if prefix > 32 {
        return Err(ApiError::NetworkInvalid("子网前缀不能大于 32".into()));
    }

    let mask = if prefix == 0 {
        0
    } else {
        !0u32 << (32 - prefix)
    };

    let ip_u32 = u32::from(ip_addr);
    let gw_u32 = u32::from(gw_addr);

    if (ip_u32 & mask) != (gw_u32 & mask) {
        return Err(ApiError::NetworkGatewayUnreachable(format!(
            "网关 {gateway} 与当前 IP {ip}/{prefix} 不在同一局域网网段"
        )));
    }

    Ok(())
}

/// 规划与校验网卡路由优先级 (Metric)
///
/// 工业原则：
/// - 管理/上行主网口分配较低 Metric（如 100），享有最高外网优先权；
/// - 从属/专网网口若配置网关，分配较高 Metric（如 500-1000），避免劫持主默认路由。
pub fn plan_interface_metric(is_mgmt: bool, user_metric: Option<u32>) -> u32 {
    if let Some(m) = user_metric {
        return m;
    }
    if is_mgmt {
        100
    } else {
        500
    }
}

/// 计算网卡对应的策略路由表 ID (100..=199)
pub fn get_table_id_for_iface(iface: &str) -> u32 {
    let sum: u32 = iface.bytes().map(|b| b as u32).sum();
    100 + (sum % 100)
}

/// 配置基于源地址的策略路由 (Policy-Based Routing)，保证“从哪个网口进来的流量，响应严格从哪个网口返回”
pub async fn configure_policy_routing(iface: &str, ip: &str, gateway: Option<&str>) {
    #[cfg(target_os = "linux")]
    {
        let table_id = get_table_id_for_iface(iface).to_string();
        let table_str = table_id.as_str();

        // 1. 先清理针对该源 IP 的旧 ip rule
        let _ = tokio::process::Command::new("ip")
            .args(["rule", "del", "from", ip, "table", table_str])
            .env("LC_ALL", "C")
            .output()
            .await;

        // 2. 添加基于源 IP 的路由规则：ip rule add from <ip> table <table_id> priority 30000
        let add_res = tokio::process::Command::new("ip")
            .args([
                "rule", "add", "from", ip, "table", table_str, "priority", "30000",
            ])
            .env("LC_ALL", "C")
            .output()
            .await;

        if let Ok(out) = add_res {
            if !out.status.success() {
                tracing::debug!(
                    "添加 ip rule 提示: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
        }

        // 3. 如果该网卡配置了默认网关，向对应独立路由表写入默认路由
        if let Some(gw) = gateway {
            if !gw.is_empty() {
                let route_res = tokio::process::Command::new("ip")
                    .args([
                        "route", "replace", "default", "via", gw, "dev", iface, "table", table_str,
                    ])
                    .env("LC_ALL", "C")
                    .output()
                    .await;
                if let Ok(out) = route_res {
                    if !out.status.success() {
                        tracing::debug!(
                            "添加独立路由表默认网关提示: {}",
                            String::from_utf8_lossy(&out.stderr)
                        );
                    }
                }
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (iface, ip, gateway);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gateway_reachable() {
        // 同网段
        assert!(validate_gateway_reachable("192.168.1.100", 24, "192.168.1.1").is_ok());
        assert!(validate_gateway_reachable("10.0.5.20", 16, "10.0.0.1").is_ok());

        // 跨网段
        assert!(validate_gateway_reachable("192.168.1.100", 24, "192.168.2.1").is_err());
    }

    #[test]
    fn test_plan_interface_metric() {
        assert_eq!(plan_interface_metric(true, None), 100);
        assert_eq!(plan_interface_metric(false, None), 500);
        assert_eq!(plan_interface_metric(true, Some(200)), 200);
        assert_eq!(plan_interface_metric(false, Some(300)), 300);
    }

    #[test]
    fn test_get_table_id_for_iface() {
        let t1 = get_table_id_for_iface("eth0");
        let t2 = get_table_id_for_iface("eth1");
        assert!((100..=199).contains(&t1));
        assert!((100..=199).contains(&t2));
    }
}
