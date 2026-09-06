# 系统设置模块 — 执行计划

## 执行原则

1. **后端先行**：每个分组先实现后端 API，再联调前端
2. **独立分组**：五个分组可并行开发，但共享基础设施（错误码、i18n、类型）需先完成
3. **小步验证**：每个步骤完成后运行对应验证命令
4. **前端已就绪**：UI 组件已完成，主要工作是接 API + 补 hooks

---

## Phase 0: 基础设施（共享）

### Step 0.1: 共享类型定义

**文件**：`crates/types/src/system.rs`（新建）、`crates/types/src/lib.rs`（修改）

- [ ] 定义 `SystemOverview`、`StorageConfig`、`StorageStatus`、`TimeConfig`、`TimeStatus` 结构体
- [ ] 定义 `NetworkInterface`、`IpConfig`、`InterfaceCapabilities`、`NetworkChangeOperation` 等网络类型
- [ ] 所有时间戳字段使用 `i64`（UTC 毫秒），相对时长使用 `i64` 带 `Ms` 后缀
- [ ] 派生 `Serialize`、`Deserialize`、`Clone`、`Debug`

**验证**：`cargo check -p types`

### Step 0.2: 错误码与 i18n

**文件**：`crates/api/src/error.rs`（修改）、`crates/api/src/i18n/`（新增 system.rs）

- [ ] 在 `ApiError` 枚举中新增 51xxx 系列错误码
- [ ] 新建 `crates/api/src/i18n/system.rs`，定义三语翻译
- [ ] 在 `crates/api/src/i18n/mod.rs` 中将 51xxx 路由到 system 模块
- [ ] 确保成功响应 message 在 zh-TW 下仍为 "success"（验证现有 i18n 行为）

**验证**：`cargo check -p api && cargo test -p api`

### Step 0.3: 前端类型与 API 层

**文件**：`web/src/types/system.ts`（新建/扩展）、`web/src/lib/system-api.ts`（新建）

- [ ] 扩展 `web/src/types/system.ts`，确保与后端 Rust 类型一一对应
- [ ] 创建 `web/src/lib/system-api.ts`，封装所有系统设置 API 调用
- [ ] 所有请求通过 `request()` 函数，复用 Bearer 注入
- [ ] 添加 AbortSignal 支持

**验证**：`cd web && pnpm typecheck`

### Step 0.4: 前端 i18n

**文件**：`web/src/i18n/{zh-CN,zh-TW,en}/system.json`、`web/src/i18n/index.ts`

- [ ] 创建三语 system.json（如不存在）
- [ ] 在 i18n/index.ts 中注册 `system` namespace
- [ ] 确保所有设置页面的 `t()` 调用有对应翻译

**验证**：`cd web && pnpm typecheck && pnpm lint`

---

## Phase 1: 系统概览

### Step 1.1: 后端 SystemService

**文件**：`crates/app/src/system_service.rs`（新建）

- [ ] 实现 `SystemService::get_overview()`
- [ ] 读取 `/proc/stat`、`/proc/meminfo`、`/proc/uptime`
- [ ] 读取 `/sys/devices/virtual/dmi/id/board_name`（设备型号）
- [ ] 调用 `uname` 获取 OS/内核信息
- [ ] 复用 `statvfs` 获取磁盘信息
- [ ] 查询 cameras/tasks/alarms 表统计
- [ ] CPU 使用率使用两次采样（500ms 间隔），在专用线程执行

**验证**：`cargo test -p app`

### Step 1.2: 后端 API 端点

**文件**：`crates/api/src/routes/system.rs`（新建）、`crates/api/src/routes/mod.rs`（修改）

- [ ] 实现 `GET /api/v1/system/overview` handler
- [ ] 加入 protected router
- [ ] 在 `crates/app/src/main.rs` 中装配 `SystemService` 到 API state

**验证**：`cargo test -p api`

### Step 1.3: 前端联调

**文件**：`web/src/features/system/hooks/use-system-overview.ts`（新建）、`web/src/features/system/SystemOverview.tsx`（修改）

- [ ] 创建 `useSystemOverview` hook
- [ ] 修改 `SystemOverview` 组件使用 hook 数据
- [ ] 处理 loading/error/success 状态
- [ ] 确保 `formatUptime` 使用 i18n

**验证**：`cd web && pnpm typecheck && pnpm lint`

---

## Phase 2: 账号与安全

### Step 2.1: 前端联调

**文件**：`web/src/features/system/AccountSecurity.tsx`（修改）

- [ ] 接入 `authApi.getMe()` 获取账号信息
- [ ] 修改密码按钮打开 Layout 已有的 `ChangePasswordModal`
- [ ] 处理 loading/error 状态

**验证**：`cd web && pnpm typecheck && pnpm lint`

> 注：账号与安全分组无需新增后端代码，复用已有认证端点。

---

## Phase 3: 存储与保留策略

### Step 3.1: StorageCleaner 改造

**文件**：`crates/pipeline/src/storage_cleaner/mod.rs`（修改）

- [ ] 将 `config` 字段改为 `Arc<RwLock<StorageCleanerConfig>>`
- [ ] 实现 `update_config()` 方法
- [ ] 实现 `get_status()` 方法（磁盘状态、健康等级、各类型统计）
- [ ] 实现 `get_config()` 方法
- [ ] 实现 `set_config()` 方法（校验 + 更新 + 持久化）
- [ ] 实现 `trigger_cleanup()` 方法
- [ ] 确保 `clean_if_needed()` 每次读取最新配置
- [ ] 添加配置校验（水位层级关系、配额非负等）

**验证**：`cargo test -p pipeline`

### Step 3.2: 后端 API 端点

**文件**：`crates/api/src/routes/system.rs`（修改）

- [ ] 实现 `GET /api/v1/system/storage/status`
- [ ] 实现 `GET /api/v1/system/storage/config`
- [ ] 实现 `PUT /api/v1/system/storage/config`
- [ ] 实现 `POST /api/v1/system/storage/cleanup`
- [ ] 所有端点调用 `StorageCleaner` 对应方法

**验证**：`cargo test -p api`

### Step 3.3: 数据库默认值

**文件**：SQL migration 或启动时初始化

- [ ] 首次启动时插入 `storage_config` 默认值（如不存在）
- [ ] 默认值：告警图 30 天、识别图 14 天、抓拍图 7 天、覆盖模式、自动清理开启

**验证**：`cargo test -p db`

### Step 3.4: 前端联调

**文件**：`web/src/features/system/hooks/use-storage-config.ts`（新建）、`web/src/features/system/StorageSettings.tsx`（修改）

- [ ] 创建 `useStorageConfig` hook（读取 status + config）
- [ ] 修改 `StorageSettings` 使用 hook 数据
- [ ] 保存调用 `systemApi.updateStorageConfig()`
- [ ] 手动清理调用 `systemApi.triggerCleanup()`
- [ ] 处理 dirty/saving/success/error 状态
- [ ] 开关控件使用新 CSS 类（已完成）

**验证**：`cd web && pnpm typecheck && pnpm lint`

---

## Phase 4: 对时服务

### Step 4.1: TimeService 实现

**文件**：`crates/pipeline/src/time_service/mod.rs`（新建）、`crates/pipeline/src/time_service/timedatectl.rs`（新建）

- [ ] 定义 `TimeService` trait
- [ ] 实现 `TimedatectlService`（通用 systemd 入口）
- [ ] `get_status()`：读取系统时间、时区、NTP 同步状态
- [ ] `get_config()`：读取 NTP 配置（从 DB）
- [ ] `update_config()`：更新时区/NTP/服务器
- [ ] `force_sync()`：执行 `timedatectl set-ntp yes`
- [ ] `set_system_time()`：安全约束 + `timedatectl set-time` + 操作日志
- [ ] chrony 深度适配（可选，P1）

**验证**：`cargo test -p pipeline`

### Step 4.2: 后端 API 端点

**文件**：`crates/api/src/routes/system.rs`（修改）

- [ ] 实现 `GET /api/v1/system/time/status`
- [ ] 实现 `GET /api/v1/system/time/config`
- [ ] 实现 `PUT /api/v1/system/time/config`
- [ ] 实现 `POST /api/v1/system/time/sync`
- [ ] 实现 `POST /api/v1/system/time/set`（高风险端点，需审计日志）

**验证**：`cargo test -p api`

### Step 4.3: 数据库默认值

- [ ] 首次启动时插入 `time_config` 默认值（NTP 启用、pool.ntp.org、系统时区）

### Step 4.4: 前端联调

**文件**：`web/src/features/system/hooks/use-time-config.ts`（新建）、`web/src/features/system/TimeSettings.tsx`（修改）

- [ ] 创建 `useTimeConfig` hook
- [ ] 修改 `TimeSettings` 使用 hook 数据
- [ ] 保存调用 `systemApi.updateTimeConfig()`
- [ ] 强制同步调用 `systemApi.forceSync()`
- [ ] 手动设置时间调用 `systemApi.setSystemTime()`
- [ ] NTP 禁用时显示警告（已完成 UI）
- [ ] 开关控件使用新 CSS 类（已完成）

**验证**：`cd web && pnpm typecheck && pnpm lint`

---

## Phase 5: 网络配置

### Step 5.1: NetworkService trait 与适配器

**文件**：`crates/pipeline/src/network/mod.rs`（新建）、`crates/pipeline/src/network/nm.rs`（新建）、`crates/pipeline/src/network/networkd.rs`（新建）

- [ ] 定义 `NetworkService` trait
- [ ] 实现 `NmService`（NetworkManager 适配）
  - [ ] `list_interfaces()`：解析 `nmcli -t -f DEVICE,TYPE,STATE,CONNECTION device status`
  - [ ] `get_interface()`：查询具体网卡 IP 配置
  - [ ] `update_interface()`：通过 `nmcli con mod` + `nmcli con up` 修改
  - [ ] 管理接口检测：匹配 Heimdall 监听地址
- [ ] 实现 `NetworkdService`（systemd-networkd 适配）
  - [ ] 解析 `.network` 配置文件
  - [ ] 通过 `networkctl` 管理
- [ ] 实现 `detect_service()` 工厂函数：启动时检测可用适配器
- [ ] 未识别管理服务的网卡：返回只读信息

**验证**：`cargo test -p pipeline`

### Step 5.2: 网络操作状态管理

**文件**：`crates/pipeline/src/network/operation.rs`（新建）

- [ ] 实现 `NetworkOperationManager`
  - [ ] 创建操作记录（备份当前配置）
  - [ ] 查询待确认操作
  - [ ] 确认操作
  - [ ] 取消操作
  - [ ] 超时恢复（30s 定时器）
- [ ] 操作记录持久化到 `system_configs` 表
- [ ] 进程重启后恢复未完成操作

**验证**：`cargo test -p pipeline`

### Step 5.3: 后端 API 端点

**文件**：`crates/api/src/routes/system.rs`（修改）

- [ ] 实现 `GET /api/v1/system/network/interfaces`
- [ ] 实现 `GET /api/v1/system/network/interfaces/:name`
- [ ] 实现 `PUT /api/v1/system/network/interfaces/:name`
- [ ] 实现 `GET /api/v1/system/network/changes/pending`
- [ ] 实现 `POST /api/v1/system/network/changes/:id/confirm`
- [ ] 实现 `POST /api/v1/system/network/changes/:id/cancel`
- [ ] 管理接口变更返回操作记录 + 新地址候选
- [ ] 非管理接口变更直接应用并返回成功

**验证**：`cargo test -p api`

### Step 5.4: 前端联调

**文件**：`web/src/features/system/hooks/use-network.ts`（新建）、`web/src/features/system/NetworkSettings.tsx`（修改）

- [ ] 创建 `useNetwork` hook（网卡列表 + pending 操作）
- [ ] 修改 `NetworkSettings` 使用 hook 数据
- [ ] 编辑表单校验（IP 格式、前缀范围、网关、DNS）
- [ ] 保存调用 `systemApi.updateNetworkInterface()`
- [ ] 管理接口变更弹出确认对话框
- [ ] 确认后显示断连指引面板
- [ ] 新地址登录后自动恢复操作上下文

**验证**：`cd web && pnpm typecheck && pnpm lint`

### Step 5.5: Layout 集成

**文件**：`web/src/app/layout.tsx`（修改）

- [ ] 登录后查询 `GET /api/v1/system/network/changes/pending`
- [ ] 若有待确认操作，提示用户并切换到网络设置分组
- [ ] 不自动确认，仅展示状态

**验证**：`cd web && pnpm typecheck && pnpm lint`

---

## Phase 6: 集成验证

### Step 6.1: 端到端流程测试

- [ ] 系统概览：加载真实系统信息，NPU 不可用时显示"不可用"
- [ ] 账号安全：查看账号信息，修改密码后重新登录
- [ ] 存储配置：修改保留天数/配额，保存后刷新页面保持一致
- [ ] 手动清理：触发清理，观察磁盘使用率变化
- [ ] 对时服务：修改时区，强制 NTP 同步
- [ ] 网络配置：查看网卡列表，修改非管理网卡 IP
- [ ] 管理网卡变更：完整流程（提交 → 断连 → 新地址登录 → 确认）
- [ ] 超时恢复：管理网卡变更后不确认，等待自动恢复

### Step 6.2: 错误路径测试

- [ ] 非法 IP 地址返回 51001
- [ ] 网卡不支持修改返回 51005
- [ ] 已有进行中操作返回 51006
- [ ] 时间差超过一年返回 51201
- [ ] 存储配置校验失败返回 51100
- [ ] 所有错误返回统一信封 `{ code, message: null, data: null, timestamp }`

### Step 6.3: 三语与主题

- [ ] zh-CN / zh-TW / en 三语切换正常
- [ ] 明暗主题下所有设置页面显示正确
- [ ] 错误信息三语完整

### Step 6.4: 无障碍

- [ ] 所有表单有关联 label/id
- [ ] 开关控件有 `role="switch"` + `aria-checked`
- [ ] 确认弹窗有 `role="dialog"` + `aria-modal` + focus trap
- [ ] 键盘导航可达所有交互元素
- [ ] focus-visible 环可见

---

## Phase 7: 质量门禁

### Step 7.1: Rust

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

### Step 7.2: Web

```bash
cd web
pnpm format
pnpm lint          # eslint --max-warnings=0
pnpm typecheck     # tsc --noEmit
pnpm test          # vitest run
pnpm build
```

### Step 7.3: 交付确认

- [ ] 所有验证命令通过
- [ ] 无 `dbg!`、`println!`、`console.log`、`todo!()`
- [ ] 无 `@ts-ignore`
- [ ] 无硬编码颜色值（使用 CSS 变量）
- [ ] 所有可见文本接入 i18n

---

## 依赖关系

```
Phase 0 (基础设施)
  ├── Step 0.1 → 0.2 → 0.3 → 0.4 (顺序执行)
  │
Phase 1-4 (可并行)
  ├── Phase 1 (系统概览) → 依赖 0.1, 0.2, 0.3, 0.4
  ├── Phase 2 (账号安全) → 依赖 0.3, 0.4 (复用已有后端)
  ├── Phase 3 (存储策略) → 依赖 0.1, 0.2, 0.3, 0.4
  └── Phase 4 (对时服务) → 依赖 0.1, 0.2, 0.3, 0.4
  │
Phase 5 (网络配置)
  └── 依赖 Phase 0 (最复杂，最后执行)
  │
Phase 6 (集成验证) → 依赖 Phase 1-5 全部完成
Phase 7 (质量门禁) → 依赖 Phase 6
```

## 回滚点

| 阶段 | 回滚策略 |
|------|---------|
| Phase 0 | 删除新增文件，恢复 lib.rs/mod.rs |
| Phase 1-4 | 各分组独立回滚，不影响其他分组 |
| Phase 5 | 网络适配器可独立禁用（返回"不支持"） |
| Phase 6-7 | 问题定位后局部修复 |

## 预估工作量

| 阶段 | 预估 | 说明 |
|------|------|------|
| Phase 0 | 2-3h | 基础设施搭建 |
| Phase 1 | 2-3h | 系统概览（涉及 /proc 解析） |
| Phase 2 | 0.5h | 账号安全（复用已有） |
| Phase 3 | 3-4h | 存储策略（改造 StorageCleaner） |
| Phase 4 | 2-3h | 对时服务（timedatectl 适配） |
| Phase 5 | 5-7h | 网络配置（最复杂，含跨 IP 恢复） |
| Phase 6 | 2-3h | 集成验证 |
| Phase 7 | 1h | 质量门禁 |
| **总计** | **18-24h** | |
