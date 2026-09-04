# User Authentication and Password Management (用户认证与密码管理模块)

## 1. Goal (目标)

依据 `@prd/prd-v1.0.md` 第 5.1 节「身份认证与安全审计中心」规范，在当前 Rust + React 架构中实现**极简单管理员轻量认证与安全控制体系**：
- 建立基于 SQLite `admin_users` 表的单管理员账户持久化体系（首次启动自动安全初始化）；
- 实现基于加盐安全哈希的密码存储与校验，以及基于 JWT 的无状态 Token 签发与基于时间戳的高效即时失效机制；
- 提供完整的 RESTful 认证接口（`login`, `password`, `me`, `logout`）；
- 构建统一的 Axum JWT 鉴权中间件与当前用户提取器，对受控接口实施强制认证防护；
- 打通与全局操作日志（Operation Logs）的脱敏联动，杜绝明文密码泄露；
- 完成 React 控制台 `LoginPage` 的真实接口对接与修改密码功能，实现 401 自动失效与全局认证闭环。

---

## 2. Requirements (功能需求)

### 2.1 账户体系、双轨初始化与开箱向导 (Account & Setup Flow)
1. **单管理员模型**：维持精简的单管理员架构，服务端杜绝在代码逻辑中硬编码死 `"admin"`，登录时按传入的 `username` 动态检索数据库记录。
2. **严禁弱出厂密码，支持双轨安全初始化**：
   - **轨道 A（环境变量静默初始化）**：现场自动化部署时，若配置了环境变量 `ARGUS_ADMIN_PASSWORD`（可选 `ARGUS_ADMIN_USERNAME`，缺省为 `admin`），系统启动时自动完成加盐哈希落库，标记系统为已初始化状态。
   - **轨道 B（强制交互式开箱向导 OOBE）**：若未配置 `ARGUS_ADMIN_PASSWORD` 且数据库无管理员记录，**严禁使用任何出厂弱密码（如 `admin123`）静默初始化**。系统保持未初始化状态（`initialized: false`），强制前端导航至开箱初始化向导界面（Setup Wizard）。
3. **初始化接口与防重入**：
   - `GET /api/v1/auth/init-status`：公开接口，返回 `{ "initialized": bool }`。
   - `POST /api/v1/auth/initialize`：仅在 `initialized == false` 时允许调用，传入自定义管理员账号与高强度密码。成功落库后直接返回 JWT 访问令牌，前端无感直接登入监控主页；一旦系统已初始化，该接口永久锁死并返回 403 Forbidden。
4. **密码安全强度校验**：初始化与修改密码时均校验新密码长度不低于 6 位。

### 2.2 JWT 令牌与即时撤销 (JWT & Token Revocation)
1. **Token 结构**：采用 HMAC-SHA256 签发 JWT，Claims 包含 `sub` (用户名)、`iat` (签发 UTC 毫秒)、`exp` (过期 UTC 毫秒，默认 24 小时)。
2. **瞬时失效机制**：`admin_users` 记录持久化存储 `token_invalid_before` 毫秒时间戳。
   - 当发生**修改密码**或**主动登出**时，将 `token_invalid_before` 更新为当前 UTC 毫秒时间。
   - 鉴权中间件校验 JWT 时，不仅检查验签与 `exp`，还比对 `iat >= token_invalid_before`，实现无状态 Token 的 $O(1)$ 瞬时安全撤销。

### 2.3 核心 HTTP 接口 (Endpoints)
1. `POST /api/v1/auth/login` (公开)：管理员凭用户名和密码登录，成功返回 `accessToken`, `username`, `expiresAt`。
2. `PUT /api/v1/auth/password` (需鉴权)：传入 `oldPassword` 和 `newPassword`，校验原密码无误后更新，使旧 Token 全部失效。
3. `GET /api/v1/auth/me` (需鉴权)：获取当前登录管理员信息及状态。
4. `POST /api/v1/auth/logout` (需鉴权)：使当前管理员 Token 即时失效。

### 2.4 接口防护与中间件 (Middleware & Security)
1. **受控接口防护**：除白名单（`/api/v1/auth/login`, 静态资源, 探活接口）外，对所有 `/api/v1/*` 接口与受保护 WebSocket 路由实施 Bearer Token 校验。
2. **标准错误响应**：
   - 未携带 Token：HTTP 401，code: `10001`，message: `"未登录（缺少凭证）"`
   - Token 已过期：HTTP 401，code: `10002`，message: `"凭据已过期"`
   - Token 无效/被撤销：HTTP 401，code: `10003`，message: `"凭据无效或已被撤销"`
   - 账号密码错误：HTTP 400，code: `10007`，message: `"用户名或密码错误"`
   - 新密码强度不符：HTTP 400，code: `10008`，message: `"新密码长度不能少于 6 位"`
3. **敏感信息脱敏**：修改密码等请求体经过 `oplog` 记录时，`oldPassword` 与 `newPassword` 等字段自动脱敏为 `******`。
4. **服务端国际化 (i18n)**：支持基于 HTTP `Accept-Language` 请求头进行语言偏好协商（支持 `zh-CN`, `zh-TW`, `en`），服务端响应自动适配目标语言的错误与成功消息，响应头附带标准 `Content-Language`。

### 2.5 Web 控制台联调 (Web Console Integration)
1. `LoginPage.tsx`：移除本地模拟代码，调用后端真实 `POST /api/v1/auth/login` 接口，支持错误信息提示。
2. `useAuthStore`：持久化保存 `token`、`username`，提供带鉴权 Header 的 API 请求工具与 401 响应全局拦截重定向。
3. 控制台修改密码：在布局或界面中提供“修改密码”弹窗组件，调用 `PUT /api/v1/auth/password` 成功后提示并自动重新登录。

---

## 3. Acceptance Criteria (验收标准)

- [ ] 系统在未配置 `ARGUS_ADMIN_PASSWORD` 时保持 `initialized: false`，严禁预置任何明文弱密码。
- [ ] `GET /api/v1/auth/init-status` 正确反映系统是否已完成初始化。
- [ ] `POST /api/v1/auth/initialize` 首次调用成功初始化管理员并下发 Token，重复调用返回 403 永久锁死。
- [ ] 前端检测到未初始化状态时强制呈现开箱初始化向导视图，支持自定义账号与密码，提交后直接进入控制台。
- [ ] `POST /api/v1/auth/login` 正确凭证返回有效 JWT 和 200，错误凭据返回 400（错误码 10007）。
- [ ] 受保护路由（如 `GET /api/v1/auth/me`）无 Token 返回 401，带正确 Token 返回 200。
- [ ] `PUT /api/v1/auth/password` 原密码正确时成功修改密码，旧 Token 立即被拦截返回 401。
- [ ] `POST /api/v1/auth/logout` 执行后当前 Token 立即失效。
- [ ] 具备全链路国际化支持，`Accept-Language: en` 返回英文消息，`Accept-Language: zh-TW` 返回繁体中文消息，且附带 `Content-Language` 头。
- [ ] 前端 `LoginPage` 真实调用后端 API，登录成功正常跳转主控制台，登录失败提示错误。
- [ ] 后端通过 `cargo test --workspace`、`cargo clippy --all-targets -- -D warnings` 与 `cargo fmt --all -- --check`。
- [ ] 前端通过 `pnpm test`、`pnpm lint`、`pnpm typecheck` 与 `pnpm build`。
