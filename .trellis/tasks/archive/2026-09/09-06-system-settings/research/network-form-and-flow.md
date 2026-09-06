# 设备网络表单与应用流程设计

状态：已完成设计，可作为 design.md 和 api.md 输入。

## 设计目标

1. 管理员能从统一界面查看设备所有网卡的真实 IP 配置
2. 能修改指定网卡的 IPv4 地址（静态/DHCP），配置持久生效且重启后保持
3. 修改管理网卡 IP 时，有完善的安全确认、连接中断恢复和超时回滚机制

---

## 一、网卡列表与状态模型

### 1.1 网卡数据结构

```typescript
interface NetworkInterface {
  name: string;              // e.g. "eth0", "wlan0"
  type: "ethernet" | "wifi" | "loopback" | "virtual";
  state: "up" | "down" | "unknown";
  mac: string;               // MAC 地址
  manager: NetworkManager;   // 管理服务类型
  ipv4: IpConfig | null;     // 当前 IPv4 配置
  capabilities: InterfaceCapabilities;
}

interface IpConfig {
  method: "dhcp" | "static" | "none";
  address: string | null;    // e.g. "192.168.1.100"
  prefix: number | null;     // e.g. 24
  gateway: string | null;
  dns: string[];             // DNS 服务器列表
  // DHCP 模式下以下字段为只读租约信息
  dhcpLeaseObtained?: number;  // UTC ms
  dhcpLeaseExpires?: number;   // UTC ms
}

type NetworkManager =
  | "networkmanager"
  | "systemd-networkd"
  | "netplan"
  | "ifupdown"
  | "connman"
  | "unmanaged";

interface InterfaceCapabilities {
  canModifyIp: boolean;      // 是否可修改 IP
  canSetDhcp: boolean;       // 是否可切换 DHCP
  canSetStatic: boolean;     // 是否可设置静态 IP
  isManagementInterface: boolean;  // 是否为当前管理接口
  reason?: string;           // 不可修改的原因说明
}
```

### 1.2 管理接口检测

后端在启动时和每次查询时检测：
- Heimdall 监听的 socket 地址（`0.0.0.0:8000` 或指定地址）对应的网卡
- 通过 `/proc/net/tcp` 或 `ss -tlnp` 查找监听端口的本地地址
- 匹配到的网卡标记为 `isManagementInterface: true`

### 1.3 只读 vs 可编辑

| 场景 | 前端表现 |
|------|---------|
| `canModifyIp: true` | 表单可编辑，显示输入框 |
| `canModifyIp: false` + 有原因 | 灰色禁用，显示原因说明 |
| `manager: "unmanaged"` | 只读展示，提示"此网卡无网络管理服务" |
| `isManagementInterface: true` | 可编辑，但额外显示管理接口警告 |

---

## 二、网络配置表单

### 2.1 表单字段

```
┌─────────────────────────────────────────────┐
│ 网卡: eth0 [Ethernet] [已连接]               │
│ 管理服务: NetworkManager                     │
│ MAC: AA:BB:CC:DD:EE:FF                      │
├─────────────────────────────────────────────┤
│ IPv4 配置                                    │
│ ○ 自动获取 (DHCP)                            │
│ ● 手动配置                                   │
│   IP 地址: [192.168.1.100  ] / [24 ▾]       │
│   网关:   [192.168.1.1    ]                 │
│   DNS:   [8.8.8.8         ]                 │
│         [8.8.4.4         ] [+]              │
├─────────────────────────────────────────────┤
│         [保存配置]                            │
└─────────────────────────────────────────────┘
```

### 2.2 表单校验规则

| 字段 | 校验 | 错误信息 |
|------|------|---------|
| IP 地址 | IPv4 格式，非 0.0.0.0，非广播地址 | "请输入有效的 IPv4 地址" |
| 前缀长度 | 1–32 整数 | "前缀长度须在 1–32 之间" |
| 网关 | IPv4 格式，与 IP 同网段（可选校验） | "请输入有效的网关地址" |
| DNS | 至少一个，每个为 IPv4 或 hostname | "请输入至少一个 DNS 服务器" |
| DHCP 模式 | 清空静态字段，不发送 IP/网关/DNS | - |

### 2.3 前端交互流程

```
用户选择网卡 → 加载当前配置 → 编辑表单 → 点击保存
                                          ↓
                                    前端校验
                                          ↓
                                    显示确认对话框
                                          ↓
                                    调用 API
                                          ↓
                              ┌─── 非管理接口 ───┐
                              │                  │
                         保存成功            保存失败
                         显示成功            显示错误
                         保留草稿            保留草稿

                              ┌─── 管理接口 ────┐
                              │                  │
                         返回操作 ID        返回错误
                         显示断连指引        显示错误
                         跳转新地址          保留草稿
```

---

## 三、管理接口 IP 变更流程

### 3.1 完整时序

```
管理员          前端              后端              网络服务         新地址
  │              │                │                 │              │
  │  1.提交配置   │                │                 │              │
  │─────────────>│                │                 │              │
  │              │  2.校验+备份    │                 │              │
  │              │───────────────>│                 │              │
  │              │                │  3.读取当前配置   │              │
  │              │                │────────────────>│              │
  │              │                │  4.返回备份       │              │
  │              │                │<────────────────│              │
  │              │  5.返回操作ID   │                 │              │
  │              │  +新地址候选    │                 │              │
  │              │<───────────────│                 │              │
  │  6.显示确认   │                │                 │              │
  │  断连影响    │                │                 │              │
  │  新地址      │                │                 │              │
  │              │                │                 │              │
  │  7.确认应用   │                │                 │              │
  │─────────────>│                │                 │              │
  │              │  8.执行网络切换  │                 │              │
  │              │───────────────>│                 │              │
  │              │                │  9.nmcli up      │              │
  │              │                │────────────────>│              │
  │              │                │  10.连接中断      │              │
  │              │                │<···············>│              │
  │              │                │                 │              │
  │              │  11.返回切换中   │                 │              │
  │              │<───────────────│                 │              │
  │              │                │                 │              │
  │  12.旧页面   │                │                 │              │
  │  显示新地址   │                │                 │              │
  │  +恢复说明   │                │                 │              │
  │              │                │                 │              │
  │  13.浏览器    │                │                 │      14.新地址
  │  导航新地址   │                │                 │      可访问
  │──────────────────────────────────────────────────────────────>│
  │              │                │                 │              │
  │  15.新地址登录 │                │                 │              │
  │──────────────────────────────────────────────────────────────>│
  │              │                │                 │              │
  │              │  16.查询pending │                 │              │
  │              │───────────────────────────────────────────────>│
  │              │                │                 │    17.返回操作│
  │              │<───────────────────────────────────────────────│
  │              │                │                 │              │
  │  18.显示确认  │                │                 │              │
  │  使用新网络   │                │                 │              │
  │              │                │                 │              │
  │  19.确认     │                │                 │              │
  │─────────────>│                │                 │              │
  │              │  20.确认操作    │                 │              │
  │              │───────────────────────────────────────────────>│
  │              │                │                 │    21.确认成功│
  │              │<───────────────────────────────────────────────│
  │              │                │                 │              │
  │  22.显示成功  │                │                 │              │
```

### 3.2 操作状态模型

```typescript
interface NetworkChangeOperation {
  id: string;                    // 操作 UUID
  status: OperationStatus;
  interfaceName: string;
  oldConfig: IpConfig;
  newConfig: IpConfig;
  createdAt: number;             // UTC ms
  confirmDeadline: number;       // UTC ms (创建后 10 分钟)
  appliedAt: number | null;      // 实际应用时间
  confirmedAt: number | null;    // 管理员确认时间
  restoredAt: number | null;     // 恢复时间
  newAccessUrl: string | null;   // 新管理地址 (e.g. "http://192.168.1.200:8000")
  error: string | null;
}

type OperationStatus =
  | "pending_confirm"   // 已应用，等待管理员在新地址确认
  | "confirmed"         // 管理员已确认，操作完成
  | "restoring"         // 超时未确认，正在恢复
  | "restored"          // 已恢复到原配置
  | "failed";           // 应用失败
```

### 3.3 确认对话框内容

```
┌─────────────────────────────────────────────┐
│ ⚠ 修改管理网卡 IP                            │
├─────────────────────────────────────────────┤
│                                              │
│ 当前管理地址: 192.168.1.100:8000              │
│ 新管理地址:   192.168.1.200:8000              │
│                                              │
│ 此操作将修改 eth0 的 IP 地址。                │
│ 修改后当前页面将失去连接。                    │
│                                              │
│ 您需要：                                      │
│ 1. 在浏览器中访问新地址                       │
│ 2. 使用管理员账号登录                         │
│ 3. 在系统设置中确认使用新网络                  │
│                                              │
│ 如果 10 分钟内未确认，系统将自动恢复原配置。   │
│                                              │
│        [取消]    [确认并应用]                 │
└─────────────────────────────────────────────┘
```

### 3.4 旧页面断连指引

网络切换执行后，旧页面（仍在旧 IP 上）显示：

```
┌─────────────────────────────────────────────┐
│ 网络配置已应用                               │
├─────────────────────────────────────────────┤
│                                              │
│ 管理网卡 eth0 的 IP 已更改为:                 │
│ 192.168.1.200                                │
│                                              │
│ 请在浏览器中访问新地址并登录确认:             │
│ http://192.168.1.200:8000                    │
│                                              │
│ 确认期限: 10 分钟                             │
│ 剩余时间: 08:32                               │
│                                              │
│ 如果未在期限内确认，系统将自动恢复原配置。     │
│                                              │
│ 恢复后的配置: 192.168.1.100                   │
└─────────────────────────────────────────────┘
```

---

## 四、后端 API 设计

### 4.1 端点列表

```
GET    /api/v1/system/network/interfaces          → 网卡列表
GET    /api/v1/system/network/interfaces/:name    → 单网卡详情
PUT    /api/v1/system/network/interfaces/:name    → 修改网卡配置
GET    /api/v1/system/network/changes/pending     → 查询待确认操作
POST   /api/v1/system/network/changes/:id/confirm → 确认操作
POST   /api/v1/system/network/changes/:id/cancel  → 取消操作（未应用前）
```

### 4.2 请求响应模型

#### GET /api/v1/system/network/interfaces

```json
{
  "code": 0,
  "message": "success",
  "data": {
    "interfaces": [
      {
        "name": "eth0",
        "type": "ethernet",
        "state": "up",
        "mac": "AA:BB:CC:DD:EE:FF",
        "manager": "networkmanager",
        "ipv4": {
          "method": "static",
          "address": "192.168.1.100",
          "prefix": 24,
          "gateway": "192.168.1.1",
          "dns": ["8.8.8.8", "8.8.4.4"]
        },
        "capabilities": {
          "canModifyIp": true,
          "canSetDhcp": true,
          "canSetStatic": true,
          "isManagementInterface": true,
          "reason": null
        }
      }
    ],
    "pendingOperation": null
  },
  "timestamp": 1757155200000
}
```

#### PUT /api/v1/system/network/interfaces/:name

请求：
```json
{
  "method": "static",
  "address": "192.168.1.200",
  "prefix": 24,
  "gateway": "192.168.1.1",
  "dns": ["8.8.8.8", "8.8.4.4"]
}
```

非管理接口响应：
```json
{
  "code": 0,
  "message": "success",
  "data": {
    "applied": true,
    "operation": null
  },
  "timestamp": 1757155200000
}
```

管理接口响应（切换后）：
```json
{
  "code": 0,
  "message": "success",
  "data": {
    "applied": true,
    "operation": {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "status": "pending_confirm",
      "interfaceName": "eth0",
      "oldConfig": { "method": "static", "address": "192.168.1.100", "prefix": 24, "gateway": "192.168.1.1", "dns": ["8.8.8.8"] },
      "newConfig": { "method": "static", "address": "192.168.1.200", "prefix": 24, "gateway": "192.168.1.1", "dns": ["8.8.8.8"] },
      "createdAt": 1757155200000,
      "confirmDeadline": 1757155800000,
      "newAccessUrl": "http://192.168.1.200:8000"
    }
  },
  "timestamp": 1757155200000
}
```

#### POST /api/v1/system/network/changes/:id/confirm

```json
{
  "code": 0,
  "message": "success",
  "data": {
    "status": "confirmed",
    "confirmedAt": 1757155500000
  },
  "timestamp": 1757155500000
}
```

### 4.3 错误码设计

| 错误码 | HTTP | 说明 |
|--------|------|------|
| 51001 | 400 | 无效的 IP 地址格式 |
| 51002 | 400 | 前缀长度超出范围 |
| 51003 | 400 | 网关与 IP 不在同一网段 |
| 51004 | 400 | DNS 列表为空 |
| 51005 | 400 | 网卡不支持修改 |
| 51006 | 409 | 已有进行中的网络操作 |
| 51007 | 404 | 网卡不存在 |
| 51008 | 500 | 网络服务执行失败 |
| 51009 | 408 | 操作超时 |
| 51010 | 409 | 操作已过期，需重新提交 |

---

## 五、超时恢复机制

### 5.1 后端定时任务

```rust
// 启动时启动恢复检查器
async fn start_recovery_checker(db: DbPool) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;
        // 查找所有 status=pending_confirm 且 confirmDeadline < now 的操作
        // 对每个超时操作执行恢复：
        //   1. 读取 backupConfig
        //   2. 通过 NetworkService 恢复原配置
        //   3. 更新 status = "restored"
        //   4. 记录 restoredAt
    }
}
```

### 5.2 恢复流程

1. 后端定时器检测到超时操作
2. 读取操作记录中的 `backupConfig`（原始配置备份）
3. 调用 `NetworkService::restore()` 恢复原配置
4. 更新操作状态为 `restored`
5. 如果管理员在恢复前已从新地址登录并查询，显示"恢复中"
6. 恢复完成后，下次查询显示"已恢复到原配置"

### 5.3 前端超时处理

- 旧页面倒计时由前端本地计时（`confirmDeadline - Date.now()`）
- 倒计时归零时不直接宣称"已恢复"，而是轮询后端状态
- 新页面登录后查 `GET /pending`，根据 `status` 显示最终状态
- `status: "restored"` 显示"配置已自动恢复"

---

## 六、边界条件与安全

### 6.1 并发保护

- 同一时间只允许一个 `pending_confirm` 操作
- 新提交的修改会检查是否有活跃操作，若有返回 `51006`
- 操作完成后（confirmed/restored/failed）才允许新操作

### 6.2 幂等性

- `confirm` 操作幂等：重复确认返回相同结果
- `cancel` 操作幂等：已恢复的操作再次取消返回成功
- 后端操作 ID 基于 UUID，不依赖请求顺序

### 6.3 凭据安全

- JWT 不放入 URL、history 或 window.name
- 新地址使用标准登录流程
- 操作 ID 本身不是授权凭据，查询需认证
- `confirm` 端点需认证 + 操作 ID 匹配

### 6.4 网络状态持久化

- 操作记录存入 `system_configs` 表（JSON），key 为 `pending_network_operation`
- 后端重启后仍能恢复未确认的操作
- 进程崩溃恢复：重启后检查 pending 操作，继续倒计时或执行恢复

---

## 七、前端实现要点

### 7.1 状态管理

- 网卡列表通过 `GET /interfaces` 获取，不常驻 Zustand
- 表单草稿为组件 local state
- 操作状态通过 `GET /pending` 查询，不建立第二条 WebSocket
- 旧页面倒计时用 `useState` + `setInterval`

### 7.2 页面进入

- 登录后 Layout 检查 `GET /pending`
- 若有待确认操作且用户未在设置页，提示"有网络操作待确认"
- 自动切换到网络设置分组
- 不依赖原 origin 的 localStorage

### 7.3 组件结构

```
web/src/features/system/
├── SettingsPage.tsx              // 五组布局 + 左侧导航
├── SystemOverview.tsx            // 只读系统信息
├── AccountSecurity.tsx           // 账号信息 + 改密按钮
├── NetworkSettings.tsx           // 网卡列表 + 配置表单
├── StorageSettings.tsx           // 存储状态 + 保留策略
├── TimeSettings.tsx              // 时间状态 + NTP 配置
├── components/
│   ├── NetworkCard.tsx           // 单网卡配置卡片
│   ├── NetworkConfirmDialog.tsx  // 管理接口变更确认
│   ├── NetworkRecoveryPanel.tsx  // 断连指引面板
│   ├── StorageStatusCard.tsx     // 磁盘状态展示
│   └── TimeStatusCard.tsx        // 时间状态展示
└── hooks/
    ├── use-network.ts            // 网卡数据获取
    ├── use-storage-config.ts     // 存储配置读写
    └── use-time-config.ts        // 时间配置读写
```

---

## 八、Open Questions

1. DHCP 模式下，新 IP 需要等待 DHCP 租约完成才能知道。后端如何报告新地址？建议：切换 DHCP 后等待 `n` 秒获取租约，超时则返回"地址待获取"。
2. WiFi 网卡的 SSID/密码配置是否纳入首版？建议：首版仅支持有线以太网。
3. 多网卡场景：是否支持同时修改多个网卡？建议：首版每次只允许修改一个网卡。
4. IPv6 支持范围？建议：首版仅支持 IPv4，IPv6 作为后续迭代。
5. VPN/隧道接口是否展示？建议：展示但标记为不可修改。
