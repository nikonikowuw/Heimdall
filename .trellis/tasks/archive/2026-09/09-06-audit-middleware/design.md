# Design: Axum 审计日志中间件统一拦截改造

## 架构概览

```
HTTP Request
    │
    ▼
┌──────────────────────┐
│   require_auth       │  ← 提取 AuthUser（username）
└──────────┬───────────┘
           │
           ▼
┌──────────────────────┐
│   audit_log (NEW)    │  ← 拦截请求，提取 IP/UA/method/path/query
│                      │    在 handler 执行后记录 status/duration
│                      │    对写操作调用 OplogRepo::record()
└──────────┬───────────┘
           │
           ▼
      Handler (业务逻辑)
```

## 核心设计

### 1. 中间件位置

审计中间件挂载在 `require_auth` 之后、业务 handler 之前。这意味着：

- 只有通过认证的请求才会被审计（公开端点如 `/auth/initialize`、`/auth/login` 不经过此层）
- `AuthUser` 已经在上游被提取，handler 可以信任用户身份
- 但中间件本身不依赖 `AuthUser`（通过 `http::request::Parts` 提取 username 的方式不可行，因为 `AuthUser` 是 handler 层 extractor）

**关键决策**：审计中间件在 Layer 层工作，无法直接访问 handler 层的 `AuthUser` extractor 结果。解决方案：

- 方案 A（采用）：在 `require_auth` 中间件执行后，将 username 注入到 request extensions 中。审计中间件从 extensions 中读取。
- 方案 B（备选）：审计中间件不记录 username，只记录 IP/path/等。但审计日志缺少操作者信息不可接受。

### 2. Extension 注入 Username

修改 `require_auth` 中件件，在认证成功后将 username 存入 `request.extensions_mut().insert(AuditUser(username.clone()))`。审计中间件通过 `req.extensions().get::<AuditUser>()` 读取。

定义新类型：
```rust
#[derive(Clone)]
pub struct AuditUser(pub String);
```

### 3. 中间件实现

采用 Axum 标准的 `Layer + Service` 模式：

```rust
// crates/api/src/middleware/audit.rs

pub struct AuditLogLayer {
    db: DatabaseConnection,
    log_writes_only: bool,  // 默认 true，只记录写操作
}

pub struct AuditLogService<S> {
    inner: S,
    db: DatabaseConnection,
    log_writes_only: bool,
}
```

`poll_ready` / `call` 实现：
1. 记录请求开始时间 `Instant::now()`
2. 调用 `self.inner.call(req).await` 获取 response
3. 计算 `duration_ms = start.elapsed().as_millis() as i64`
4. 检查 method 是否为写操作（POST/PUT/PATCH/DELETE）
5. 如果 `log_writes_only == true` 且 method 为 GET/HEAD/OPTIONS，直接返回 response
6. 否则，从 request 提取：path、query_string、client IP、User-Agent
7. 尝试从 request body 提取 body（需要 `axum::body::to_bytes` 但 body 可能已被消费——见下文）
8. 异步 spawn 一个 task 执行 `OplogRepo::record()`（不阻塞当前请求）
9. 返回 response

### 4. Request Body 提取问题

Axum 的 body 在 handler 消费后不可再次读取。中间件处于 handler 上游，理论上可以在 handler 执行前缓存 body，但这会增加内存开销且违背零拷贝原则。

**决策**：审计中间件的 `body` 字段设为空字符串。body 内容对审计来说不是关键信息（主要审计 who/what/when/where），且避免了 body 缓存的复杂性。现有手动插桩的 body 也是 `req_json`（请求 DTO 的序列化），实际上审计价值有限。

> **例外**：auth 模块保留手动插桩（带脱敏 body），因为登录/改密的 body 审计需要密码脱敏处理。

### 5. Client IP 解析

按优先级依次尝试：
1. `X-Forwarded-For` header（取第一个 IP，适用于反向代理场景）
2. `X-Real-IP` header
3. `SocketAddr` peer address（从 `connect_info` extension 中提取）

```rust
fn extract_client_ip(req: &http::request::Parts) -> String {
    if let Some(xff) = req.headers.get("x-forwarded-for") {
        if let Ok(s) = xff.to_str() {
            if let Some(first) = s.split(',').next() {
                return first.trim().to_string();
            }
        }
    }
    if let Some(xri) = req.headers.get("x-real-ip") {
        if let Ok(s) = xri.to_str() {
            return s.trim().to_string();
        }
    }
    req.extensions
        .get::<axum::extract::connect_info::ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}
```

### 6. Module/Action 自动推断

从 path 和 method 推断 module 和 action：

| Path 前缀 | Module | Method → Action |
|-----------|--------|-----------------|
| `/cameras` | camera | POST=create, PUT=update, DELETE=delete |
| `/tasks/instances` | task_instance | POST=create, PUT=update, DELETE=delete |
| `/tasks` | task | PUT=update_rules, DELETE=delete |
| `/algorithms` | algorithm | POST=upload, PUT=activate, DELETE=uninstall |
| `/alarms` | alarm | PUT=update_status |

对于未匹配的路径，module 取 path 第一段，action 取 method 映射（POST→create, PUT→update, DELETE=delete）。

### 7. 手动审计清理

以下 handler 中的手动 `OplogRepo::record()` 调用将被删除：

- `camera.rs`：create_camera、update_camera、delete_camera（3 处）
- `task.rs`：update_task、delete_task、create_instance、update_instance、set_instance_enabled、delete_instance（6 处）
- `algo.rs`：upload_package、activate_version、uninstall_version（3 处）

以下保留不动（auth 模块需要脱敏处理）：
- `auth.rs`：initialize、login、change_password、logout（4 处）——但补充 IP/UA/duration 字段

### 8. Auth 模块审计增强

现有 auth 审计保留手动调用模式（因为需要密码脱敏），但增强字段填充：

- 通过新增一个 `extract_client_ip_from_request` 公共工具函数获取 IP/UA
- 将 `duration_ms` 设为实际耗时（需要在 handler 入口记录 start time）
- 将 `ip` 和 `user_agent` 字段填充真实值

### 9. 前端改造

`OplogPage.tsx` 改造为：

1. 使用 `useEffect` + `useState` 调用 `GET /api/v1/logs/operations?limit=50&offset=0`
2. 支持 module 下拉筛选（复用现有 `?module=xxx` query param）
3. 刷新按钮触发重新拉取
4. 时间格式化为本地时间
5. 状态码着色（2xx 绿色、4xx 黄色、5xx 红色）

### 10. Router 组装变更

```rust
// crates/api/src/routes/mod.rs

pub fn api_router(state: &AppState) -> Router<AppState> {
    let protected = Router::new()
        .nest("/cameras", camera::router())
        // ... 其他路由 ...
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::require_auth,
        ))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::audit_log,  // NEW: 审计层在 auth 层之后
        ));
    // ...
}
```

> 注意：Axum 的 `route_layer` 执行顺序是**从下到上**（最后添加的先执行）。所以 `audit_log` 需要放在 `require_auth` 之后添加，这样它会在 `require_auth` 之后执行（即在更靠近 handler 的位置）。

## 兼容性

- `operation_logs` 表结构不变
- `OplogRepo::record()` API 不变
- `GET /api/v1/logs/operations` 端点不变
- 现有 auth 模块审计完全保留（只增强字段）
- 现有测试中审计相关断言（auth 测试中的 oplog 验证）保持通过

## 风险与回滚

- **风险**：审计中间件的 DB 写入在高并发场景下可能成为瓶颈。缓解：使用 `tokio::spawn` 异步写入，不阻塞请求。
- **回滚**：如需回滚，删除 `audit_log` route_layer 和 `AuditUser` extension 注入，恢复手动插桩即可。所有手动插桩代码在清理前先注释而非删除。
