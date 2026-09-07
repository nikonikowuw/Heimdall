# Implementation Plan: Axum 审计日志中间件统一拦截改造

## 执行步骤

### Step 1: 创建审计中间件核心实现

**文件**: `crates/api/src/middleware/audit.rs`（新建）

- [ ] 定义 `AuditUser(pub String)` struct（用于 extension 注入 username）
- [ ] 定义 `AuditLogLayer` with `new(db, log_writes_only)` constructor
- [ ] 实现 `Layer<S>` for `AuditLogLayer`
- [ ] 定义 `AuditLogService<S>` struct
- [ ] 实现 `Service<Request>` for `AuditLogService<S>`
  - 记录 `Instant::now()`
  - 调用 `self.inner.call(req).await`
  - 计算 `duration_ms`
  - 检查 method 是否为写操作
  - 提取 client IP（X-Forwarded-For → X-Real-IP → ConnectInfo → "unknown"）
  - 提取 User-Agent
  - 从 path + method 推断 module/action
  - 从 request extensions 读取 `AuditUser` 获取 username
  - `tokio::spawn` 异步执行 `OplogRepo::record()`
  - 返回 response
- [ ] 实现 `extract_client_ip()` 工具函数
- [ ] 实现 `infer_module_action()` 推断函数
- [ ] 实现 `is_write_method()` 判断函数

### Step 2: 修改 require_auth 注入 AuditUser

**文件**: `crates/api/src/middleware/auth.rs`（修改）

- [ ] 在 `require_auth` 函数中，认证成功后插入 `AuditUser(username)` 到 `request.extensions_mut()`
- [ ] 确保不影响现有 `AuthUser` extractor 的工作

### Step 3: 注册审计中间件到 Router

**文件**: `crates/api/src/routes/mod.rs`（修改）

- [ ] 在 `api_router` 中，于 `require_auth` route_layer 之后添加 `audit_log` route_layer
- [ ] 确认 Axum 中间件执行顺序正确（audit 在 auth 之后执行）

### Step 4: 导出中间件模块

**文件**: `crates/api/src/middleware/mod.rs`（修改）

- [ ] 添加 `pub mod audit;`
- [ ] 确保 `AuditUser` 和 `audit_log` 函数可从 middleware 模块访问

### Step 5: 删除 handler 中的手动审计插桩

**文件**: `crates/api/src/routes/camera.rs`（修改）
- [ ] 删除 `create_camera` 中的 `OplogRepo::record()` 调用
- [ ] 删除 `update_camera` 中的 `OplogRepo::record()` 调用
- [ ] 删除 `delete_camera` 中的 `OplogRepo::record()` 调用
- [ ] 删除 `use db::OplogRepo;` 导入（如果不再需要）

**文件**: `crates/api/src/routes/task.rs`（修改）
- [ ] 删除 `update_task` 中的 `OplogRepo::record()` 调用
- [ ] 删除 `delete_task` 中的 `OplogRepo::record()` 调用
- [ ] 删除 `create_instance` 中的 `OplogRepo::record()` 调用
- [ ] 删除 `update_instance` 中的 `OplogRepo::record()` 调用
- [ ] 删除 `set_instance_enabled` 中的 `OplogRepo::record()` 调用
- [ ] 删除 `delete_instance` 中的 `OplogRepo::record()` 调用
- [ ] 清理 `use db::OplogRepo;` 导入

**文件**: `crates/api/src/routes/algo.rs`（修改）
- [ ] 删除 `upload_package` 中的 `OplogRepo::record()` 调用
- [ ] 删除 `activate_version` 中的 `OplogRepo::record()` 调用
- [ ] 删除 `uninstall_version` 中的 `OplogRepo::record()` 调用
- [ ] 清理 `use db::OplogRepo;` 导入

### Step 6: 增强 auth 模块审计字段

**文件**: `crates/api/src/routes/auth.rs`（修改）
- [ ] 创建公共工具函数 `extract_client_ip_from_parts()` 和 `extract_user_agent_from_parts()`（或复用 audit.rs 中的实现）
- [ ] 在 `initialize` handler 中提取 IP/UA 并填入审计调用
- [ ] 在 `login` handler 中提取 IP/UA 并填入审计调用
- [ ] 在 `change_password` handler 中提取 IP/UA 并填入审计调用
- [ ] 在 `logout` handler 中提取 IP/UA 并填入审计调用

### Step 7: 前端 OplogPage 接入真实 API

**文件**: `web/src/features/oplog/OplogPage.tsx`（重写）
- [ ] 添加 `useState` / `useEffect` 状态管理
- [ ] 实现 `fetchLogs(module?, offset?)` 函数调用 `GET /api/v1/logs/operations`
- [ ] 实现 module 下拉筛选
- [ ] 实现刷新按钮功能
- [ ] 实现分页加载（加载更多按钮或滚动加载）
- [ ] 保持现有表格布局和样式
- [ ] 时间格式化为本地化时间
- [ ] 状态码着色（2xx 绿 / 4xx 黄 / 5xx 红）

### Step 8: 验证与测试

- [ ] `cargo fmt --all`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cd web && pnpm lint && pnpm typecheck`
- [ ] 手动验证：创建摄像头 → 检查 oplog 表自动产生 camera/create 记录
- [ ] 手动验证：更新告警状态 → 检查 oplog 表自动产生 alarm/update_status 记录
- [ ] 手动验证：前端 OplogPage 展示真实数据

## 验证门禁

```bash
# Rust
cargo fmt --all && cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace

# Web
cd web && pnpm lint && pnpm typecheck
```

## Execution Notes

- 已完成 `AuditLogLayer` / `AuditLogService`，默认只审计受保护写请求，异步写入并保留 2048 UTF-8 字符 body 上限。
- 已将 `require_auth` 注入 `AuditUser`，并删除 camera、task、algorithm handler 的手工审计；auth 手工审计继续脱敏并补充真实 IP、User-Agent、duration。
- 已使用 Axum `OriginalUri` 保留嵌套路由完整路径，并在生产 server 注入 `ConnectInfo<SocketAddr>`。
- 已将 `OplogPage` 接入真实 API，支持 module 筛选、有限 offset 分页、刷新、加载/错误/空状态和本地化时间。
- 验证通过：`cargo clippy --all-targets -- -D warnings`、`cargo test --workspace`、`pnpm --dir web format`、`pnpm --dir web typecheck`、`pnpm --dir web lint`、`pnpm --dir web test`、`pnpm --dir web build`。
- 代码审查缺陷修复完成：
  - 移除受保护路由 handler（camera/task/algorithm/alarm）中未使用的 `_user: AuthUser` 死参数，并在 `AuthUser::from_request_parts` 增加 extensions 快速路径，杜绝重复 JWT 验签；
  - 移除 `AuditLogService` 中针对 `/auth` 的死分支 `is_manual_auth_audit_path`；
  - 前端 `useOplogs` 统一提取 `executeFetch`，消除 Promise 重复代码；
  - 修复 `getErrorMessage` 遇到非 Error 异常时的非空兜底保证，确保错误横幅与重试按钮在任何异常时正常渲染；
  - `mergeLogs` 实现基于 `log.id` 的增量分页去重，避免高频写入时的 React key 重复与多重行渲染问题；
  - 新增 `web/src/features/oplog/hooks/useOplogs.test.ts` 单元测试，全绿通过。
- 未执行真实摄像头、告警状态和浏览器手动验收；当前环境仅完成自动化路由、数据库和前端构建测试。
