# 工业级网络服务配置 — 技术设计

## 1. 系统目标与架构分层

本设计旨在将 Heimdall 的网络服务从“原型级 CLI 包装器”升级为符合**工业边缘与安防设备标准（海康/大华 NVR、研华工控机）的鲁棒网络管理子系统**。

### 1.1 代码边界与模块划分

```
crates/
├── types/src/system.rs             # 共享领域模型（扩展冲突报告、网卡链路信息）
├── api/
│   ├── src/error.rs                # 新增网络错误码（51011 IP冲突, 51012 网关不可达）
│   ├── src/routes/system.rs        # HTTP Handler 薄层（路由分发、参数提取）
│   └── src/network_service/        # 工业级网络核心子系统（由单文件重构成模块）
│       ├── mod.rs                  # 对外统一服务门面 (NetworkService)
│       ├── operation.rs            # Commit-Confirm 事务与独立看门狗管理器
│       ├── arp.rs                  # RFC 5227 地址冲突检测 (ACD) 与 Gratuitous ARP
│       ├── route.rs                # 双网卡拓扑、Metric 分配与策略路由校验
│       ├── detector.rs             # 精准管理口判定与物理载波/链路状态读取
│       └── backend/
│           ├── mod.rs              # 驱动抽象接口 (NetworkBackend trait)
│           ├── nm.rs               # NetworkManager 驱动实现 (强制 LC_ALL=C)
│           └── networkd.rs         # systemd-networkd 驱动实现 (原子文件写入)
web/
├── src/types/system.ts             # 前端类型契约同步
├── src/lib/system-api.ts           # 前端 API 封装
└── src/features/system/
    ├── NetworkSettings.tsx         # 网卡列表与配置主界面
    ├── components/
    │   ├── NetworkCard.tsx         # 单网卡卡片（含物理链路状态与载波显示）
    │   ├── NetworkTrialBanner.tsx  # 试运行倒计时悬浮横幅（防失联）
    │   └── NetworkConflictModal.tsx# IP 冲突告警弹窗（展示冲突 MAC）
    └── hooks/
        └── use-network-trial.ts    # 试运行状态轮询与心跳维护 Hook
```

---

## 2. 核心机制详细设计

### 2.1 Commit-Confirm 事务与看门狗自动回滚（Fail-Safe Watchdog）

#### 状态机模型

```
           [下发变更]
[Idle] ─────────────────> [PendingConfirm (试运行中)]
                               │          │
                     确认调用  │          │ 超时未确认 (60s) 或
                    (confirm)  │          │ 主动取消 (cancel)
                               ▼          ▼
                          [Confirmed]  [Restoring -> Restored]
```

#### 工作流设计：
1. **生成原子快照（Snapshot）**：
   - 变更前，读取当前接口全部网络参数，持久化写入 `/run/heimdall/network_backup_<iface>.json`。
   - 记录原配置、新配置、创建时间戳与确认截止时间（`confirm_deadline_ms`，默认 +60,000 ms）。
2. **试运行应用（Trial Apply）**：
   - 写入新配置并重载网络。
   - 在内存中注册活跃事务，并启动独立 Tokio 后台任务（Watchdog）：
     ```rust
     tokio::spawn(async move {
         tokio::time::sleep_until(deadline).await;
         if operation_is_still_pending(&op_id).await {
             rollback_to_snapshot(&snapshot).await;
         }
     });
     ```
3. **确认与解除（Confirm）**：
   - 管理员通过新 IP 访问控制台（系统通过 `new_access_url` 引导）。
   - 登录成功后调用 `POST /api/v1/system/network/changes/:id/confirm`。
   - 后台看门狗被中止，快照安全归档，状态置为 `Confirmed`。
4. **超时自动回滚（Rollback）**：
   - 若 60 秒内未收到有效确认（如 IP 填错导致断网），看门狗立即调用驱动层的 `restore_snapshot(&snapshot)` 将原网络配置恢复，并执行网络栈重载，恢复原 IP。
5. **冷启动断电保护**：
   - 系统启动时检查 `/run/heimdall/network_backup_*.json`。若发现残留的未决快照且已过期，开机第一步无条件回滚，保证设备永不成砖。

---

### 2.2 RFC 5227 地址冲突检测（ACD）与 ARP Probe

在将静态 IP 生效前，必须确保局域网内无其他设备使用相同 IP。

#### 探测算法流程：
1. **ARP Probe 阶段**：
   - 构造 RFC 5227 标准 ARP 请求报文：
     - Sender IP: `0.0.0.0`（防止局域网设备提前污染自身 ARP 缓存）
     - Target IP: 目标静态 IP
     - Sender MAC: 当前物理网卡 MAC
     - Target MAC: `00:00:00:00:00:00`
   - 通过系统调用（或通过设置了 `LC_ALL=C` 的底层探测器，如 Linux RAW Socket 或 `arping -c 2 -w 1 -D -I <iface> <ip>`）：
     - 探测 2~3 次，间隔 500ms。
   - 若侦听到响应（收到 Target IP 的 Reply）：
     - 提取冲突方的 MAC 地址；
     - 终止下发，直接返回 `ApiError::NetworkIpConflict(format!("IP 地址 {} 已被局域网设备 [{}] 占用", ip, mac))`。
2. **Gratuitous ARP 宣告阶段**：
   - IP 成功绑定物理网卡后，向全网连续广播 2 次免费 ARP：
     - Sender IP = Target IP = 本机新 IP
     - Sender MAC = 本机 MAC
     - Target MAC = `FF:FF:FF:FF:FF:FF`
   - 强制刷新二层交换机的 CAM 表和局域网路由器的 ARP 映射。

---

### 2.3 工业双网口隔离与策略路由（Dual-NIC & Routing）

工业边缘场景典型拓扑为：
* **LAN1（视频专网）**：接 IPC 摄像头局域网（例如 `192.168.1.0/24`），**无默认网关**；
* **LAN2（业务上行网）**：接园区主网/外网（例如 `10.10.0.0/16`），**带默认网关与外部 DNS**。

#### 路由约束与策略配置：
1. **默认网关唯一性与 Metric 优先级**：
   - 严禁双网卡同时写入相同权重的 `default via` 路由。
   - 若用户确需配置多网关，系统自动为上行管理网卡赋予最高优先级（`metric 100`），从属网卡赋予低优先级（`metric 1000`）。
2. **基于源地址的策略路由（Policy-Based Routing）**：
   - 避免跨网口非对称路由（如请求从 LAN1 进，响应却从 LAN2 默认网关丢出导致握手失败）。
   - 为每个网口创建独立路由表：
     - `ip rule add from <LAN1_IP> table 101`
     - `ip route add default via <LAN1_GW> dev LAN1 table 101`
     - 同理配置 LAN2 table 102。
   - 保证“从哪个网口进来的包，响应原路从哪个网口返回”。

---

### 2.4 底层健壮性与掉电原子安全（Crash-Safe Persistence）

1. **环境隔离与国际化保护**：
   - 所有执行的系统命令强制注入环境变量：
     `cmd.env("LC_ALL", "C").env("LANG", "C")`
   - 废除所有的子串包含匹配，网卡状态解析统一匹配标准 POSIX/C 字符串（`connected` / `disconnected` / `routable`）。
2. **掉电原子落盘协议**：
   - 对于所有配置文件（如 `/etc/systemd/network/*.network`）：
     1. 生成临时文件：`/etc/systemd/network/.10-<iface>.network.tmp.<uuid>`
     2. 写入配置内容并显式执行 `tokio::fs::File::sync_all()` 强制刷入物理介质；
     3. 调用原子系统调用 `tokio::fs::rename(tmp, target)` 完成指针替换；
     4. 同步保留 `.bak` 镜像文件。
   - 即使写入过程中发生突发停电，文件系统也能保证旧文件完好或新文件完整，绝不产生 0 字节损坏文件。

---

### 2.5 真实物理载波与管理接口识别

1. **物理载波探测（Carrier Detect）**：
   - 读取 `/sys/class/net/<iface>/carrier`：
     - `1`：网线已插入并建立物理链路（Link Up）；
     - `0`：网线未插入（No Carrier）；
   - 读取 `/sys/class/net/<iface>/speed` 与 `duplex` 获取协商速率（如 1000 Mbps, Full Duplex）。
2. **管理接口动态推导**：
   - 废弃 `port_hex == 8000` 硬编码。
   - 判定标准：
     - 检查当前接收 HTTP 请求的宿主 IP 是否绑定在该网卡；
     - 检查内核默认路由（`ip route show default`）的 `dev <iface>` 是否指向该网卡；
     - 满足任意一条即标记为 `isManagementInterface: true`。

---

## 3. 兼容性与迁移

* **向后兼容**：
  * API 路径、请求参数结构保持与 `/api/v1/system/network/*` 完全一致；
  * 原有类型定义 `NetworkInterface`、`IpConfig` 保持兼容，新增字段均为可选扩展；
* **环境兼容**：
  * 在 macOS 或只读开发环境下，自动回退到模拟只读模式，不破坏本地开发体验；
  * Linux 下优先识别 NetworkManager，次优先 systemd-networkd。
