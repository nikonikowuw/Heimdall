# Execution Plan: User Authentication & Password Management

## 1. 任务分工与实施阶段

```
Phase 1: 共享类型与数据持久层 (crates/types & crates/db)
Phase 2: 密码学机制与 API 服务层 (crates/api & crates/app)
Phase 3: 前端控制台对接与状态管理 (web/)
Phase 4: 全链路测试与门禁验证
```

---

## 2. 实施清单

### Step 1: 共享数据类型定义 (`crates/types`)
- [ ] 在 `crates/types/src/lib.rs` 中暴露 `pub mod auth;`
- [ ] 创建 `crates/types/src/auth.rs`：
  - 定义 `AdminUser` 领域实体
  - 定义 `LoginRequest`, `LoginResponse`, `ChangePasswordRequest`, `AdminUserDto`
  - 定义 `AuthClaims` (JWT payload)
- 验证：`cargo test -p types`

### Step 2: 数据库表结构与 Repository (`crates/db`)
- [ ] 更新 `crates/db/src/schema.rs`，添加 `admin_users` 表 DDL 与索引
- [ ] 创建 `crates/db/src/entity/admin_user.rs`（SeaORM Entity）
- [ ] 创建 `crates/db/src/repository/admin_user.rs`：
  - `ensure_default_admin(db, default_password_hash)` 首次开机幂等初始化
  - `find_by_username(db, username)`
  - `update_password(db, username, new_hash, now_ms)`
  - `invalidate_tokens(db, username, now_ms)`
- [ ] 在 `crates/db/src/lib.rs` 暴露相关模块
- 验证：`cargo test -p db`

### Step 3: API 服务、密码学与中间件 (`crates/api` & `crates/app`)
- [ ] 密码学支持：
  - 基于 PBKDF2-HMAC-SHA256 或加盐哈希的密码哈希生成与校验
  - 基于 HMAC-SHA256 的 JWT 编码与解码
- [ ] 扩展 `AppState`：
  - 加入 `jwt_secret`
  - 加入 `token_invalid_before: Arc<AtomicI64>` 内存极速缓存
- [ ] 编写 `crates/api/src/middleware/auth.rs`：
  - Bearer Token 解析
  - 过期判断与撤销时间戳核验
  - `AuthUser` Axum Extractor
- [ ] 编写 `crates/api/src/routes/auth.rs`：
  - `GET /api/v1/auth/init-status`
  - `POST /api/v1/auth/initialize`
  - `POST /api/v1/auth/login`
  - `PUT /api/v1/auth/password`
  - `GET /api/v1/auth/me`
  - `POST /api/v1/auth/logout`
- [ ] 在 `crates/api/src/routes/oplog.rs` / 审计拦截中增加密码脱敏逻辑
- [ ] 挂载路由到 `crates/api/src/routes/mod.rs`
- [ ] 在 `crates/app/src/main.rs` 启动时检查：若配置了 `ARGUS_ADMIN_PASSWORD` 则静默初始化，否则保留未初始化状态
- 验证：`cargo test -p api && cargo test -p app`

### Step 4: React 前端控制台对接 (`web/`)
- [ ] 封装 `web/src/lib/api.ts`（统一 fetch，带 Bearer Token，401 自动触发 store.logout）
- [ ] 新增 `web/src/features/auth/SetupPage.tsx`（开箱向导视图，支持自定义账号与密码）
- [ ] 更新 `web/src/features/auth/LoginPage.tsx`：挂载 init-status 检查，未初始化时切换到 SetupPage；登录接入真实 `api.login` 请求，处理错误 toast
- [ ] 新增 `web/src/components/ChangePasswordModal.tsx`，并在全局导航栏提供入口
- 验证：`cd web && pnpm test && pnpm typecheck && pnpm lint`

### Step 5: 全链路质量门禁验证
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cd web && pnpm build`
