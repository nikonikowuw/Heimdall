# API 契约：系统概述模块升级

- 任务：.trellis/tasks/09-08-system-overview-upgrade
- 状态：draft
- 确认时间：（待用户确认）
- 作者：主会话撰写，用户确认

## 1. Scope（目录边界）

| 侧       | 目录           | 规则 |
|----------|----------------|------|
| frontend | web/src        | 前端实现域，不得改动其他目录 |
| backend  | crates         | 后端实现域，不得改动其他目录 |
| shared   | crates/types   | 共享类型，仅主会话可改 |

## 2. Conventions（全局约定）

- 时间格式：13 位 Unix 毫秒整数（UTC）
- ID 格式：字符串（摄像头 ID）或整数（数据库主键）
- 分页参数：page/pageSize（本 API 无分页）
- 错误响应外层结构：{ "code": number, "message": string, "data": null, "timestamp": number }
- 认证头：Authorization: Bearer <token>
- 内容类型：application/json

## 3. Shared Models（共享数据模型）

### SystemOverview（增强版）

```json
{
  "softwareVersion": "0.1.0",
  "deviceModel": "RK3588 EVB Board",
  "osInfo": "Ubuntu 22.04 LTS",
  "kernelVersion": "5.10.160",
  "uptimeSeconds": 123456,
  "cpu": {
    "overallPercent": 45.2,
    "perCore": [
      { "coreId": 0, "usagePercent": 62.3, "frequencyMhz": 1800, "temperature": null },
      { "coreId": 1, "usagePercent": 23.1, "frequencyMhz": 1200, "temperature": null }
    ],
    "temperature": 62.5,
    "frequencyMhz": 2000,
    "topProcesses": [
      { "pid": 1234, "name": "rknn_infer", "cpuPercent": 45.2, "memoryMb": 256 }
    ]
  },
  "memory": {
    "totalMb": 8192,
    "usedMb": 4096,
    "availableMb": 4096,
    "cachedMb": 1024,
    "bufferMb": 512,
    "swapTotalMb": 2048,
    "swapUsedMb": 128
  },
  "npu": {
    "deviceType": "rk3588-npu",
    "cores": [
      { "coreId": 0, "utilizationPercent": 82.3, "frequencyMhz": 1000, "powerWatts": null },
      { "coreId": 1, "utilizationPercent": 45.6, "frequencyMhz": 800, "powerWatts": null },
      { "coreId": 2, "utilizationPercent": 0.0, "frequencyMhz": 500, "powerWatts": null }
    ],
    "totalMemoryMb": 2048,
    "usedMemoryMb": 1024,
    "temperature": 72.1,
    "activeSessions": 2
  },
  "network": [
    {
      "name": "eth0",
      "rxBytes": 1234567890,
      "txBytes": 987654321,
      "rxPackets": 1234567,
      "txPackets": 987654,
      "rxErrors": 0,
      "txErrors": 0,
      "rxDropped": 0,
      "txDropped": 0,
      "speedMbps": 1000,
      "linkUp": true
    }
  ],
  "thermal": {
    "zones": [
      { "name": "cpu-thermal", "temperature": 62.5, "typeLabel": "cpu" },
      { "name": "npu-thermal", "temperature": 72.1, "typeLabel": "npu" },
      { "name": "ddr-thermal", "temperature": 55.0, "typeLabel": "ddr" }
    ],
    "throttleActive": false
  },
  "disk": {
    "totalGb": 58.5,
    "usedGb": 23.4,
    "availableGb": 35.1,
    "inodeTotal": 3840000,
    "inodeUsed": 156000,
    "inodeAvailable": 3684000
  },
  "activeCameras": 4,
  "totalCameras": 8,
  "activeTasks": 2,
  "todayAlarms": 12,
  "todayCaptures": 156
}
```

### 字段说明

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| softwareVersion | string | ✅ | 软件版本号 |
| deviceModel | string | ✅ | 设备型号 |
| osInfo | string | ✅ | 操作系统信息 |
| kernelVersion | string | ✅ | 内核版本 |
| uptimeSeconds | number | ✅ | 运行时间（秒） |
| cpu | CpuMetrics | ✅ | CPU 指标 |
| memory | MemoryMetrics | ✅ | 内存指标 |
| npu | NpuMetrics \| null | ✅ | NPU 指标（无 NPU 时为 null） |
| network | NetworkInterfaceMetrics[] | ✅ | 网络接口列表 |
| thermal | ThermalMetrics | ✅ | 温度指标 |
| disk | DiskMetrics | ✅ | 磁盘指标 |
| activeCameras | number | ✅ | 活跃摄像头数 |
| totalCameras | number | ✅ | 总摄像头数 |
| activeTasks | number | ✅ | 活跃任务数 |
| todayAlarms | number | ✅ | 今日告警数 |
| todayCaptures | number | ✅ | 今日抓拍数 |

## 4. Endpoints

### GET /api/v1/system/overview

- 描述：获取系统概述信息（增强版）
- 路径参数：无
- 查询参数：无
- 请求体：无
- 响应 200：
  ```json
  {
    "code": 0,
    "message": "success",
    "data": { /* SystemOverview */ },
    "timestamp": 1694169600000
  }
  ```
- 错误响应：
  - 401: 未认证
  - 500: 系统信息读取失败

#### 响应示例（完整）

```json
{
  "code": 0,
  "message": "success",
  "data": {
    "softwareVersion": "0.1.0",
    "deviceModel": "RK3588 EVB Board",
    "osInfo": "Ubuntu 22.04 LTS",
    "kernelVersion": "5.10.160",
    "uptimeSeconds": 123456,
    "cpu": {
      "overallPercent": 45.2,
      "perCore": [
        { "coreId": 0, "usagePercent": 62.3, "frequencyMhz": 1800, "temperature": null },
        { "coreId": 1, "usagePercent": 23.1, "frequencyMhz": 1200, "temperature": null },
        { "coreId": 2, "usagePercent": 78.5, "frequencyMhz": 2000, "temperature": null },
        { "coreId": 3, "usagePercent": 17.0, "frequencyMhz": 1000, "temperature": null }
      ],
      "temperature": 62.5,
      "frequencyMhz": 2000,
      "topProcesses": [
        { "pid": 1234, "name": "rknn_infer", "cpuPercent": 45.2, "memoryMb": 256 },
        { "pid": 2345, "name": "ffmpeg", "cpuPercent": 12.3, "memoryMb": 128 }
      ]
    },
    "memory": {
      "totalMb": 8192,
      "usedMb": 4096,
      "availableMb": 4096,
      "cachedMb": 1024,
      "bufferMb": 512,
      "swapTotalMb": 2048,
      "swapUsedMb": 128
    },
    "npu": {
      "deviceType": "rk3588-npu",
      "cores": [
        { "coreId": 0, "utilizationPercent": 82.3, "frequencyMhz": 1000, "powerWatts": null },
        { "coreId": 1, "utilizationPercent": 45.6, "frequencyMhz": 800, "powerWatts": null },
        { "coreId": 2, "utilizationPercent": 0.0, "frequencyMhz": 500, "powerWatts": null }
      ],
      "totalMemoryMb": 2048,
      "usedMemoryMb": 1024,
      "temperature": 72.1,
      "activeSessions": 2
    },
    "network": [
      {
        "name": "eth0",
        "rxBytes": 1234567890,
        "txBytes": 987654321,
        "rxPackets": 1234567,
        "txPackets": 987654,
        "rxErrors": 0,
        "txErrors": 0,
        "rxDropped": 0,
        "txDropped": 0,
        "speedMbps": 1000,
        "linkUp": true
      },
      {
        "name": "wlan0",
        "rxBytes": 98765432,
        "txBytes": 12345678,
        "rxPackets": 98765,
        "txPackets": 12345,
        "rxErrors": 0,
        "txErrors": 0,
        "rxDropped": 0,
        "txDropped": 0,
        "speedMbps": 72,
        "linkUp": true
      }
    ],
    "thermal": {
      "zones": [
        { "name": "cpu-thermal", "temperature": 62.5, "typeLabel": "cpu" },
        { "name": "npu-thermal", "temperature": 72.1, "typeLabel": "npu" },
        { "name": "ddr-thermal", "temperature": 55.0, "typeLabel": "ddr" },
        { "name": "board-thermal", "temperature": 45.0, "typeLabel": "board" }
      ],
      "throttleActive": false
    },
    "disk": {
      "totalGb": 58.5,
      "usedGb": 23.4,
      "availableGb": 35.1,
      "inodeTotal": 3840000,
      "inodeUsed": 156000,
      "inodeAvailable": 3684000
    },
    "activeCameras": 4,
    "totalCameras": 8,
    "activeTasks": 2,
    "todayAlarms": 12,
    "todayCaptures": 156
  },
  "timestamp": 1694169600000
}
```

## 5. Auth / Session

- 所有请求需要 JWT 认证
- Header: `Authorization: Bearer <token>`
- 无 NPU 设备时，`npu` 字段返回 `null`
- 无权限读取某些指标时，对应字段返回 `null`

## 6. 变更记录

| 日期 | 变更内容 | 重新确认 |
|------|----------|----------|
| 2024-09-08 | 初始版本 | - |
