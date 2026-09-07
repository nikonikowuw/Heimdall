# 工业级网络服务 — API 契约与规范

## 1. 范围与目录边界 (Scope)

| 领域 | 负责目录 | 契约责任 |
|------|---------|---------|
| 共享类型 | `crates/types/src/system.rs`, `web/src/types/system.ts` | 双端序列化对齐、字段驼峰、单位与空值表示 |
| 后端实现 | `crates/api/src/network_service/`, `crates/api/src/routes/system.rs` | 路由分发、错误码映射、事务状态机驱动 |
| 前端实现 | `web/src/features/system/`, `web/src/lib/system-api.ts` | 交互状态、试运行横幅、ARP冲突提示、倒计时管理 |

---

## 2. 约定规范 (Conventions)

- 基础路径：`/api/v1`
- 编码格式：JSON，键名使用 `camelCase`
- 统一信封格式：
  ```json
  {
    "code": 0,
    "message": "success",
    "data": T,
    "timestamp": 1741334400000
  }
  ```
  错误时 `code != 0`，`data` 为 `null`。
- 时间戳：统一为 13 位 UTC 毫秒整数（`i64` / `number`）。

---

## 3. 共享数据模型 (Shared Models)

### 3.1 网卡信息 `NetworkInterface`

```typescript
interface NetworkInterface {
  name: string;                                 // 网卡名称，例如 "eth0"
  type: "ethernet" | "wifi" | "loopback" | "virtual";
  state: "up" | "down" | "unknown";
  carrier: boolean | null;                      // 物理网线是否插入 (Link Detect)
  speed: number | null;                         // 协商速率 Mbps (如 1000)
  duplex: "full" | "half" | null;               // 双工模式
  mac: string;                                  // MAC 地址 (大写，冒号分隔)
  manager: "networkmanager" | "systemd-networkd" | "netplan" | "ifupdown" | "connman" | "unmanaged";
  ipv4: IpConfig | null;
  capabilities: InterfaceCapabilities;
}

interface InterfaceCapabilities {
  canModifyIp: boolean;                         // 是否可修改 IP
  canSetDhcp: boolean;                          // 是否可设置 DHCP
  canSetStatic: boolean;                        // 是否可设置静态 IP
  isManagementInterface: boolean;               // 是否为当前承载 Web 管理控制台的网卡
  reason?: string | null;                       // 不可修改时的提示说明
}
```

### 3.2 IP 配置 `IpConfig`

```typescript
interface IpConfig {
  method: "dhcp" | "static" | "none";
  address: string | null;                       // IPv4 地址，例如 "192.168.1.100"
  prefix: number | null;                        // 子网掩码前缀 (1-32)，例如 24
  gateway: string | null;                       // 默认网关 (可选)
  dns: string[];                                // DNS 服务器列表
  metric?: number | null;                       // 路由优先级 (默认 100)
}
```

### 3.3 试运行事务 `NetworkChangeOperation`

```typescript
interface NetworkChangeOperation {
  id: string;                                   // 事务唯一 UUID
  status: "pending_confirm" | "confirmed" | "restoring" | "restored" | "failed";
  interfaceName: string;                        // 变更的目标网卡
  oldConfig: IpConfig;                          // 变更前快照
  newConfig: IpConfig;                          // 变更的新配置
  createdAt: number;                            // 创建时间戳 (UTC ms)
  confirmDeadlineMs: number;                    // 确认截止时间戳 (UTC ms)
  newAccessUrl: string | null;                  // 建议的新控制台访问 URL
}
```

### 3.4 响应结果

```typescript
interface NetworkInterfacesResponse {
  interfaces: NetworkInterface[];
  pendingOperation: NetworkChangeOperation | null;
}

interface NetworkUpdateResult {
  applied: boolean;                             // 是否已永久生效（非管理口为 true，管理口试运行为 false）
  operation: NetworkChangeOperation | null;     // 若进入试运行模式，返回此操作对象
}

interface OperationConfirmResult {
  status: "confirmed" | "restored" | "failed";
  confirmedAt: number | null;
}
```

---

## 4. API 端点 (Endpoints)

### 4.1 获取网卡列表及未决操作
- **方法与路径**：`GET /api/v1/system/network/interfaces`
- **鉴权**：Bearer Token
- **响应**：`ApiResponse<NetworkInterfacesResponse>`

### 4.2 获取单个网卡详情
- **方法与路径**：`GET /api/v1/system/network/interfaces/:name`
- **鉴权**：Bearer Token
- **响应**：`ApiResponse<NetworkInterface>`

### 4.3 提交网卡配置修改（带 ARP 冲突预检与管理口防失联保护）
- **方法与路径**：`PUT /api/v1/system/network/interfaces/:name`
- **鉴权**：Bearer Token
- **请求体**：`IpConfig`
- **逻辑分支**：
  1. **ARP 冲突**：若检测到局域网已有相同 IP，返回 `51011` 错误，携带冲突 MAC。
  2. **非管理网卡**：直接原子应用并广播免费 ARP，返回 `{ applied: true, operation: null }`。
  3. **管理网卡**：生成快照、进入试运行模式、启动 60s 看门狗，返回 `{ applied: false, operation: { ... } }`。
- **响应**：`ApiResponse<NetworkUpdateResult>`

### 4.4 查询当前未决的试运行变更
- **方法与路径**：`GET /api/v1/system/network/changes/pending`
- **鉴权**：Bearer Token
- **响应**：`ApiResponse<NetworkChangeOperation | null>`

### 4.5 确认试运行网络变更（永久固化生效）
- **方法与路径**：`POST /api/v1/system/network/changes/:id/confirm`
- **鉴权**：Bearer Token
- **说明**：管理员在新 IP 登录后调用，停止看门狗，固化持久化配置。
- **响应**：`ApiResponse<OperationConfirmResult>`

### 4.6 放弃试运行网络变更（立即手动回滚）
- **方法与路径**：`POST /api/v1/system/network/changes/:id/cancel`
- **鉴权**：Bearer Token
- **说明**：管理员主动要求取消变更，立即无条件还原备份快照。
- **响应**：`ApiResponse<OperationConfirmResult>`

---

## 5. 错误码规范 (Error Codes)

| 错误码 | 标识符 | HTTP 状态码 | 含义说明 |
|-------|--------|------------|---------|
| 51001 | `NetworkInvalid` | 400 Bad Request | IP 地址格式无效、前缀超出 1-32、网关格式错误 |
| 51005 | `NetworkInterfaceReadOnly` | 400 Bad Request | 网卡为只读网卡（如未受管接口或回环） |
| 51006 | `NetworkPendingOperation` | 409 Conflict | 当前已有正在试运行的变更事务，请先确认或回滚 |
| 51007 | `NetworkInterfaceNotFound` | 404 Not Found | 目标网卡不存在 |
| 51008 | `NetworkFailed` | 500 Internal Error | 底层系统调用失败或网络服务异常 |
| 51009 | `NetworkOperationTimeout` | 408 Request Timeout | 试运行超时，已触发自动回滚 |
| 51010 | `NetworkOperationExpired` | 409 Conflict | 操作已失效或已被其他流程取消 |
| **51011** | `NetworkIpConflict` | 409 Conflict | **RFC 5227 检测到静态 IP 冲突（包含占用者 MAC）** |
| **51012** | `NetworkGatewayUnreachable`| 400 Bad Request | **网关与指定 IP 不在同一子网或链路不可达** |

---

## 6. 变更记录 (Changelog)

- **v1.0 (2026-09-07)**:
  - 扩展 `NetworkInterface` 增加物理链路属性（`carrier`, `speed`, `duplex`）。
  - 扩展 `IpConfig` 增加可选路由权重字段（`metric`）。
  - 新增 `51011 NetworkIpConflict` 与 `51012 NetworkGatewayUnreachable` 工业级错误码。
  - 完善 `confirm` 与 `cancel` 端点的状态流转与数据契约。
