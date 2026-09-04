# API 契约：用户认证与密码管理模块

- 任务：.trellis/tasks/09-04-user-auth-module
- 状态：confirmed
- 确认时间：2026-09-04
- 作者：主会话撰写，用户确认

---

## 1. Scope（目录边界）

| 侧 | 目录 | 规则 |
|---|---|---|
| frontend | `web/` | 前端实现域，不得改动后端源码 |
| backend | `crates/api/`, `crates/db/`, `crates/app/` | 后端实现域，不得改动前端源码 |
| shared | `crates/types/` | 跨层共享类型契约 |

---

## 2. Conventions（全局约定）

- **统一根路径**：`/api/v1`
- **数据格式**：所有请求与响应均为 JSON，字段严格使用 **`camelCase`**。
- **时间规范**：系统所有绝对时间戳均为 **13 位 UTC Unix 毫秒整数**。
- **统一响应信封**：
  ```json
  {
    "code": 0,
    "message": "success",
    "data": {},
    "timestamp": 1747584000000
  }
  ```
- **统一认证头**：`Authorization: Bearer <accessToken>`
- **国际化支持 (i18n)**：请求支持标准 `Accept-Language` 头（`zh-CN` / `en` / `zh-TW`），服务端中间件自动对 `ApiResponse.message` 进行三语本地化转换，并在响应头附带标准 `Content-Language`。
- **错误响应规范**：
  - 成功时 `code: 0`，`message: "success"`，`data: T`；
  - 错误时 `code` 为对应 5 位业务错误码，`message` 为人类可读描述，`data: null`。

---

## 3. Shared models（共享数据模型）

### InitStatusResponse (初始化状态响应体)
| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| initialized | boolean | ✅ | 系统是否已完成管理员初始化 |

### InitializeRequest (首次初始化请求体)
| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| username | string | ✅ | 初始管理员用户名（支持自定义） |
| password | string | ✅ | 初始高强度密码（长度 ≥ 6） |

### LoginRequest (登录请求体)
| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| username | string | ✅ | 管理员用户名（系统默认初始化为 `admin`，支持自定义用户名） |
| password | string | ✅ | 管理员登录密码 |

### LoginResponse (登录成功响应体)
| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| accessToken | string | ✅ | JWT Bearer Token |
| username | string | ✅ | 用户名 |
| expiresAt | number | ✅ | Token 过期绝对毫秒时间戳 (13位) |

### ChangePasswordRequest (修改密码请求体)
| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| oldPassword | string | ✅ | 当前原密码 |
| newPassword | string | ✅ | 新密码（长度 ≥ 6） |

### AdminUserDto (管理员信息响应体)
| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| username | string | ✅ | 用户名 |
| createdAt | number | ✅ | 创建时间戳 (13位) |
| updatedAt | number | ✅ | 上次更新/改密时间戳 (13位) |

---

## 4. Endpoints (接口清单)

### 4.0 GET /api/v1/auth/init-status
- **描述**：查询系统是否已完成管理员初始化（开箱检测）。
- **权限**：公开访问（免鉴权）。
- **请求体**：无。
- **成功响应 (200 OK)**：
  ```json
  {
    "code": 0,
    "message": "success",
    "data": {
      "initialized": false
    },
    "timestamp": 1747584000000
  }
  ```

---

### 4.1 POST /api/v1/auth/initialize
- **描述**：首次开箱向导提交初始化管理员账号与密码。成功后直接下发 JWT 访问令牌。
- **权限**：公开访问（仅在未初始化时可用）。
- **请求体示例**：
  ```json
  {
    "username": "admin",
    "password": "mySecurePassword2026"
  }
  ```
- **成功响应 (200 OK)**：
  ```json
  {
    "code": 0,
    "message": "success",
    "data": {
      "accessToken": "eyJhbGciOi...",
      "username": "admin",
      "expiresAt": 1747670400000
    },
    "timestamp": 1747584000000
  }
  ```
- **常见错误**：
  - `403 Forbidden` (code: `10006`)：系统已完成初始化，禁止重复调用。
  - `400 Bad Request` (code: `10008`)：密码强度不符合要求（少于 6 位）。

---

### 4.2 POST /api/v1/auth/login
- **描述**：管理员登录，校验成功后签发 JWT Token。
- **权限**：公开访问（免鉴权）。
- **请求体示例**：
  ```json
  {
    "username": "admin",
    "password": "admin123"
  }
  ```
- **成功响应 (200 OK)**：
  ```json
  {
    "code": 0,
    "message": "success",
    "data": {
      "accessToken": "eyJhbGciOi...",
      "username": "admin",
      "expiresAt": 1747670400000
    },
    "timestamp": 1747584000000
  }
  ```
- **常见错误**：
  - `400 Bad Request` (code: `10007`)：用户名或密码错误。
  - `400 Bad Request` (code: `40001`)：请求参数缺失或格式非法。

---

### 4.3 PUT /api/v1/auth/password
- **描述**：修改管理员登录密码。修改成功后，此前签发的所有 Token 立即失效。
- **权限**：需要携带有效 Bearer Token。
- **请求体示例**：
  ```json
  {
    "oldPassword": "admin123",
    "newPassword": "newSecretPassword2026"
  }
  ```
- **成功响应 (200 OK)**：
  ```json
  {
    "code": 0,
    "message": "success",
    "data": {
      "username": "admin",
      "updatedAt": 1747584000000
    },
    "timestamp": 1747584000000
  }
  ```
- **常见错误**：
  - `401 Unauthorized` (code: `10001`~`10003`)：凭据无效/过期/被撤销。
  - `400 Bad Request` (code: `10007`)：原密码错误。
  - `400 Bad Request` (code: `10008`)：新密码长度不能少于 6 位。

---

### 4.4 GET /api/v1/auth/me
- **描述**：获取当前已登录管理员信息。
- **权限**：需要携带有效 Bearer Token。
- **请求体**：无。
- **成功响应 (200 OK)**：
  ```json
  {
    "code": 0,
    "message": "success",
    "data": {
      "username": "admin",
      "createdAt": 1747584000000,
      "updatedAt": 1747584000000
    },
    "timestamp": 1747584000000
  }
  ```
- **常见错误**：
  - `401 Unauthorized` (code: `10001`~`10003`)：凭据无效/过期/被撤销。

---

### 4.5 POST /api/v1/auth/logout
- **描述**：管理员登出，使当前及此前签发的所有 Token 立即失效。
- **权限**：需要携带有效 Bearer Token。
- **请求体**：无。
- **成功响应 (200 OK)**：
  ```json
  {
    "code": 0,
    "message": "success",
    "data": null,
    "timestamp": 1747584000000
  }
  ```
- **常见错误**：
  - `401 Unauthorized` (code: `10001`~`10003`)：凭据无效/过期/被撤销。

---

## 5. Auth / Session

- 客户端通过 HTTP Header 传递凭证：`Authorization: Bearer <accessToken>`
- 后端中间件提取 Token，验签并比对签发时间 `iat >= tokenInvalidBefore`。
- 校验失败返回 HTTP 401 及统一错误结构体。前端捕获 401 自动清空 localStorage 并重定向到登录页。

---

## 6. 变更记录

| 日期 | 变更内容 | 状态 |
|---|---|---|
| 2026-09-04 | 初始草案定义（login, password, me, logout） | draft |
| 2026-09-04 | 解除用户名硬编码 `admin` 的限制，支持动态用户名配置与认证 | draft |
| 2026-09-04 | 移除弱出厂密码静默初始化；增加开箱初始化向导契约 (init-status, initialize)，支持环境变量与交互式双轨开箱 | confirmed |
| 2026-09-04 | 引入服务端 RFC 标准 Accept-Language / Content-Language 国际化中间件，打通全链路多语言 | confirmed |
