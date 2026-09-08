//! 工业级边缘网络服务门面
//!
//! 具备：
//! 1. RFC 5227 地址冲突检测 (ACD)
//! 2. Commit-Confirm 事务与 60s 独立看门狗防失联回滚
//! 3. 物理载波检测与管理口动态推导
//! 4. 掉电安全原子写入与 LC_ALL=C 国际化强隔离

pub mod arp;
pub mod backend;
pub mod detector;
pub mod operation;
pub mod route;

#[cfg(target_os = "macos")]
pub mod macos;

use crate::error::ApiError;
use std::sync::LazyLock;
use types::system::{
    IpConfig, IpMethod, NetworkChangeOperation, NetworkInterface, NetworkInterfacesResponse,
    NetworkManager, NetworkUpdateResult, OperationConfirmResult,
};

use backend::networkd::update_interface_networkd;
use backend::nm::update_interface_nm;
#[cfg(not(target_os = "macos"))]
use backend::{networkd::list_interfaces_networkd, nm::list_interfaces_nm};
use operation::NetworkOperationManager;

/// 全局事务与看门狗管理器
pub static OPERATION_MANAGER: LazyLock<NetworkOperationManager> =
    LazyLock::new(NetworkOperationManager::new);

#[derive(Debug)]
pub struct NetworkService;

impl NetworkService {
    /// 枚举所有物理与逻辑网卡
    pub async fn list_interfaces() -> Result<Vec<NetworkInterface>, ApiError> {
        #[cfg(target_os = "macos")]
        {
            macos::list_interfaces_macos().await
        }

        #[cfg(not(target_os = "macos"))]
        {
            let manager = detect_network_manager().await;

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

    /// 列出网卡并附带当前未决的试运行操作
    pub async fn list_with_pending() -> Result<NetworkInterfacesResponse, ApiError> {
        let interfaces = Self::list_interfaces().await?;
        let pending_operation = OPERATION_MANAGER.get_pending().await;
        Ok(NetworkInterfacesResponse {
            interfaces,
            pending_operation,
        })
    }

    /// 工业级修改网卡配置（集成预检、冲突检测与防失联看门狗）
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

        let mut planned_config = config.clone();
        let is_mgmt = iface.capabilities.is_management_interface;

        // 静态 IP 预检与冲突检测 (RFC 5227)
        if config.method == IpMethod::Static {
            let addr_str = config.address.as_deref().unwrap_or("");
            let prefix = config.prefix.unwrap_or(24);

            // 1. 校验网关子网可达性
            if let Some(gw) = &config.gateway {
                if !gw.is_empty() {
                    route::validate_gateway_reachable(addr_str, prefix, gw)?;
                }
            }

            // 2. 规划 Metric 优先级
            planned_config.metric = Some(route::plan_interface_metric(is_mgmt, config.metric));

            // 3. RFC 5227 ARP 冲突预检（仅在不同于当前 IP 时探测）
            let need_probe =
                iface.ipv4.as_ref().and_then(|cur| cur.address.as_deref()) != Some(addr_str);

            if need_probe {
                if let Some(conflict_mac) = arp::probe_ipv4_conflict(name, addr_str).await? {
                    return Err(ApiError::NetworkIpConflict(format!(
                        "目标 IP {addr_str} 已被局域网主机 [{conflict_mac}] 占用，禁止绑定"
                    )));
                }
            }
        }

        let manager = detect_network_manager().await;
        let old_config = iface.ipv4.unwrap_or_default();

        // 分流：若是管理网卡，必须开启 Commit-Confirm 试运行机制与独立看门狗！
        if is_mgmt {
            let manager_str = match manager {
                NetworkManager::Networkmanager => "networkmanager",
                NetworkManager::SystemdNetworkd => "systemd-networkd",
                _ => "unmanaged",
            };

            let new_url = planned_config
                .address
                .as_ref()
                .map(|addr| format!("http://{addr}:8000"));

            // 启动试运行事务
            let op = OPERATION_MANAGER
                .start_trial(
                    name,
                    old_config,
                    planned_config.clone(),
                    new_url,
                    manager_str,
                    |iface_name, rollback_cfg| async move {
                        tracing::warn!("看门狗触发：恢复网卡 {iface_name} 配置...");
                        apply_backend_config(&iface_name, &rollback_cfg)
                            .await
                            .map(|_| ())
                    },
                )
                .await?;

            // 下发试运行配置
            if let Err(e) = apply_backend_config(name, &planned_config).await {
                tracing::warn!("试运行配置下发失败，立即清理看门狗与快照: {e}");
                OPERATION_MANAGER.abort_trial(&op.id).await;
                return Err(e);
            }

            return Ok(NetworkUpdateResult {
                applied: false,
                operation: Some(op),
            });
        }

        // 非管理网卡直接应用
        let result = apply_backend_config(name, &planned_config).await?;

        // 成功后向局域网广播免费 ARP 刷新交换机
        if let Some(addr) = &planned_config.address {
            arp::announce_gratuitous_arp(name, addr).await;
        }

        Ok(result)
    }

    /// 查询当前试运行操作
    pub async fn get_pending_operation() -> Result<Option<NetworkChangeOperation>, ApiError> {
        Ok(OPERATION_MANAGER.get_pending().await)
    }

    /// 确认试运行网络操作
    pub async fn confirm_operation(id: &str) -> Result<OperationConfirmResult, ApiError> {
        OPERATION_MANAGER.confirm(id).await
    }

    /// 放弃试运行网络操作并立即回滚
    pub async fn cancel_operation(id: &str) -> Result<OperationConfirmResult, ApiError> {
        OPERATION_MANAGER
            .cancel(id, |iface_name, old_cfg| async move {
                apply_backend_config(&iface_name, &old_cfg)
                    .await
                    .map(|_| ())
            })
            .await
    }

    /// 连通性自检诊断 (ICMP Ping / DNS 解析)
    pub async fn diagnose(
        req: &types::system::NetworkDiagnosticRequest,
    ) -> Result<types::system::NetworkDiagnosticResult, ApiError> {
        match req.diagnostic_type {
            types::system::NetworkDiagnosticType::Ping => {
                let mut cmd = tokio::process::Command::new("ping");
                cmd.args(["-c", "2", "-W", "2"]);
                if let Some(iface) = &req.interface {
                    if !iface.is_empty() {
                        cmd.args(["-I", iface]);
                    }
                }
                cmd.arg(&req.target);
                cmd.env("LC_ALL", "C").env("LANG", "C");

                let start = std::time::Instant::now();
                match cmd.output().await {
                    Ok(out) => {
                        let duration = start.elapsed().as_secs_f64() * 1000.0;
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        let success = out.status.success();

                        let latency = parse_ping_latency(&stdout).unwrap_or(duration);

                        let message = if success {
                            format!("Ping {} 成功: 往返延迟 {:.1} ms", req.target, latency)
                        } else {
                            format!("Ping {} 失败: {}", req.target, stderr.trim())
                        };

                        Ok(types::system::NetworkDiagnosticResult {
                            success,
                            latency_ms: if success { Some(latency) } else { None },
                            message,
                        })
                    }
                    Err(e) => Ok(types::system::NetworkDiagnosticResult {
                        success: false,
                        latency_ms: None,
                        message: format!("执行 Ping 命令失败: {e}"),
                    }),
                }
            }
            types::system::NetworkDiagnosticType::Dns => {
                let start = std::time::Instant::now();
                let host_str = if req.target.contains(':') {
                    req.target.clone()
                } else {
                    format!("{}:80", req.target)
                };

                match tokio::net::lookup_host(host_str).await {
                    Ok(mut addrs) => {
                        let duration = start.elapsed().as_secs_f64() * 1000.0;
                        if let Some(first) = addrs.next() {
                            Ok(types::system::NetworkDiagnosticResult {
                                success: true,
                                latency_ms: Some(duration),
                                message: format!(
                                    "DNS 解析成功: {} -> {} (耗时 {:.1} ms)",
                                    req.target,
                                    first.ip(),
                                    duration
                                ),
                            })
                        } else {
                            Ok(types::system::NetworkDiagnosticResult {
                                success: false,
                                latency_ms: None,
                                message: format!("DNS 解析返回空记录: {}", req.target),
                            })
                        }
                    }
                    Err(e) => Ok(types::system::NetworkDiagnosticResult {
                        success: false,
                        latency_ms: None,
                        message: format!("DNS 解析失败: {e}"),
                    }),
                }
            }
        }
    }

    /// 系统开机冷启动自动恢复：扫描掉电或重启前残留的未决快照并自动回滚
    pub async fn recover_pending_snapshots_on_startup() {
        let snapshots = operation::load_pending_snapshots().await;
        for snap in snapshots {
            let iface = snap.operation.interface_name.clone();
            tracing::warn!(
                "【防失联自愈】检测到未确认的网络快照 (网卡: {iface}, 创建时间: {})，正在执行开机无条件自愈回滚...",
                snap.operation.created_at
            );
            if let Err(e) = apply_backend_config(&iface, &snap.operation.old_config).await {
                tracing::error!("【防失联自愈】网卡 {iface} 自愈回滚失败: {e}");
            } else {
                tracing::info!("【防失联自愈】网卡 {iface} 已成功恢复至旧配置！");
                operation::remove_snapshot(&iface).await;
            }
        }
    }
}

/// 底层驱动应用分发
async fn apply_backend_config(
    name: &str,
    config: &IpConfig,
) -> Result<NetworkUpdateResult, ApiError> {
    let manager = detect_network_manager().await;
    let res = match manager {
        NetworkManager::Networkmanager => update_interface_nm(name, config).await,
        NetworkManager::SystemdNetworkd => update_interface_networkd(name, config).await,
        _ => Err(ApiError::NetworkInvalid("不支持的网络管理服务".to_string())),
    }?;

    // 针对静态配置下发策略路由 (PBR)，保证多网卡场景流量原路返回
    if config.method == IpMethod::Static {
        if let Some(ip) = &config.address {
            route::configure_policy_routing(name, ip, config.gateway.as_deref()).await;
        }
    }

    Ok(res)
}

/// 解析 ping 命令输出中的平均往返延迟 (ms)
fn parse_ping_latency(output: &str) -> Option<f64> {
    for line in output.lines() {
        if line.contains("min/avg/max") || line.contains("rtt ") || line.contains("round-trip") {
            if let Some((_, stats)) = line.split_once('=') {
                let mut parts = stats.trim().split('/');
                let _min = parts.next();
                return parts.next()?.trim().parse::<f64>().ok();
            }
        }
    }
    None
}

/// 识别宿主系统网络管理守护进程 (异步执行外部检查，不阻塞 Tokio 运行时)
pub async fn detect_network_manager() -> NetworkManager {
    #[cfg(target_os = "macos")]
    {
        NetworkManager::Unmanaged
    }

    #[cfg(not(target_os = "macos"))]
    {
        // 优先检查 NetworkManager
        if tokio::process::Command::new("nmcli")
            .args(["general", "status"])
            .env("LC_ALL", "C")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return NetworkManager::Networkmanager;
        }

        // 其次检查 systemd-networkd
        if tokio::process::Command::new("networkctl")
            .args(["status"])
            .env("LC_ALL", "C")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return NetworkManager::SystemdNetworkd;
        }

        NetworkManager::Unmanaged
    }
}

/// 校验网卡名称
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

/// 校验 IP 配置参数
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ping_latency() {
        let sample = "rtt min/avg/max/mdev = 1.234/2.345/3.456/0.123 ms";
        assert_eq!(parse_ping_latency(sample), Some(2.345));

        let sample2 = "round-trip min/avg/max/stddev = 0.500/1.500/2.500/0.100 ms";
        assert_eq!(parse_ping_latency(sample2), Some(1.500));
    }

    #[test]
    fn test_validate_interface_name() {
        assert!(validate_interface_name("eth0").is_ok());
        assert!(validate_interface_name("enp3s0").is_ok());
        assert!(validate_interface_name("eth0.100").is_ok());
        assert!(validate_interface_name("").is_err());
        assert!(validate_interface_name("eth0; rm -rf").is_err());
        assert!(validate_interface_name("eth..0").is_err());
    }

    #[tokio::test]
    async fn test_diagnose_dns() {
        let req = types::system::NetworkDiagnosticRequest {
            target: "localhost".to_string(),
            diagnostic_type: types::system::NetworkDiagnosticType::Dns,
            interface: None,
        };
        let res = NetworkService::diagnose(&req)
            .await
            .expect("DNS 诊断应执行");
        assert!(res.success);
    }
}
