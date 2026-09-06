# 系统设置模块 — API 契约

## 范围

| 层 | 边界 |
|---|---|
| 共享类型 | `crates/types/src/system.rs`（Rust DTO）、`web/src/types/system.ts`（TS DTO） |
| 后端 API | `crates/api/src/routes/system.rs`（handler）、`crates/api/src/i18n/system.rs`（错误 i18n） |
| 后端服务 | `crates/app/src/system_service.rs`、`crates/pipeline/src/network/`、`crates/pipeline/src/time_service/`、`crates/pipeline/src/storage_cleaner/` |
| 前端 API 层 | `web/src/lib/system-api.ts` |
| 前端页面 | `web/src/features/system/`（组件 + hooks） |

## 约定

- 所有端点：`/api/v1/system/*`，JSON，`camelCase`
- 认证：`Authorization: Bearer <token>`，进入 `protected` router
- 统一信封：`{ code: 0, message: "success", data: T, timestamp: i64 }`
- 错误信封：`{ code: number, message: null, data: null, timestamp: i64 }`
- 时间戳：13 位 UTC 毫秒 `i64`
- 相对时长：带 `Ms` 后缀（如 `timeoutMs`、`confirmDeadlineMs`）

---

## 端点详情

### 1. 系统概览

#### `GET /api/v1/system/overview`

只读。聚合系统运行状态。

**Response data:**

```json
{
  "softwareVersion": "0.1.0",
  "deviceModel": "RK3588-EVB",
  "osInfo": "Ubuntu 22.04.3 LTS",
  "kernelVersion": "5.10.160",
  "uptimeSeconds": 86400,
  "cpuUsagePercent": 35.2,
  "memoryUsagePercent": 62.8,
  "memoryUsedMb": 2512,
  "memoryTotalMb": 4000,
  "npuUsagePercent": 78.5,
  "diskUsagePercent": 45.3,
  "diskUsedGb": 12.8,
  "diskTotalGb": 28.3,
  "activeCameras": 4,
  "totalCameras": 6,
  "activeTasks": 3,
  "todayAlarms": 12,
  "todayCaptures": 156
}
```

**错误码：** 51300（系统信息读取失败）

---

### 2. 账号与安全

复用已有端点，无新增：

- `GET /api/v1/auth/me` → `AdminUserDto`
- `PUT /api/v1/auth/password` → `AdminUserDto`

---

### 3. 网络配置

#### `GET /api/v1/system/network/interfaces`

查询所有网卡及当前配置。

**Response data:**

```json
{
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
}
```

**manager 枚举：** `"networkmanager" | "systemd-networkd" | "netplan" | "ifupdown" | "connman" | "unmanaged"`

**type 枚举：** `"ethernet" | "wifi" | "loopback" | "virtual"`

**state 枚举：** `"up" | "down" | "unknown"`

#### `GET /api/v1/system/network/interfaces/:name`

查询单个网卡详情。响应 data 同上 `interfaces[]` 元素。

**错误码：** 51007（网卡不存在）

#### `PUT /api/v1/system/network/interfaces/:name`

修改网卡 IPv4 配置。

**Request body:**

```json
{
  "method": "static",
  "address": "192.168.1.200",
  "prefix": 24,
  "gateway": "192.168.1.1",
  "dns": ["8.8.8.8", "8.8.4.4"]
}
```

`method` 为 `"dhcp"` 时，`address`/`prefix`/`gateway`/`dns` 可省略。

**Response data（非管理接口）：**

```json
{
  "applied": true,
  "operation": null
}
```

**Response data（管理接口）：**

```json
{
  "applied": true,
  "operation": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "status": "pending_confirm",
    "interfaceName": "eth0",
    "oldConfig": {
      "method": "static",
      "address": "192.168.1.100",
      "prefix": 24,
      "gateway": "192.168.1.1",
      "dns": ["8.8.8.8"]
    },
    "newConfig": {
      "method": "static",
      "address": "192.168.1.200",
      "prefix": 24,
      "gateway": "192.168.1.1",
      "dns": ["8.8.8.8"]
    },
    "createdAt": 1757155200000,
    "confirmDeadlineMs": 600000,
    "newAccessUrl": "http://192.168.1.200:8000"
  }
}
```

**operation.status 枚举：** `"pending_confirm" | "confirmed" | "restoring" | "restored" | "failed"`

**错误码：** 51001–51010

#### `GET /api/v1/system/network/changes/pending`

查询当前待确认的网络操作。

**Response data:** `NetworkChangeOperation | null`（同上 operation 结构）

#### `POST /api/v1/system/network/changes/:id/confirm`

确认网络操作（管理员在新地址登录后调用）。

**Response data:**

```json
{
  "status": "confirmed",
  "confirmedAt": 1757155500000
}
```

#### `POST /api/v1/system/network/changes/:id/cancel`

取消未执行的操作。

**Response data:**

```json
{
  "status": "restored",
  "restoredAt": 1757155500000
}
```

---

### 4. 存储与保留策略

#### `GET /api/v1/system/storage/status`

只读。磁盘状态与证据统计。

**Response data:**

```json
{
  "totalGb": 28.3,
  "usedGb": 12.8,
  "availableGb": 15.5,
  "usagePercent": 45.3,
  "healthLevel": "normal",
  "alarmCount": 120,
  "alarmSizeMb": 45.2,
  "recognitionCount": 340,
  "recognitionSizeMb": 128.5,
  "captureCount": 1560,
  "captureSizeMb": 890.3
}
```

**healthLevel 枚举：** `"normal" | "evicting" | "emergency" | "critical"`

#### `GET /api/v1/system/storage/config`

**Response data:**

```json
{
  "alarmRetentionDays": 30,
  "alarmQuotaMb": 0,
  "recognitionRetentionDays": 14,
  "recognitionQuotaMb": 0,
  "captureRetentionDays": 7,
  "captureQuotaMb": 0,
  "overwriteMode": "overwrite",
  "autoCleanupEnabled": true,
  "minFreeRatio": 0.15,
  "targetFreeRatio": 0.25,
  "emergencyFreeRatio": 0.08,
  "criticalFreeRatio": 0.05,
  "batchDeleteSize": 100
}
```

**overwriteMode 枚举：** `"overwrite" | "stop"`

#### `PUT /api/v1/system/storage/config`

**Request body:** 同上结构（全部字段）。

**校验规则：**
- `targetFreeRatio` > `minFreeRatio`
- `emergencyFreeRatio` < `minFreeRatio`
- `criticalFreeRatio` < `emergencyFreeRatio`
- `alarmRetentionDays`/`recognitionRetentionDays`/`captureRetentionDays`: 1–365
- `batchDeleteSize`: 10–500

**Response data:** 返回更新后的完整配置（服务器规范化值）。

**错误码：** 51100（配置校验失败）

#### `POST /api/v1/system/storage/cleanup`

手动触发一次清理。

**Response data:**

```json
{
  "deletedCount": 45,
  "freedMb": 128.5,
  "durationMs": 2300
}
```

---

### 5. 对时服务

#### `GET /api/v1/system/time/status`

只读。系统时间状态。

**Response data:**

```json
{
  "systemTime": 1757155200000,
  "timezone": "Asia/Shanghai",
  "timezoneOffset": 28800,
  "ntpSynced": true,
  "ntpService": "chrony",
  "ntpServer": "pool.ntp.org",
  "offsetMs": 12
}
```

**ntpService 枚举：** `"chrony" | "timesyncd" | "ntpd" | "none"`

#### `GET /api/v1/system/time/config`

**Response data:**

```json
{
  "ntpEnabled": true,
  "ntpServer": "pool.ntp.org",
  "timezone": "Asia/Shanghai"
}
```

#### `PUT /api/v1/system/time/config`

**Request body:** 同上结构。

**Response data:** 返回更新后的完整配置。

**错误码：** 51200（配置校验失败）

#### `POST /api/v1/system/time/sync`

手动触发 NTP 同步。

**Response data:**

```json
{
  "synced": true
}
```

**错误码：** 51202（NTP 服务执行失败）

#### `POST /api/v1/system/time/set`

手动设置系统时间（高风险）。

**Request body:**

```json
{
  "time": "2026-09-06 16:00:00"
}
```

**安全约束：**
- 时间差不超过一年
- 操作日志审计

**Response data:**

```json
{
  "applied": true,
  "previousTime": 1757151600000,
  "newTime": 1757155200000
}
```

**错误码：** 51201（时间差超过一年）、51202（执行失败）

---

## 错误码汇总

| 码 | HTTP | 说明 |
|---|---|---|
| 51001 | 400 | 无效的 IP 地址格式 |
| 51002 | 400 | 前缀长度超出范围 (1–32) |
| 51003 | 400 | 网关与 IP 不在同一网段 |
| 51004 | 400 | DNS 列表为空 |
| 51005 | 400 | 网卡不支持修改 |
| 51006 | 409 | 已有进行中的网络操作 |
| 51007 | 404 | 网卡不存在 |
| 51008 | 500 | 网络服务执行失败 |
| 51009 | 408 | 操作超时 |
| 51010 | 409 | 操作已过期，需重新提交 |
| 51100 | 400 | 存储配置校验失败 |
| 51200 | 400 | 时间配置校验失败 |
| 51201 | 400 | 时间差超过一年 |
| 51202 | 500 | NTP 服务执行失败 |
| 51300 | 500 | 系统信息读取失败 |

## Changelog

- 2026-09-06: 初始版本，覆盖五个分组全部端点。
