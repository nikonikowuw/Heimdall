# 系统设置模块 — 技术设计

## 1. 范围与约束

本设计覆盖五个设置分组的前后端实现：系统概览、账号与安全、网络/服务、存储与保留策略、对时服务。

### 已有可复用能力

| 能力 | 位置 | 复用方式 |
|------|------|---------|
| 认证 (login/logout/password/me) | `crates/api/src/routes/auth.rs` | 账号与安全分组直接调用 |
| `system_configs` 表 + CRUD | `crates/db/src/repository/system_config.rs` | 存储配置、对时配置持久化 |
| 统一信封 `{code, message, data, timestamp}` | `crates/api/src/response.rs` | 所有新端点遵循 |
| `request()` + Bearer 注入 | `web/src/lib/api.ts` | 前端所有设置 API 调用 |
| `ChangePasswordModal` | `web/src/components/ChangePasswordModal.tsx` | 账号安全分组复用 |
| 双主题 CSS 变量 | `web/src/styles/globals.css` | 前端样式统一 |
| i18n 三语动态加载 | `web/src/i18n/index.ts` | 新增 `system` namespace |

### 不在首版范围

- 多用户/角色/权限管理
- WiFi SSID/密码配置（仅支持有线以太网）
- IPv6 配置
- 远程升级、备份恢复、告警通知渠道
- 在线修改硬件并发/解码器数量等管线参数

---

## 2. 后端架构

### 2.1 Crate 职责划分

```
crates/
├── app/src/main.rs           # 装配所有服务句柄，传入 API state
├── api/src/routes/system.rs  # 新增：系统设置 HTTP handler（薄层）
├── api/src/state.rs          # 新增：持有 SystemService 句柄
├── pipeline/src/             # 新增：network/ 子模块（网络适配）
│   └── network/
│       ├── mod.rs            # NetworkService trait + factory
│       ├── nm.rs             # NetworkManager 适配
│       ├── networkd.rs       # systemd-networkd 适配
│       └── types.rs          # NetworkInterface, IpConfig 等
├── pipeline/src/storage_cleaner/  # 修改：支持运行时配置热更新
└── types/src/system.rs       # 新增：系统设置共享类型
```

**关键约束**：
- `api/routes/system.rs` 只做参数提取、校验和调用 service，不直接执行 `nmcli` 或 `timedatectl`
- 网络适配逻辑在 `pipeline/src/network/` 中，通过 trait 抽象，不泄漏平台差异到上层
- 存储清理器改造在 `pipeline/src/storage_cleaner/` 内部完成
- 对时服务作为独立模块放在 `pipeline/src/` 下（或 `app/src/ntp/`，取决于对宿主系统命令的依赖程度）

### 2.2 新增后端模块

#### A. SystemService（系统概览）

```rust
// crates/app/src/system_service.rs
pub struct SystemService {
    db: DbPool,
}

impl SystemService {
    /// 聚合只读系统信息：版本、设备型号、OS、内核、运行时间、
    /// CPU/内存/磁盘使用率、NPU 状态、活跃摄像头/任务/告警数
    pub async fn get_overview(&self) -> Result<SystemOverview>;
}
```

数据来源：
- 软件版本：`env!("CARGO_PKG_VERSION")` 编译时嵌入
- 设备型号：读 `/sys/devices/virtual/dmi/id/board_name`（失败返回 "Unknown"）
- OS/内核：`uname` 系统调用或 `/etc/os-release`
- CPU/内存：解析 `/proc/stat` + `/proc/meminfo`（快照差值计算使用率）
- 磁盘：复用 `statvfs`（与 StorageCleaner 共享）
- NPU：查询平台 SDK（不可用时返回 `null`）
- 运行时间：`/proc/uptime`
- 业务统计：cameras/tasks/alarms 表 count

#### B. NetworkService（网络配置）

```rust
// crates/pipeline/src/network/mod.rs
#[async_trait]
pub trait NetworkService: Send + Sync {
    /// 列出所有网卡及当前配置
    async fn list_interfaces(&self) -> Result<Vec<NetworkInterface>>;

    /// 获取单个网卡详情
    async fn get_interface(&self, name: &str) -> Result<NetworkInterface>;

    /// 修改网卡 IP 配置
    async fn update_interface(&self, name: &str, config: IpConfig) -> Result<NetworkUpdateResult>;

    /// 查询待确认的网络操作
    async fn get_pending_operation(&self) -> Result<Option<NetworkChangeOperation>>;

    /// 确认网络操作（管理员在新地址登录后调用）
    async fn confirm_operation(&self, operation_id: &str) -> Result<OperationConfirmResult>;

    /// 取消未执行的操作
    async fn cancel_operation(&self, operation_id: &str) -> Result<()>;
}
```

**适配策略**：
1. 启动时检测网卡管理服务类型（`nmcli general` / `networkctl status`）
2. 每种管理服务实现 `NetworkService` trait
3. 首版适配：NetworkManager (P0) + systemd-networkd (P0)
4. 未识别管理服务的网卡：只读展示，标记 `canModifyIp: false`

**管理网卡 IP 变更流程**：
1. 校验输入 → 备份当前配置 → 返回操作 ID + 新地址候选
2. 执行网络切换（通过 nmcli/networkctl）
3. 后端启动 10 分钟倒计时恢复器
4. 管理员在新地址登录 → 查询 pending → 确认
5. 超时未确认 → 自动恢复原配置

#### C. TimeService（对时服务）

```rust
// crates/pipeline/src/time_service/mod.rs
#[async_trait]
pub trait TimeService: Send + Sync {
    async fn get_status(&self) -> Result<TimeStatus>;
    async fn get_config(&self) -> Result<TimeConfig>;
    async fn update_config(&self, config: TimeConfig) -> Result<TimeConfig>;
    async fn force_sync(&self) -> Result<()>;
    async fn set_system_time(&self, time: &str) -> Result<()>;
}
```

**适配策略**：
1. 通用入口：`timedatectl`（systemd 系统标配）
2. 深度适配：chrony（`chronyc tracking/sources`）、systemd-timesyncd
3. 手动设置时间安全约束：时间差 ≤ 1年、操作日志审计

#### D. StorageCleaner 改造

```rust
// crates/pipeline/src/storage_cleaner/mod.rs
pub struct StorageCleaner {
    config: Arc<RwLock<StorageCleanerConfig>>,  // 改为热更新
    // ... 其余字段不变
}

impl StorageCleaner {
    /// 运行时更新配置（用户保存设置后调用）
    pub async fn update_config(&self, new_config: StorageCleanerConfig) {
        *self.config.write().await = new_config;
    }

    /// 查询当前磁盘状态（只读）
    pub async fn get_status(&self) -> Result<StorageStatus>;

    /// 查询当前配置
    pub async fn get_config(&self) -> Result<StorageCleanerConfig>;

    /// 更新配置
    pub async fn set_config(&self, config: StorageCleanerConfig) -> Result<StorageCleanerConfig>;

    /// 手动触发清理
    pub async fn trigger_cleanup(&self) -> Result<EvictionReport>;
}
```

**配置热更新路径**：
1. 用户保存 → `PUT /api/v1/system/storage/config`
2. API handler 校验 → 调用 `cleaner.set_config(new_config)`
3. 清理器内部 `RwLock` 立即生效
4. 下次清理周期使用新配置

### 2.3 错误码分配

在现有错误码体系中分配 **51xxx** 区间：

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
| 51100 | 400 | 存储配置校验失败 |
| 51200 | 400 | 时间配置校验失败 |
| 51201 | 400 | 时间差超过一年 |
| 51202 | 500 | NTP 服务执行失败 |
| 51300 | 500 | 系统信息读取失败 |

**i18n 路由**：新增 `crates/api/src/i18n/system.rs`，在 `i18n/mod.rs` 中将 51xxx 路由到此文件。

---

## 3. API 契约

### 3.1 端点总览

```
# 系统概览（只读）
GET  /api/v1/system/overview              → SystemOverview

# 账号与安全（复用已有）
GET  /api/v1/auth/me                      → AdminUserDto（已有）
PUT  /api/v1/auth/password                → AdminUserDto（已有）

# 网络配置
GET  /api/v1/system/network/interfaces    → { interfaces, pendingOperation }
GET  /api/v1/system/network/interfaces/:name → NetworkInterface
PUT  /api/v1/system/network/interfaces/:name → { applied, operation? }
GET  /api/v1/system/network/changes/pending  → NetworkChangeOperation | null
POST /api/v1/system/network/changes/:id/confirm → { status, confirmedAt }
POST /api/v1/system/network/changes/:id/cancel  → { status }

# 存储与保留策略
GET  /api/v1/system/storage/status        → StorageStatus
GET  /api/v1/system/storage/config        → StorageConfig
PUT  /api/v1/system/storage/config        → StorageConfig
POST /api/v1/system/storage/cleanup       → EvictionReport

# 对时服务
GET  /api/v1/system/time/status           → TimeStatus
GET  /api/v1/system/time/config           → TimeConfig
PUT  /api/v1/system/time/config           → TimeConfig
POST /api/v1/system/time/sync             → { synced: bool }
POST /api/v1/system/time/set              → { applied: bool }
```

### 3.2 请求/响应模型

所有端点遵循统一信封 `{ code: 0, message: "success", data: T, timestamp: i64 }`。
时间戳统一为 13 位 UTC 毫秒 `i64`。

详细模型定义见 PRD 中各分组的 API 设计章节，以及 research 文档中的完整示例。

### 3.3 认证

所有 `/api/v1/system/*` 端点进入 `protected` router，由 `require_auth` 中间件统一鉴权。

---

## 4. 前端架构

### 4.1 目录结构

```
web/src/features/system/
├── SettingsPage.tsx              # 五组布局 + 左侧导航（已完成）
├── SystemOverview.tsx            # 系统概览（已完成 UI，待接 API）
├── AccountSecurity.tsx           # 账号与安全（已完成 UI，待接 API）
├── NetworkSettings.tsx           # 网络配置（已完成 UI，待接 API）
├── StorageSettings.tsx           # 存储与保留策略（已完成 UI，待接 API）
├── TimeSettings.tsx              # 对时服务（已完成 UI，待接 API）
├── components/
│   ├── SettingsSection.tsx       # 卡片容器（已完成）
│   ├── ConfirmDialog.tsx         # 确认弹窗（已完成，含无障碍）
│   └── SettingsGroup.tsx         # 备用容器（可清理）
├── hooks/
│   ├── use-system-overview.ts    # 系统概览数据获取
│   ├── use-network.ts            # 网卡数据获取与操作
│   ├── use-storage-config.ts     # 存储配置读写
│   └── use-time-config.ts        # 时间配置读写
└── types.ts                      # 前端专用 DTO（从 types/system.ts 扩展）

web/src/lib/
└── system-api.ts                 # 系统设置 API 封装（待实现）

web/src/types/
└── system.ts                     # 共享类型定义（待扩展）
```

### 4.2 数据获取模式

每个分组使用独立的 custom hook，返回 `{ data, loading, error, refetch }`：

```typescript
// 示例：use-system-overview.ts
export function useSystemOverview() {
  const [data, setData] = useState<SystemOverview | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const fetchData = useCallback(async (signal?: AbortSignal) => {
    setLoading(true)
    setError(null)
    try {
      const result = await systemApi.getOverview(signal)
      setData(result)
    } catch (err) {
      if (err instanceof DOMException && err.name === 'AbortError') return
      setError(err instanceof Error ? err.message : '加载失败')
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    const controller = new AbortController()
    fetchData(controller.signal)
    return () => controller.abort()
  }, [fetchData])

  return { data, loading, error, refetch: () => fetchData() }
}
```

**关键约束**：
- 切组/卸载时 AbortController 取消未完成请求
- 保存失败保留当前草稿，不覆盖
- 保存成功用服务器返回值替换基线
- 不建立第二条 WebSocket 连接
- 不在 Zustand 中存储设置数据

### 4.3 网络操作状态管理

网络操作涉及跨 IP 恢复，需要特殊处理：

1. **Layout 启动时查询** `GET /api/v1/system/network/changes/pending`
2. 若有待确认操作，自动切换到网络设置分组
3. 新地址登录后，通过 pending 查询恢复操作上下文
4. 旧页面倒计时用 `useState` + `setInterval`，不依赖后端推送

### 4.4 API 封装层

```typescript
// web/src/lib/system-api.ts
import { request } from './api'

export const systemApi = {
  getOverview: (signal?: AbortSignal) =>
    request<SystemOverview>('/system/overview', { signal }),

  getNetworkInterfaces: (signal?: AbortSignal) =>
    request<NetworkInterfacesResponse>('/system/network/interfaces', { signal }),

  updateNetworkInterface: (name: string, config: IpConfig) =>
    request<NetworkUpdateResult>(`/system/network/interfaces/${name}`, {
      method: 'PUT',
      body: JSON.stringify(config),
    }),

  getStorageStatus: (signal?: AbortSignal) =>
    request<StorageStatus>('/system/storage/status', { signal }),

  getStorageConfig: (signal?: AbortSignal) =>
    request<StorageConfig>('/system/storage/config', { signal }),

  updateStorageConfig: (config: StorageConfig) =>
    request<StorageConfig>('/system/storage/config', {
      method: 'PUT',
      body: JSON.stringify(config),
    }),

  // ... 其余端点类似
}
```

### 4.5 类型定义

```typescript
// web/src/types/system.ts
export interface SystemOverview {
  softwareVersion: string
  deviceModel: string
  osInfo: string
  kernelVersion: string
  uptimeSeconds: number
  cpuUsagePercent: number
  memoryUsagePercent: number
  memoryUsedMb: number
  memoryTotalMb: number
  npuUsagePercent: number | null
  diskUsagePercent: number
  diskUsedGb: number
  diskTotalGb: number
  activeCameras: number
  totalCameras: number
  activeTasks: number
  todayAlarms: number
  todayCaptures: number
}

// NetworkInterface, IpConfig, StorageConfig, TimeConfig 等
// 与后端 Rust 类型一一对应，字段名 camelCase
```

---

## 5. 跨层数据流

### 5.1 存储配置保存流程

```
用户点击保存
  → StorageSettings 组件校验
  → systemApi.updateStorageConfig(draft)
  → HTTP PUT /api/v1/system/storage/config
  → api/routes/system.rs handler 提取参数、校验
  → pipeline/storage_cleaner.set_config(new_config)
  → RwLock 立即生效
  → system_config repo 写入 DB
  → 返回新配置
  → 前端用服务器返回值替换基线
```

### 5.2 网络配置修改流程（非管理网卡）

```
用户编辑表单 → 点击保存
  → NetworkSettings 校验
  → systemApi.updateNetworkInterface(name, config)
  → HTTP PUT /api/v1/system/network/interfaces/:name
  → handler 校验 + 检查 capabilities
  → NetworkService.update_interface()
  → nmcli/systemctl 执行修改
  → 验证内核实际生效 (ip addr show)
  → 返回 { applied: true, operation: null }
  → 前段显示成功
```

### 5.3 网络配置修改流程（管理网卡）

```
用户编辑表单 → 点击保存 → 弹出确认对话框
  → 用户确认
  → systemApi.updateNetworkInterface(name, config)
  → handler 校验 + 备份当前配置
  → 返回 { applied: true, operation: { id, status: "pending_confirm", ... } }
  → 前端显示断连指引面板 + 新地址链接
  → 后端执行网络切换
  → 用户在新地址登录
  → Layout 查询 GET /changes/pending
  → 发现待确认操作 → 切换到网络分组
  → 用户点击"确认使用新网络"
  → POST /changes/:id/confirm
  → 操作完成
```

---

## 6. 安全考虑

### 6.1 网络操作

- 同一时间只允许一个 `pending_confirm` 操作
- 管理网卡变更需二次确认（前端弹窗 + 后端操作记录）
- 超时（10 分钟）自动恢复原配置
- 恢复由后端独立执行，不依赖浏览器在线
- 操作记录持久化到 DB，进程重启后可恢复

### 6.2 时间操作

- 手动设置时间需二次确认
- 时间差不超过一年
- NTP 启用时警告可能被自动校正
- 操作日志审计（旧时间 → 新时间）

### 6.3 存储操作

- 手动清理需二次确认
- 水位安全网防止磁盘写满导致系统崩溃
- 配额限制防止单类证据占满磁盘

---

## 7. 性能与可靠性

### 7.1 系统概览

- CPU 使用率需要两次采样（间隔 500ms），使用专用线程避免阻塞 tokio
- 其余信息单次快照即可
- 不缓存，每次请求实时读取

### 7.2 网络配置

- 网卡列表通过 `ip link` + 管理服务查询，低频操作
- 网络切换是阻塞操作，需在专用线程执行
- 恢复定时器使用 `tokio::time::sleep`，不依赖系统时钟

### 7.3 存储配置

- 配置热更新通过 `RwLock` 实现，读多写少无竞争
- 清理操作本身在后台线程执行，不阻塞 API

---

## 8. 测试策略

### 后端

- 系统概览：mock `/proc` 文件系统读取
- 网络配置：mock `NetworkService` trait 实现
- 存储配置：mock `StorageCleaner` 配置更新
- 对时服务：mock `TimeService` trait 实现
- API handler：集成测试覆盖正常/错误路径

### 前端

- 扩展 `api.test.ts` 覆盖系统 API 调用
- 组件测试：表单校验、加载/错误/成功状态
- 网络操作流程：pending → confirm → restored 状态机

---

## 9. 迁移与兼容性

### 数据库

- `system_configs` 表新增 `storage_config` 和 `time_config` 两个 JSON key
- 首次启动时插入默认值（如不存在）
- 网络操作记录使用 `pending_network_operation` key

### 配置文件

- `AppConfig` 新增 `storage` 字段（保留天数、配额等默认值）
- 运行时配置优先级：API 修改 > 数据库 > 配置文件 > 代码默认值
- 配置文件中的值作为首次启动的初始值
