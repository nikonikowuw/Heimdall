# PRD: API 层架构重构

## 背景

`crates/api` 经过业务迭代，部分文件承载了过多职责，影响可维护性：

1. `state.rs` 既是状态容器又包含 5 个业务方法（探活、广播、认证同步），违反单一职责
2. `algo.rs` 约 700 行，混合了路由处理、归档解压、沙箱校验、RAII 守卫
3. `system.rs` 约 350 行，聚合了 overview/network/storage/time 四个独立子域

## 目标

在**零业务语义变化**的前提下，通过纯结构重组提升可维护性。

## 改动范围

### 1. state.rs 瘦身

- 将 `probe_single_camera`、`start_periodic_probe_worker`、`update_and_broadcast_probe` 移至独立 `CameraProbeService`
- 将 `sync_auth_state` 调用下沉至 `app` 层初始化逻辑
- `AppState` 退化为纯数据持有者 + 轻量工具方法

### 2. algo.rs 拆分

- 归档解压逻辑（ZIP/TAR/TAR.GZ 格式检测 + Tar Slip 安全校验 + RAII 守卫）→ `algo/archive.rs`
- 沙箱同步处理（`process_uploaded_package_archive_sync` + spawn_blocking 调度）→ `algo/upload_service.rs`
- 路由 handler + DTO 定义 → 保留 `routes/algo.rs`

### 3. system.rs 拆分

按子域拆分为：
- `routes/system/mod.rs` — Router 拼装
- `routes/system/overview.rs` — 系统全景概览
- `routes/system/network.rs` — 网卡配置管理
- `routes/system/storage.rs` — 存储保留策略
- `routes/system/time.rs` — 对时服务

### 4. 小修补

- `routes/mod.rs`：为 `route_layer` 注册顺序加注释说明洋葱模型
- `evidence.rs`：证据图片服务添加 access log
- `ws.rs`：WebSocket Lagged 与 Closed 错误区分处理

## 约束

- 零新功能、零行为变化（小修补除外）
- 所有现有测试必须继续通过
- 不改动 `lib.rs` 的 `create_app()` 签名
- 不改动 `types`、`db`、`media`、`infer`、`pipeline` crate

## 验收标准

1. `cargo fmt --all && cargo clippy --all-targets -- -D warnings` 全绿
2. `cargo test --workspace` 全绿
3. `routes/algo.rs` < 200 行
4. `routes/system.rs`（如保留）仅做 mod 拼装，< 50 行
5. `state.rs` 无 async fn（纯数据 + 同步工具方法）
