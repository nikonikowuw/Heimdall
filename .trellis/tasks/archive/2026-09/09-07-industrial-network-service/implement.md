# 工业级网络服务配置 — 执行计划

## 执行原则

1. **工业级可靠第一**：任何网络操作失败不得导致系统崩溃或网络断死，管理网卡变更必须有可恢复的快照与独立看门狗。
2. **前后端契约对齐**：严格遵循 `api.md` 定义的数据结构与状态枚举。
3. **小步推进与门禁闭环**：每步完成后运行对应编译与测试，确保零编译告警。

---

## Phase 1: 契约与领域类型层对齐

### Step 1.1: 扩展 Rust 领域类型
**文件**：`crates/types/src/system.rs`
- 在 `NetworkInterface` 结构中增加 `carrier: Option<bool>`, `speed: Option<u32>`, `duplex: Option<String>`。
- 在 `IpConfig` 结构中增加 `metric: Option<u32>`。
- 派生必要的 `Serialize`, `Deserialize`, `Clone`, `Debug`。
- **验证**：`cargo check -p types`

### Step 1.2: 新增错误码与 i18n
**文件**：`crates/api/src/error.rs`, `crates/api/src/i18n/system.rs`
- 在 `ApiError` 新增 `NetworkIpConflict(String)` (51011) 和 `NetworkGatewayUnreachable(String)` (51012)。
- 补充 zh-CN, zh-TW, en 三语翻译。
- **验证**：`cargo test -p api`

### Step 1.3: 前端类型与 API 封装
**文件**：`web/src/types/system.ts`, `web/src/lib/system-api.ts`
- 同步 TypeScript 类型定义。
- 确保 `systemApi` 各接口与最新契约完全匹配。
- **验证**：`cd web && pnpm typecheck`

---

## Phase 2: 工业级网络核心子系统重构

### Step 2.1: 模块拆分与链路/管理口探测器
**文件**：`crates/api/src/network_service/detector.rs`
- 实现物理载波探测（读取 `/sys/class/net/<iface>/carrier`）。
- 实现协商速率与双工探测（`/sys/class/net/<iface>/speed`, `duplex`）。
- 实现管理网卡动态推导（检查监听 Socket 绑定 IP 与内核默认路由指向）。

### Step 2.2: RFC 5227 地址冲突检测 (ACD)
**文件**：`crates/api/src/network_service/arp.rs`
- 实现预下发 ARP Probe 探测（使用带 `LC_ALL=C` 的底层 arping 机制或标准报文解析）。
- 若检测到冲突提取冲突 MAC 并阻断。
- 成功绑定后广播 Gratuitous ARP。

### Step 2.3: Commit-Confirm 事务与独立看门狗管理器
**文件**：`crates/api/src/network_service/operation.rs`
- 实现 `NetworkOperationManager`。
- 快照原子化持久化（写入 `/run/heimdall/network_backup_<iface>.json`）。
- 启动基于 Tokio 的独立倒计时看门狗任务（60s 超时）。
- 实现 `confirm`、`cancel` 及超时自动 `rollback` 逻辑。
- 进程冷启动过期快照自动恢复检查。

### Step 2.4: 掉电安全与后端适配驱动
**文件**：`crates/api/src/network_service/backend/`
- NetworkManager (`nm.rs`)：系统命令全面强制注入 `LC_ALL=C` 和 `LANG=C`，精准解析 POSIX 字段。
- systemd-networkd (`networkd.rs`)：文件写入采用 `write tmp -> sync_all -> rename -> .bak` 掉电原子保护。

### Step 2.5: 统一服务门面组装
**文件**：`crates/api/src/network_service/mod.rs`
- 聚合 `detector`, `arp`, `operation`, `backend`。
- 实现与上层 Handler 的单点对外接口。
- **验证**：`cargo test -p api`

---

## Phase 3: 后端 HTTP 路由闭环

### Step 3.1: 实现 Handler 事务流转
**文件**：`crates/api/src/routes/system.rs`
- 完善 `get_pending_network_change` 查询。
- 完善 `confirm_network_change` 与 `cancel_network_change`。
- `update_network_interface` 区分管理口与非管理口流转。
- **验证**：`cargo test -p api`

---

## Phase 4: 前端交互与试运行体验升级

### Step 4.1: 试运行倒计时悬浮条
**文件**：`web/src/features/system/components/NetworkTrialBanner.tsx`
- 存在 `pendingOperation` 时显示吸顶高危预警横幅。
- 动态显示剩余秒数倒计时。
- 提供「确认永久生效」与「放弃并恢复原配置」快捷按钮。

### Step 4.2: 工业级网卡卡片展示
**文件**：`web/src/features/system/components/NetworkCard.tsx`
- 展示物理载波状态（绿色“已连接线缆” / 灰色“未插网线”）。
- 展示协商速率（如 1000 Mbps 全双工）。
- 路由 Metric 配置项（选填）。

### Step 4.3: 界面交互与冲突告警
**文件**：`web/src/features/system/NetworkSettings.tsx`
- 提交管理口 IP 弹出试运行风险说明对话框。
- 捕获 `51011` 错误时弹出精准的 IP 冲突告警（显示冲突 MAC）。
- **验证**：`cd web && pnpm typecheck && pnpm lint && pnpm build`

---

## Phase 5: 全量验证门禁

### Step 5.1: 后端门禁
```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

### Step 5.2: 前端门禁
```bash
cd web
pnpm format
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```
