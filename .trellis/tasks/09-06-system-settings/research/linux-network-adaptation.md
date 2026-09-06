# Linux 网络管理服务适配技术调研

状态：已完成调研，可作为 design.md 输入。

## 调研目标

明确后端如何统一查询和修改边缘设备网卡 IP，覆盖 NetworkManager、systemd-networkd 及无管理服务的裸接口场景，保证配置重启后持久生效。

## Linux 网络管理生态分层

```
┌─────────────────────────────────────────────┐
│  Heimdall 后端 (统一 API)                     │
├─────────────────────────────────────────────┤
│  适配层: NetworkManager / systemd-networkd /  │
│          iproute2(fallback) / netplan(透传)    │
├─────────────────────────────────────────────┤
│  内核: rtnetlink, netns                       │
└─────────────────────────────────────────────┘
```

### 网卡管理服务识别矩阵

| 管理服务 | 检测方式 | 配置源路径 | 持久化机制 | 典型发行版 |
|---------|---------|-----------|-----------|-----------|
| **NetworkManager** | `NetworkManager.service` active + `nmcli general` 可用 | `/etc/NetworkManager/system-connections/` | NM connection profiles + dispatcher | Ubuntu Desktop/Server, CentOS, RHEL, Rocky, Alma, openSUSE |
| **systemd-networkd** | `systemd-networkd.service` active + 网卡在 `.network` 文件 Match 范围内 | `/etc/systemd/network/`, `/run/systemd/network/` | systemd-networkd `.network` 文件 | Ubuntu Server (netplan 底层), Arch, NixOS, 容器/嵌入式 |
| **Netplan** | `/etc/netplan/*.yaml` 存在且 `netplan generate` 可用 | `/etc/netplan/*.yaml` | 生成 NM 或 networkd 配置，自身不管理运行时 | Ubuntu 18.04+ 默认 |
| **ifupdown** | `/etc/network/interfaces` 存在 + `ifup`/`ifdown` 可用 | `/etc/network/interfaces` | 直接编辑文件 | Debian legacy, 旧版 Ubuntu |
| **connman** | `connman.service` active | D-Bus 接口 | 内置持久化 | 树莓派 OS, 部分嵌入式 |
| **无管理服务** | 以上均不匹配 | 无 | 需自行写入 netplan/NM 或 networkd 配置 | 嵌入式定制系统 |

### 识别算法（后端实现逻辑）

```
fn detect_interface_manager(iface: &str) -> InterfaceManager {
    // 1. 检查 NetworkManager
    if service_active("NetworkManager") && nmcli_iface_managed(iface) {
        return NetworkManager;
    }
    // 2. 检查 systemd-networkd
    if service_active("systemd-networkd") && networkd_has_match(iface) {
        return SystemdNetworkd;
    }
    // 3. 检查 netplan (仅 Ubuntu)
    if netplan_available() && netplan_manages(iface) {
        return Netplan;  // 底层透传到 NM 或 networkd
    }
    // 4. 检查 ifupdown
    if command_available("ifup") && interfaces_file_has(iface) {
        return Ifupdown;
    }
    // 5. 检查 connman
    if service_active("connman") {
        return Connman;
    }
    // 6. 无管理服务
    return Unmanaged;
}
```

**关键原则**：
- 不以发行版名称判断，而是检测实际运行的服务和配置文件
- "nmcli 可用"不等于"NM 管理此网卡"，需验证 `nmcli device status` 中该网卡的 MANAGED 状态
- Netplan 是声明式配置生成层，最终由 NM 或 networkd 执行；修改 netplan 配置后需 `netplan apply`

## 各管理服务的读写能力

### NetworkManager

| 操作 | 命令/API | 持久化 | 备注 |
|------|---------|--------|------|
| 读取网卡列表 | `nmcli -t -f DEVICE,TYPE,STATE,CONNECTION device status` | - | 包含连接名和状态 |
| 读取 IPv4 配置 | `nmcli -f IP4.ADDRESS,IP4.GATEWAY,IP4.DNS connection show <name>` | - | 当前生效值 |
| 读取 DHCP 状态 | `nmcli -f IPV4.METHOD connection show <name>` | - | auto=DHCP, manual=静态 |
| 设置静态 IP | `nmcli connection modify <name> ipv4.addresses <ip/prefix> ipv4.gateway <gw> ipv4.dns <dns> ipv4.method manual` | 写入 `/etc/NetworkManager/system-connections/` | 需 `nmcli connection up` 生效 |
| 切换 DHCP | `nmcli connection modify <name> ipv4.method auto` | 同上 | 清除静态配置 |
| 应用配置 | `nmcli connection up <name>` | - | 会短暂中断连接 |
| 备份当前配置 | `nmcli -g connection.name,ipv4.addresses,ipv4.gateway,ipv4.dns,ipv4.method connection show <name>` | - | 用于恢复 |

**NM 优势**：连接配置文件是 INI 格式，支持原子替换；`nmcli connection clone` 可创建备份；`nmcli connection up/down` 有完善的错误处理。

### systemd-networkd

| 操作 | 命令/API | 持久化 | 备注 |
|------|---------|--------|------|
| 读取网卡状态 | `networkctl status <iface>` | - | 显示管理状态和地址 |
| 读取当前 IP | `ip -4 addr show dev <iface>` | - | 内核层面实际地址 |
| 设置静态 IP | 写入 `.network` 文件 `[Network]` + `[Address]` + `[Route]` 段 | `/etc/systemd/network/<name>.network` | 需 `networkctl reconfigure <iface>` |
| 切换 DHCP | `.network` 文件中 `DHCP=yes` | 同上 | |
| 应用配置 | `networkctl reconfigure <iface>` | - | 比 NM 风险稍高，需确保有控制台访问 |

**注意**：systemd-networkd 没有 `nmcli connection clone` 等价物；备份需手动复制 `.network` 文件。

### Netplan (Ubuntu)

Netplan 是声明式 YAML 配置层，生成 NM 或 networkd 配置：

```yaml
# /etc/netplan/01-config.yaml
network:
  version: 2
  renderer: networkd  # 或 nm
  ethernets:
    eth0:
      dhcp4: false
      addresses:
        - 192.168.1.100/24
      gateway4: 192.168.1.1
      nameservers:
        addresses: [8.8.8.8, 8.8.4.4]
```

- 修改后执行 `netplan generate` + `netplan apply`
- 底层由 NM 或 networkd 执行，需识别 renderer
- 配置文件有严格的 YAML schema 校验

### 无管理服务 (Unmanaged)

对没有网络管理服务的网卡：
- 可通过 `ip addr add/del` 临时修改内核地址
- 持久化需写入某个配置文件（如 `/etc/network/interfaces` 或创建 NM connection）
- **风险**：接管网卡生命周期，Heimdall 需承担 DHCP 续租、DNS 管理等职责
- **建议**：首版不支持自行接管，只读展示并提示用户手动配置

## 首版适配范围

| 管理服务 | 读取 | 修改 | 持久化 | 应用 | 恢复 | 优先级 |
|---------|------|------|--------|------|------|--------|
| NetworkManager | ✅ | ✅ | ✅ | ✅ | ✅ | P0 |
| systemd-networkd | ✅ | ✅ | ✅ | ✅ | ✅ | P0 |
| Netplan | ✅ | ✅(透传) | ✅ | ✅ | ✅ | P1 |
| ifupdown | ✅ | ⚠️ | ⚠️ | ⚠️ | ❌ | P2 |
| connman | ✅ | ❌ | ❌ | ❌ | ❌ | P2 |
| Unmanaged | ✅(只读) | ❌ | ❌ | ❌ | ❌ | - |

## 后端实现架构建议

```
crates/app/src/network/
├── mod.rs              // NetworkService trait + factory
├── nm.rs               // NetworkManager 适配器
├── networkd.rs         // systemd-networkd 适配器
├── netplan.rs          // Netplan 适配器 (透传)
├── types.rs            // NetworkInterface, IpConfig, etc.
└── fallback.rs         // iproute2 只读查询
```

**NetworkService trait**：
```rust
#[async_trait]
pub trait NetworkService: Send + Sync {
    /// 检测此服务是否管理指定网卡
    async fn manages(&self, iface: &str) -> bool;
    /// 读取网卡当前 IPv4 配置
    async fn get_ipv4_config(&self, iface: &str) -> Result<IpConfig>;
    /// 设置静态 IPv4 配置
    async fn set_static_ipv4(&self, iface: &str, config: &StaticIpConfig) -> Result<()>;
    /// 切换为 DHCP
    async fn set_dhcp(&self, iface: &str) -> Result<()>;
    /// 应用配置 (可能中断连接)
    async fn apply(&self, iface: &str) -> Result<()>;
    /// 备份当前配置
    async fn backup(&self, iface: &str) -> Result<ConfigBackup>;
    /// 从备份恢复配置
    async fn restore(&self, iface: &str, backup: &ConfigBackup) -> Result<()>;
}
```

## 权限与安全

- 所有网络修改操作需要 `root` 或 `CAP_NET_ADMIN` 权限
- Heimdall 以 systemd service 运行时，需在 service 文件中配置 `AmbientCapabilities=CAP_NET_ADMIN`
- `nmcli connection modify/up` 需要 polkit 授权或以 root 运行
- 建议：后端网络操作封装为独立进程或 sudo 调用，不长期持有高权限

## 重启生效验证策略

### 应用后即时验证

配置写入后必须验证实际生效，不能仅凭命令退出码判断成功：

```
执行 nmcli connection up / networkctl reconfigure
  ↓
等待 2-3 秒（给网络服务重配时间）
  ↓
ip -4 addr show dev <iface>  ← 读内核实际地址
  ↓
比对期望 IP vs 实际 IP
  ↓
一致 → 返回成功，持久化已确认
不一致 → 返回 51008 + 实际地址，保留备份供恢复
```

### 启动时校验

Heimdall 启动时检查 `system_configs` 中是否有未完成的网络操作：

| 状态 | 处理 |
|------|------|
| `pending_confirm` 且已超时 | 执行恢复，更新状态为 `restored` |
| `pending_confirm` 且未超时 | 保持状态，继续倒计时 |
| `confirmed` | 读取实际网卡状态，与操作记录比对；不匹配写入告警日志 |
| 无操作 | 正常启动 |

### 验证函数

```rust
async fn verify_ip_applied(
    iface: &str,
    expected: &IpConfig,
    timeout: Duration,
) -> Result<bool> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let actual = read_kernel_ip(iface).await?;
        if actual.address == expected.address
            && actual.prefix == expected.prefix
        {
            return Ok(true);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Ok(false)
}
```

- 超时默认 10 秒
- 验证失败不自动回滚（可能网络服务还在重配），返回失败状态由前端展示
- 管理接口变更场景：验证失败则中止切换，不执行断连

### 为什么不能跳过验证

- `nmcli connection up` 返回成功 ≠ IP 已在内核中生效（可能有异步延迟）
- `nmcli connection modify` 只修改配置文件 ≠ 当前连接已切换
- 配置写入 `/etc/NetworkManager/system-connections/` ≠ 重启后 NM 一定读取（文件权限、SELinux 等可能导致静默失败）
- 验证是唯一能确认"配置已写入 + 已生效 + 可被 NM 在重启后重新读取"的方式

## 测试方案

1. **单元测试**：mock NetworkService trait，验证表单校验、状态机流转
2. **集成测试**：在 Docker 容器中安装 NetworkManager，验证 nmcli 读写 + 重启后配置保持
3. **设备测试**：在目标 RK3568/RK3576 设备上验证实际网卡配置和重启保持
4. **恢复测试**：修改管理网卡 IP 后，验证旧连接断开、新地址可访问、超时恢复

## Open Questions

1. 目标设备预装的网络管理服务是什么？（NetworkManager vs systemd-networkd）
2. 是否需要支持多网卡同时配置？
3. DHCP 模式下如何报告实际分配的 IP？（需等待 DHCP 租约完成）
4. IPv6 支持范围？首版是否只做 IPv4？
