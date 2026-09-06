# Research: 系统设置前端复用、认证与跨 IP 恢复

- Query: 五分组设置页如何最小化复用现有前端；设备管理 IP 改变后如何重新登录并确认网络；现有 API、错误码与信封有什么限制。
- Scope: internal；浏览器同源规则附标准文档参考，未执行外部网络或设备修改实验。
- Date: 2026-09-06
- Phase: 1.2 research。只写本文件，不修改产品代码、spec 或其他任务工件，不执行 Git 操作。

## Findings

### 1. 结论

现有 SPA 足以承载五分组设置页：沿用 `Layout` 的本地标签导航、原生表单、`request()`、现有改密弹窗和 i18next。仓库没有 React Router、TanStack Query、React Hook Form、Zod 或已生成的 shadcn/ui 组件，不能把 spec 示例当成已存在的工具。

设备 IP 改变时，浏览器 origin 也改变。推荐在新地址使用现有登录页登录，随后查询服务器保存的待确认网络操作，由管理员明确确认。查询待确认操作必须经过认证；操作 ID 本身不能是授权凭据。无需跨 origin 搬运 JWT、改变现有全局 token 撤销规则或引入免登录确认接口。

### 2. 已找到的文件与复用位置

| 文件 / 行号 | 已验证事实与复用点 |
| --- | --- |
| `web/src/app/layout.tsx:29` | `NavTab` 是六项字符串联合；`currentTab` 初值为 `live`，页面通过条件渲染切换，没有 URL router。加入 `system` 分支即可。 |
| `web/src/app/layout.tsx:65` | 左侧现有图标导航按钮；新增 Lucide 设置图标及三语 accessible label。 |
| `web/src/app/layout.tsx:135` | 语言、主题、改密、登出控制区已存在。设置页账号区可通过回调打开同一个全局改密弹窗。 |
| `web/src/app/layout.tsx:192` | `ChangePasswordModal` 已由 Layout 统一挂载，避免再维护第二套密码表单。 |
| `web/src/components/ChangePasswordModal.tsx:26` | 已有旧密码、新密码、确认密码校验、提交、结果提示；成功后 1500 ms 清空输入并执行本地 logout。 |
| `web/src/features/cameras/components/CameraModal.tsx:25` | 现有原生表单 + local state + `isSubmitting` + inline error 模式。`84` 行提交、`123` 行失败保留输入、`53` 行 ESC 监听与清理，可复用交互模式。 |
| `web/src/lib/api.ts:43` | 所有普通 JSON 请求通过 `request<T>(endpoint, RequestInit)`；Bearer 与 Accept-Language 在此注入。`api.get/post/put/delete` 位于 `82` 行。 |
| `web/src/lib/api.ts:157` | `authApi` 已涵盖初始化、登录、改密、当前账号、登出，设置页不需要新认证端点。 |
| `web/src/types/index.ts:1` | 实际前端共享 DTO 位于此文件，而非 spec 中的 `types/api.ts`。认证 DTO 在 `8–36` 行。 |
| `web/src/i18n/index.ts:22` | `import(./${language}/${namespace}.json)` 动态加载资源；`31` 行 namespace 列表尚无 `system`。 |
| `web/src/hooks/use-locale.ts:16` | `useLocale` 已订阅 i18n 状态，切换语言会写 `argus-locale`。 |
| `web/src/hooks/use-theme.ts:27` | 已有 `useTheme`；当前使用本地 hook 状态和 DOM/localStorage，并非 spec 示例中的 Zustand uiStore。设置页保持使用现有入口即可。 |
| `web/src/lib/utils.ts:4` | `cn()` 已组合 clsx 和 tailwind-merge，用于条件样式。 |
| `web/src/styles/globals.css:8` | 实际字体 tokens、`16` 行亮色 tokens、`42` 行暗色 tokens、`67` 行 `.frosted-glass`，均可直接复用。 |
| `web/package.json` | React 19、Zustand 5、Vite 6、Tailwind 4、react-i18next 17、Lucide、motion；未安装 router、数据查询库、表单库或 Radix/shadcn 依赖。此处为声明版本范围，非本次解析 lockfile 的锁定版本。 |
| `web/vite.config.ts` | 开发 `/api` 代理目标为 `http://127.0.0.1:8000`；不能将 Vite 的端口 5173 直接拼进生产设备的新管理地址。 |
| `crates/api/src/routes/mod.rs:17` | 普通业务端点集中进入 `protected` router，再统一施加 `require_auth`；新增设置与操作端点应进入这里。 |
| `crates/api/src/lib.rs:21` | `/api/v1` 根路由、i18n 响应层、SPA fallback；当前 CORS 为 permissive。CORS 并不共享浏览器 storage。 |
| `crates/app/src/main.rs:173` | 应用通过监听 socket 提供服务；`200` 行直接 `axum::serve(listener, app)`，未发现用于网络确认的请求本地目的地址扩展。 |

### 3. 真实认证契约

类型来源：`crates/types/src/auth.rs:25`、`:48`、`:57`、`:65`；路由来源：`crates/api/src/routes/auth.rs:19`。

| 端点 | 输入 / 输出 data | 认证与行为 |
| --- | --- | --- |
| `GET /api/v1/auth/init-status` | `{ "initialized": true }` | 公开。现有 LoginPage 挂载后调用。 |
| `POST /api/v1/auth/initialize` | 输入 `username`, `password`；输出 `accessToken`, `username`, `expiresAt` | 仅未初始化可用；已初始化 HTTP 403 / 10006。 |
| `POST /api/v1/auth/login` | 输入 `username`, `password`；输出 `accessToken`, `username`, `expiresAt` | 公开；用户名或密码错误 HTTP 400 / 10007。没有 refresh token。 |
| `GET /api/v1/auth/me` | `{ "username": "...", "createdAt": 1780000000000, "updatedAt": 1780000000000 }` | `AuthUser` extractor 校验 JWT。没有 `id`、displayName、role、email 字段，也没有修改用户名端点。 |
| `PUT /api/v1/auth/password` | 输入 `{ "oldPassword": "...", "newPassword": "..." }`；输出 `AdminUserDto` | `confirmPassword` 仅前端使用，不发送；成功撤销所有旧 token，不签发新 token。 |
| `POST /api/v1/auth/logout` | 无 body；成功 `data: null` | 全局撤销当前单管理员的旧 token，并写操作日志。 |

- `issue_token()` 使用 HS256，签发有效期 24 小时；`iat`、`exp`、`expiresAt` 均为 UTC 毫秒：`crates/api/src/routes/auth.rs:29`。现有浏览器 store 不记录 `expiresAt`，通常通过后续 401 得知失效。
- 登录、初始化对用户名 `trim()`；初始与新密码后端最低要求为 Rust 字符串字节长度 6，不是复杂度策略：`crates/api/src/routes/auth.rs:63`、`:68`、`:117`、`:161`。当前改密前端用 JS `length < 6`，对非 ASCII 的计量有差异；本任务不应默默引入更强密码策略。
- 改密更新数据库及全局 `token_invalid_before`：`crates/api/src/routes/auth.rs:175`。登出也更新同一全局时间阈值：`:223`。所以这是单管理员全部旧会话撤销，而非仅销毁当前浏览器的一条服务端 session。
- Layout 手工登出即使后端请求失败也清理本地状态：`web/src/app/layout.tsx:39`。API 自动 401 登出只做本地清理：`web/src/lib/api.ts:62`。
- `AuthUser` 优先读取 `Authorization: Bearer ...`，无 Authorization 时也接受 query token，用于现有流接口：`crates/api/src/middleware/auth.rs:25`。这个兼容行为不构成把 JWT 放进新地址导航链接的理由。
- 单管理员账户页面以 `authApi.getMe()` 为事实来源；store 中的 username 只是当前浏览器缓存，不能代替账号数据查询。

### 4. 跨 IP 恢复的浏览器限制与建议流程

#### 已有行为

`web/src/stores/auth.ts:11` 定义 `argus-token`、`argus-user`、`argus-remember-user`。初始化优先读 localStorage，随后读 sessionStorage（`:22`）；remember=true 写 localStorage，false 写 sessionStorage（`:53`）；本地 logout 清理两个位置的 token/user，保留 remembered username（`:72`）。

localStorage 按 scheme、host、port 隔离；sessionStorage 还按顶层浏览上下文隔离。因此从 `http://192.0.2.10:8000` 到 `http://192.0.2.20:8000` 的全页导航不会继承 token、用户名、语言或主题偏好。即使两个 IP 指向同一设备、服务端 JWT secret 未变，浏览器也不会自动读取旧 origin 的 token。

现有 HTTP `BASE_URL='/api/v1'`，WS/媒体 URL 使用 `window.location`：`web/src/lib/api.ts:41`、`:136`、`:145`。更换页面 origin 后会自然使用新地址；停在旧页面只会继续访问旧地址。当前 token 不绑定 IP，这与 storage 隔离是两件事。

#### 建议流程（待主会话 api.md 定稿，以下新端点均未实现）

1. 管理员在网络分组选择真实网卡并提交配置。危险操作确认界面展示当前地址、预计新地址、断连影响与自动恢复期限。网络操作须由后端持久化并限制为至多一个待确认操作。
2. 后端在切网前返回操作 ID、状态、确认期限、新管理地址候选与恢复指引。响应丢失不能促使前端盲目重发变更；先查询活动操作或使用约定的幂等键。
3. 旧页面展示可点击的新管理地址与旧地址恢复说明。仅允许正确解析的 HTTP(S) 地址，不含 userinfo/JWT/密码；后端不能信任任意 Host/Forwarded 头生成跳转目标。页面端新地址导航使用完整页面加载。
4. 新地址使用现有 LoginPage 正常登录。无需调用后端 `/auth/logout` 来“转移会话”：那会撤销包括其它管理员浏览器在内的旧 token。如果产品要求清除旧页本地登录，只能作为独立本地行为，不能用于撤销服务器会话。
5. 登录成功后，由认证后的 Layout 协调一个轻量 pending 查询，例如 `GET /api/v1/system/network/changes/pending`，返回对象或 `null`。页面不依赖原 origin 的 localStorage，也不要求用户手抄 operation ID。根页面直接访问、刷新页面、丢失初次响应都能恢复流程。
6. 找到待确认操作后进入设置页网络分组，显示实际网卡地址、剩余确认时间与“确认使用新网络”按钮。只有管理员明确操作才调用已认证的确认端点，例如 `POST /api/v1/system/network/changes/{id}/confirm`。登录、页面可加载、WS 打开、GET 操作成功本身都不能自动确认。
7. 请求失败时保留恢复信息。未确认直至期限结束，由后端独立执行恢复；前端显示“结果待确认”直到能从后端读取最终状态，不能在本地计时归零时直接宣称“已恢复”。

#### 契约必须定义的边界

- **确认资格**：仅有 JWT + operation ID 不证明访问发生在目标新地址。后端需要验证当前活动操作、实际应用状态以及请求访问新管理网络的证据；单靠前端提交 `window.location.hostname`、任意 HTTP Host 或客户端 `confirmed: true` 不足够。现有应用没有此验证，必须由网络设计补齐。反向代理、稳定 DNS、改非管理网卡的确认条件要单独明确，不能在 UI 里伪造“新 IP 已验证”。
- **页面进入方式**：最低改动可在登录后查 pending 并自动切换现有 NavTab；无需新增 router。若需要直达，可增加只识别 `#system-network` 一种意图的窄逻辑，不接受任意 `returnUrl`。查询仍需鉴权且绝不凭 hash 自动确认。
- **已有新 origin 登录缓存**：新地址未必是首次访问。启动时仍须向当前服务器验证会话并查询 pending；若契约要求新鲜登录，可复用登录页做一次本地 re-auth，但不调用全局 logout。不把“页面已有 token”视为网络已确认。
- **访问地址**：优先从服务器可信配置及设备实际地址构造；不能把 Vite 开发端口、内网反向代理端口、任意 forwarded header 当作目标设备可达的管理地址。HTTPS 改 IP 可能涉及证书 SAN，不能自动降级到 HTTP。
- **DHCP**：应用前可能不知道新 IP。API 应允许访问地址未知，UI 给出明确恢复指引，不能预填假地址。多网卡不应只凭第一张网卡推断当前管理接口。
- **时间**：operation deadline 等绝对时间为 UTC 毫秒；相对期限用 `remainingMs` / `confirmationTimeoutMs`。设备对时可能影响墙钟，后端恢复计时应有独立可靠的时长机制；前端倒计时只是提示，恢复责任不依赖浏览器在线。
- **信息保留与凭据**：旧页面可保留非敏感的操作 ID 和地址指引；不能把 JWT 放进 URL、history、`window.name` 或跨 origin 自动传递。新登录沿用原认证流程。

### 5. WebSocket 实际生命周期

- 实际事件外层是 `{ topic, payload, timestamp }`，不是 spec 的 `{ type, payload }`：`crates/api/src/state.rs:10`。前端 topic 常量只有 camera probe、alarm triggered、alarm status changed：`web/src/types/index.ts:39`。
- 事件 socket 当前建在 `LivePage` 的 effect 中：`web/src/features/live/LivePage.tsx:238`；不是全局 provider/store。离开实时页进入设置页，Layout 条件渲染会卸载 LivePage（`web/src/app/layout.tsx:169`）。
- 重连为 1、2、4…秒，上限 30 秒，没有 jitter；连接成功清零重试计数：`web/src/features/live/LivePage.tsx:244`。目标永远是当前 `window.location.host`，无法自行发现新 IP。
- 卸载时取消 timer、解绑回调，关闭 socket：`web/src/features/live/LivePage.tsx:366`。网络设置不要复制第二套 socket。
- 后端 broadcast 容量 1024：`crates/api/src/state.rs:49`。WS 仅在握手路由鉴权；`crates/api/src/routes/ws.rs:17` 的循环处理 shutdown 和广播，不重新校验 token，也不接收确认消息。广播接收错误会结束连接，没有离线事件重放。
- **首版建议**：网络操作恢复必须有已认证的 GET 状态端点；登录/重新进入页面/手动刷新后读取。UI 只维护低频剩余时间显示；若希望推送即时状态，必须把现有事件连接真正提升为全局复用，并在边界集中解码，不能宣称现成全局 WS 可以直接使用。不要将 settings 数据常驻 Zustand 或引入持续轮询实时数据的第二套机制。

### 6. HTTP 信封、错误码与 i18n 的实际偏差

#### 统一信封

- 成功 `ApiResponse<T>` 为 `{code:0,message:"success",data:T,timestamp:i64}`：`crates/api/src/response.rs:7`。错误 `ApiError::into_response()` 输出同样四字段，`data:null`：`crates/api/src/error.rs:87`。`()` 成功 data 也会序列化为 null。
- **已验证偏差**：成功响应经过 i18n 后，zh-TW 的 message 会成为 `成功`：`crates/api/src/i18n/common.rs:5`；中间件对 code=0 同样改写：`crates/api/src/i18n/mod.rs:65`、`crates/api/src/middleware/i18n.rs:41`。与仓库“成功固定 success”契约冲突。新契约应保留固定 success，实施时集中修正，并补三语边界用例。
- **HTTP 拒绝边界缺口**：路由使用原生 `Json<T>` extractor（例如 `crates/api/src/routes/auth.rs:55`），仓库搜索未找到 JsonRejection/QueryRejection 的统一映射。Axum 0.8 原生 malformed JSON、缺字段、错误 Content-Type 等拒绝可直接返回文本错误，而不经过 ApiError 信封。i18n 仅处理 Content-Type 含 application/json 的响应（`:21`），不能补救这些文本响应。此项为源码结构 + 框架默认行为推断，本轮未运行 HTTP 复现。
- 前端 `request()` 对响应直接 `response.json()`，解析失败抛固定中文“网络响应解析失败”；网络 fetch rejection 原样抛出，未统一成 ApiError；也未独立判断 `response.ok`：`web/src/lib/api.ts:57`、`:71`、`:75`。新设置边界应规范错误，保留 HTTP status/业务 code，并为网络切换区分断连与真正的业务失败。
- 当前 ApiError 构造函数是 `(message, code)`，只含 code，不含 timestamp、httpStatus、fieldErrors：`web/src/lib/api.ts:32`。不要照搬 spec 示例中反过来的参数顺序或假定已存在 `silentToast` 选项 / 全局 Toast。

#### 当前占用的错误码

| 码 / HTTP 状态 | 实际用途 / 来源 |
| --- | --- |
| 10001 / 401 | Unauthorized、AuthRequired；`crates/api/src/error.rs:57` |
| 10002 / 401、10003 / 401 | TokenExpired、TokenRevoked；`:59` |
| 10006 / 403 | AlreadyInitialized；`:61` |
| 10007 / 400、10008 / 400 | InvalidCredentials、WeakPassword；`:62` |
| 20001–20011 / 当前统一 400 | Media error_code；`crates/media/src/error.rs:42`，API 映射 `crates/api/src/error.rs:65`，并非每个码按 spec 分配不同 HTTP 状态。 |
| 30001–30006 / 当前统一 400 | Pipeline errors，含底层透传；`crates/pipeline/src/error.rs:33`。 |
| 30011–30022 / 当前统一 400 | Infer errors；`crates/infer/src/error.rs:48`。 |
| 40001 / 400 | **通用 BadRequest**，不是 spec 写的“告警不存在”；`crates/api/src/error.rs:56`。 |
| 40301 / 403、40901 / 409 | 内置算法保护、算法在使用中；`crates/api/src/error.rs:68`。 |
| 40401 / 404 | 通用资源 / DB 记录不存在；`crates/api/src/error.rs:64`、`:72`。 |
| 50000 / 500、50001 / 500 | Internal、通用 Db；`crates/api/src/error.rs:77`。**50001 不能重新分配给系统概览失败。** |
| 10004、40002、40003、50002 | 已占用 i18n 字典；其中 50002 是“分析管线处理异常”，虽当前 Pipeline 映射主要走 3xxxx，也不应作为网络无效复用。 |

`rg '51[0-9]{3}' crates web/src` 本轮无匹配。新系统设置可由主会话在 api.md 选择未占用的 **51xxx** 区间；这里不私自分配具体错误码。

i18n 额外限制：`crates/api/src/i18n/mod.rs:70` 将 40002–49999 全部路由到 alarm，通用 40401 实际不会进入 common 内现存 40401 分支；5xxxx 默认走 common，英文/繁中未知码变成泛化“Internal server error”（`common.rs:58`）。因此新增 51xxx 需明确路由至新 system 翻译，不只添加 ApiError 枚举。错误需用稳定 code 决策，不能按已翻译的 message 文本比较。

### 7. 首版 UI 草案

已阅读 `.agents/skills/frontend-design-direction/SKILL.md`。方向是面向设备管理员的安静工业维护面板：优先真实状态与明确操作，无营销头图、装饰数据、嵌套卡片或新的动效依赖。显著细节为网络变更时同列展示“当前地址 → 新地址”和服务器确认期限。

| 分组 | 首版界面建议（字段需后端能力与 api.md 确认） |
| --- | --- |
| 系统概览 | 只读信息行，展示真实软件/设备信息和可获取的运行状态，标记采样时间；缺少能力显示不可用，不伪造 CPU/NPU 数字。 |
| 账号与安全 | `getMe()` 的 username、创建和更新时间；一个“修改密码”按钮打开 Layout 已有弹窗。保留单管理员模型。 |
| 网络 / 服务 | 网卡列表、当前地址与可编辑能力；编辑目标网卡 IPv4/DHCP 等由能力约束；保存后展示独立 operation 面板。服务监听信息不等价于设备网卡配置。 |
| 存储与保留策略 | 实际存储状态独立显示，下方编辑后端真正支持的保留策略；保存只提交该分组字段。 |
| 对时服务 | 当前设备时间、同步来源/状态独立于可编辑设置；格式使用 Intl，明确时区；高风险的时间改变需确认。 |

- 桌面主工作区内左侧文字分组导航、右侧单列/窄双列表单；小屏分组换为原生 select 或可换行导航，表单回到单列。沿用 Layout 64 px 图标栏，不新建另一套应用壳。
- 使用 `--bg-surface`、`--text-primary`、`--text-secondary`、`--border`、`--accent`、`--accent-amber`、`--destructive` 等实际 tokens；IP/数值使用已有等宽字体；按钮和图标使用现有 Lucide。
- 每组单独 loading/error/dirty/saving/result。失败保留草稿；成功以服务器返回的规范化值替换该组基线，不覆盖其它组。
- 服务端资源放 feature 专用 hooks，返回 `{ data, loading, error, refetch }`；输入草稿为 local state。现在没有可直接复用的通用资源 hook，应该新增窄 hook 而非假装已有 `useSettings()`。
- `request()` 已接收 RequestInit，能承接 AbortSignal；现有 `api.get()` 等便捷方法尚无 options 参数（`web/src/lib/api.ts:82`），设置 API 可直接复用 request 并传 signal。切组/卸载取消未完成读取，拒绝过期响应覆盖新草稿；取消浏览器请求不等于取消服务器网络操作。
- 所有标签、错误兜底、操作状态、确认说明接入 `system.json`；添加 zh-CN/zh-TW/en 三份，与 namespace 注册同步。不要按默认中文字符串补“伪三语”。
- 新表单必须关联 label/id、字段错误用 aria-describedby、状态使用恰当 aria-live；图标按钮有 aria-label，焦点可见；危险确认支持键盘和焦点返回。现存组件有硬编码色值与局部 a11y 缺口，只复用其业务能力，不复制这些偏差。

### 8. 精确实施落点建议

以下均是待实施建议，未创建这些产品文件：

| 归属 | 文件 | 最小责任 |
| --- | --- | --- |
| FE | `web/src/app/layout.tsx` | 加 system 导航和 SettingsPage；向账号页传改密回调；认证后发现 pending 并定向网络分组；改动处使用细粒度 auth 选择器。 |
| FE | `web/src/features/system/SettingsPage.tsx` + 分组组件 | 五组布局、草稿与结果；网络确认/恢复状态。新目录名由主会话固定。 |
| FE | `web/src/features/system/hooks/` | 分资源读取与 mutation、AbortController、pending 恢复；不建立第二条事件 WS。 |
| FE | `web/src/lib/api.ts` | 新增 systemApi；复用 request、authApi；如修复错误处理须保留原调用顺序与 401 本地登出语义。 |
| shared contract / FE | `web/src/types/index.ts` | 同步设置 DTO、能力、网络操作状态联合；和 Rust 契约一致，null/时间单位明确。 |
| FE | `web/src/i18n/{zh-CN,zh-TW,en}/system.json`、同目录 `common.json`、`web/src/i18n/index.ts` | 新设置文案、nav.system、namespace。 |
| FE（仅需要登录上下文提示时） | `web/src/features/auth/LoginPage.tsx` | 可增加网络恢复说明，保留原 login DTO；基本登录流程本身足够，无需复制登录页。 |
| FE（有针对性） | `web/src/components/ChangePasswordModal.tsx` | 账号组直接复用；若调整生命周期，应清理 `50` 行成功延时 timer，避免卸载后影响后续会话；不扩大为认证重构。 |
| BE | `crates/api/src/routes/mod.rs` + 新 `routes/system.rs` | 加入 protected router；只负责协议、校验和服务句柄调用，不在 handler 执行网络命令。 |
| BE | `crates/api/src/state.rs` | 持有后端设置/网络操作服务句柄；资源有限且可恢复，具体归属由总体设计确定。 |
| BE | `crates/api/src/error.rs`、`i18n/mod.rs`、新 `i18n/system.rs`、`i18n/common.rs` | 51xxx 映射与三语；固定成功 message；新请求拒绝转换为统一信封。 |
| BE / app | `crates/app/src/main.rs` | 若确认新网络依赖请求本地目的地址，在 HTTP 服务接入边界提供可信来源；不是在 TS 侧补个 hostname 字段。 |

`web/src/stores/auth.ts` 无需为跨 IP 传 token 而修改；`routes/auth.rs` 无需新增网络专用登录、免认证确认或修改全局撤销行为。其它分组的存储、网络和对时后端职责以对应研究与 design.md 为准。

### 9. 验收与验证建议

- 现成 Vitest 入口：`web/src/lib/api.test.ts:13`（普通请求）、`:32`（登录）、`:63`（401 清理）、`:81`（Accept-Language）；`web/src/stores/auth.test.ts:38`、`:56`（remember 与本地登出）；`web/src/i18n/index.test.ts:11`（语言切换）。可在实际修改这些行为时扩展有意义的边界测试。
- 后端已有完整认证生命周期测试：`crates/api/src/routes/auth.rs:269`。覆盖旧 token 在改密、登出后失效（`:437`、`:482`）、i18n（`:492`）和 WS 需认证（`:552`）。新设置不能改变这些语义。
- 网络流程测试：未认证 pending/confirm 为 401；新 origin 无旧 token 时走登录；新登录能发现服务器 pending；只登录不确认；旧地址/错误 operation ID 不可误确认；确认重复请求幂等；响应丢失不产生重复切网；超时和关闭页面后服务器仍恢复；返回页面依据后端最终状态展示结果。
- 页面行为：五组独立读取与保存；失败保持草稿；切组/刷新恢复已保存值；权限不足/不支持网卡只读；实际未知指标不显示假零；三语、明暗主题、小屏、键盘可用。
- API 边界：非法 JSON、缺字段、错误类型、非法 IP、非整数时间/超界配置均返回定义好的 HTTP + 四字段信封；繁中成功 message 仍为 success；51xxx 三语不降成通用内部错误；离线和业务失败清楚区分。
- 生产新地址回连需用真实嵌入 SPA / 受控 Linux 环境验证，不能以 localhost Vite 代理通过视为设备 IP 迁移验证通过。
- 本轮只做研究，未运行前后端测试、构建或设备网络修改。实现后按仓库门禁执行格式化、lint、类型检查、测试和 build。

## Related Specs

- `.trellis/workflow.md`：Phase 1.2 持久化研究；1.3/1.4 由主会话定设计与 API 契约。
- `.trellis/spec/guides/index.md`、`architecture-overview.md`、`cross-layer-thinking-guide.md`、`code-reuse-thinking-guide.md`：边界、唯一 DTO/解析责任和既有实现优先。
- `.trellis/spec/frontend/index.md`、`component-guidelines.md`、`styling-guidelines.md`：双主题 tokens、i18n、可访问性与轻量组件。
- `.trellis/spec/frontend/hook-guidelines.md`、`state-management.md`、`type-safety.md`、`error-handling.md`：资源 hook、清理、有界实时数据、共享类型和统一 API。
- `.trellis/spec/backend/index.md`、`api-guidelines.md`：薄 handler、固定信封、认证与 UTC 毫秒。
- `.agents/skills/frontend-design-direction/SKILL.md`：产品维护工具的视觉方向与复用约束。

## External References

- MDN, Window.localStorage：<https://developer.mozilla.org/en-US/docs/Web/API/Window/localStorage>；存储按 origin 分隔，HTTP 与 HTTPS 也不同。
- MDN, Window.sessionStorage：<https://developer.mozilla.org/en-US/docs/Web/API/Window/sessionStorage>；按 origin 与顶层浏览上下文分隔。
- MDN, Same-origin policy：<https://developer.mozilla.org/en-US/docs/Web/Security/Same-origin_policy>；origin 的 scheme/host/port 定义。
- Axum 0.8 Json extractor / rejection：<https://docs.rs/axum/0.8/axum/struct.Json.html>、<https://docs.rs/axum/0.8/axum/extract/rejection/enum.JsonRejection.html>。仓库 `Cargo.toml:42` 声明 axum 0.8。
- 上述链接为标准与框架参考，本轮未联网抓取网页；项目结论依据本地源码检索。未使用设计库搜索。

## Caveats / Not Found

- 当前没有 settings API、系统设置页面、全局事件 WS provider、通用数据获取 hook、网络操作 pending/confirm 服务或跨 IP 登录恢复逻辑；推荐路径和新文件不可描述为已实现。
- Spec 多处仍标记“立项约定”；真实代码的 DTO 路径、错误码、WS 外层与生命周期、成功 message 国际化均有差异。本研究只记录事实；由主会话在后续 spec 更新流程回填。
- API envelope 拒绝边界未实际跑请求，需在实现前用失败测试确认；本轮没有引入宽泛错误框架重构。
- 新网络访问的可信确认方法、真实管理 origin、DHCP 未知地址、反向代理/HTTPS、后端恢复期限及进程故障恢复属于总体网络设计，浏览器无法独自保证。
