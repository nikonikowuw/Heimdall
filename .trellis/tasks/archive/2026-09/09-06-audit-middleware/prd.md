# PRD: Axum 审计日志中间件统一拦截改造

## 背景

当前系统采用**手动插桩模式**记录审计日志——在每个 handler 函数内部手动调用 `OplogRepo::record()`。经全量排查发现以下问题：

1. **覆盖缺口**：告警状态变更（`PUT /alarms/{id}/status`）等核心写操作完全缺失审计；system 模块预留写端点未布设审计
2. **上下文丢失**：所有审计记录的 `ip`、`user_agent`、`duration_ms`、`query` 字段均为空字符串/零值
3. **架构脆弱**：每新增一个写操作端点必须人工记得添加审计调用，遗漏不可检测
4. **前端断裂**：`OplogPage.tsx` 为硬编码假数据，从未调用 `GET /api/v1/logs/operations` API

## 目标

将审计日志从"手动插桩"升级为"中间件统一拦截"，确保所有受保护的写操作端点自动产生完整审计记录。

## 约束

- 不改变现有 `operation_logs` 数据库表结构
- 不改变现有 `OplogRepo::record()` 签名（保持向后兼容）
- 审计中间件只挂载在受保护路由（`require_auth` 中间件之后），不审计公开端点（auth 初始化、登录）
- 审计写入失败不阻塞业务响应（与当前 `let _ = ...` 行为一致）
- 手动插桩的 auth 模块审计保留（登录/改密等需要脱敏处理，不适合中间件统一处理）

## 需求

### 后端

1. 创建 `AuditLog` Axum 中间件（Layer + Service），拦截所有经过该层的 HTTP 请求
2. 中间件自动从请求中提取：method、path、query_string、client IP（从 `X-Forwarded-For` / `X-Real-IP` / socket peer addr 依次解析）、User-Agent
3. 中间件在 handler 执行后记录：status_code、duration_ms（毫秒精度）
4. 对于写操作（POST / PUT / PATCH / DELETE），调用 `OplogRepo::record()` 写入 `operation_logs` 表
5. 对于读操作（GET / HEAD / OPTIONS），默认不记录（避免高频读请求淹没审计日志）；可通过配置开启
6. 审计日志的 `body` 字段：对于写操作，截取前 2048 字符的请求体（避免超大 payload 撑爆存储）
7. 审计日志的 `module` / `action` 字段：从 path 中自动推断（如 `/api/v1/cameras` → module=camera, action=create/update 等，结合 method 推断）
8. 删除各 handler 中现有的手动 `OplogRepo::record()` 调用（auth 模块的脱敏审计除外）
9. 修复 auth 模块手动审计中的空字段问题（IP、UA、duration）

### 前端

10. 将 `OplogPage.tsx` 从硬编码假数据改为调用 `GET /api/v1/logs/operations` API 拉取真实数据
11. 支持按 module 筛选、分页加载、刷新操作
12. 表格展示与现有 mock 布局保持一致（列：用户名、模块、操作、方法、路径、IP、状态、耗时、时间）

## 验收标准

- [ ] 所有受保护写操作端点（camera CRUD、task CRUD、algorithm upload/activate/uninstall、alarm status update）自动产生审计日志
- [ ] 审计记录包含真实 client IP、User-Agent、duration_ms
- [ ] auth 模块的登录/改密/登出审计保留手动脱敏逻辑，且补充真实 IP/UA/duration
- [ ] `OplogPage.tsx` 展示真实后端数据，支持 module 筛选和刷新
- [ ] 现有测试全部通过，新增审计中间件单元测试
- [ ] `cargo clippy --all-targets -- -D warnings` 无警告
