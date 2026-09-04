# 错误处理

> Argus 的错误分层、传播与转换规则。核心原则：**库层用 `thiserror` 给出精确类型，二进制入口用 `anyhow` 收口，边缘设备上能降级的绝不 panic。**

> ⚠️ **状态：立项约定（尚未经代码验证）**
> 首批代码落地后需回填真实错误枚举与转换点，并删除本提示。

---

## 分层规则

| 层 | 用什么 | 理由 |
|----|--------|------|
| `types` / `db` / `media` / `infer` / `pipeline` | `thiserror` 定义具体枚举 | 调用方需要按变体分支决策（重连？跳帧？停用该路？） |
| `api` | `ApiError` + `IntoResponse` | 统一映射为 HTTP 状态码与响应体 |
| `app` | `anyhow::Result` | 入口只负责报告与退出，不需要区分变体 |

**禁止**：在库 crate 的公开 API 上返回 `anyhow::Error`。调用方拿到它就只能打印，无法做降级决策。

---

## 每个 crate 一个错误枚举

放在该 crate 的 `src/error.rs`：

```rust
// crates/infer/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum InferError {
    #[error("模型加载失败: {path}")]
    ModelLoad {
        path: String,
        #[source]
        source: BackendError,
    },

    #[error("输入张量形状不匹配: 期望 {expected:?}, 实际 {actual:?}")]
    ShapeMismatch { expected: Vec<usize>, actual: Vec<usize> },

    #[error("后端 {backend} 未编译进本次构建")]
    BackendUnavailable { backend: &'static str },

    #[error("推理超时，超过 {0:?}")]
    Timeout(std::time::Duration),

    #[error(transparent)]
    Media(#[from] types::FrameError),
}
```

约定：

- **错误消息用中文**，与团队文档语言一致；但**变体名和字段名用英文**。
- 消息里必须带上定位信息（哪个模型、哪一路、什么形状）。`#[error("推理失败")]` 这种没有排查价值。
- **结构化错误码导出**：底层领域错误枚举（如 `MediaError`、`DbError`）必须实现 `error_code(&self) -> u32` 导出对应的 5 位业务错误码（如 `20001`），`thiserror` 的 `Display` 文本仅用于后端 `tracing::error!` 日志，客户端多语言消息由 `api::i18n` 模块化字典全量接管。
- 跨 crate 传播用 `#[from]`；只有语义确实不变时才用 `#[error(transparent)]`。
- **不要给每个错误都加 `Other(String)` 兜底变体** —— 它会变成所有人偷懒的垃圾桶，让调用方无法分支。

---

## 什么时候 panic

边缘设备无人值守，panic 会让整个进程死掉。

**允许 panic 的场景（仅此三类）**：

1. **启动期的配置/模型错误** —— fail fast，宁可起不来也不要带病运行。用 `expect("配置文件缺少 cameras 段")` 并给出可操作的消息。
2. **违反内部不变量** —— 说明是代码 bug，不是运行时状况。
3. **测试代码**。

**禁止 panic 的场景**：

- 摄像头断流、RTSP 超时 → 返回错误，由上层重连
- 推理失败、NPU 忙 → 返回错误，跳过该帧并计数
- 磁盘写失败 → 返回错误，触发清理策略
- 任何 `unwrap()` 出现在每帧路径上

**规则**：`unwrap()` / `expect()` 只允许出现在 `app` 的启动路径和测试里。库 crate 里出现 `unwrap()` 必须在同行写注释说明为什么不可能失败。

---

## FFI 错误转换

C++ 侧返回错误码，必须在绑定层立刻转换成 Rust 错误，不要让整数错误码往上层扩散：

```rust
// crates/infer/src/backends/rknn.rs
fn check(code: i32, op: &'static str) -> Result<(), BackendError> {
    if code == 0 {
        return Ok(());
    }
    Err(BackendError::Sdk { op, code })
}
```

---

## 服务端国际化 (i18n) 模块化分层

`crates/api/src/i18n/` 必须按业务领域拆分独立子模块，严禁将全系统错误消息堆叠在单个文件中：

```
crates/api/src/i18n/
├── mod.rs      # Locale 解析与全局 5 位错误码段路由调度中心
├── common.rs   # 0 成功码、40001 参数校验、50000 系统通用错误
├── auth.rs     # 10000~19999 认证授权、Token 撤销、密码强度错误
├── camera.rs   # 20000~29999 摄像头、RTSP 握手、SPS 解析、硬解及探活错误
├── task.rs     # 30000~39999 分析任务、几何规则配置、模型推理错误
└── alarm.rs    # 40000~49999 告警记录、快照证据错误
```

- 规则：`localize_api_message` 依据错误码千位/万位高位段路由至对应子模块，实现局部 $O(1)$ 静态匹配；
- 多语言支持：所有模块统一支持 `zh-CN`（简体中文）、`zh-TW`（繁体中文）、`en`（英文）。

```json
{
  "code": 20001,
  "message": "摄像头不存在: cam-99",
  "data": null,
  "timestamp": 1704123456000
}
```

错误码为 5 位数字，按模块分段（详见 [api-guidelines.md](./api-guidelines.md#错误码体系)）：

| 范围 | 模块 |
|------|------|
| `0` | 成功 |
| `10000-19999` | 认证授权 |
| `20000-29999` | 摄像头 |
| `30000-39999` | 算法任务 |
| `40000-49999` | 告警事件 |
| `50000-59999` | 系统 |
| `90000-99999` | 内部错误 |

规则：

- **内部错误细节绝不返回给客户端**，只写日志。
- **错误码是前端分支判断的依据**，比消息文本更稳定（文本可能因 i18n 改变）。
- 领域错误到 `ApiError` 的映射写在 handler 里，不要给领域错误类型直接实现 `IntoResponse`（那会让 `pipeline` 依赖 axum）。

---

## 错误日志的位置

**错误只在被最终处理的地方记日志一次**。每层都 `tracing::error!` 一遍会让一个故障刷出五条日志。

```rust
// ❌ 每层都记
match backend.infer(frame) {
    Err(e) => { tracing::error!(?e, "推理失败"); return Err(e.into()); }
    ...
}

// ✅ 传播时不记，在决定如何降级的那一层记
match pipeline.process(frame) {
    Err(e) => {
        tracing::warn!(camera = %cam.id, error = ?e, "跳过该帧");
        metrics.frames_dropped.inc();
    }
    ...
}
```

详见 [logging-guidelines.md](./logging-guidelines.md)。

---

## 待验证事项

- [ ] `BackendError` 放 `types` 还是各后端 crate 自己定义
- [ ] 错误码是否需要预留扩展空间（当前 5 位数是否够用）
- [x] 前端如何根据错误码做国际化（已验证：采用 HTTP RFC 9110 标准的 `Accept-Language` 语言协商机制。客户端请求自动附带 `Accept-Language` 头；服务端通过 `i18n_response_middleware` 拦截出站 JSON 响应，根据 5 位错误码及语言偏好将 `message` 自动本地化为三语 `zh-CN`/`zh-TW`/`en` 并注入 `Content-Language` 响应头；前端优先直接展示服务端国际化后的 `message`，同时也支持基于错误码在前端做本地精准映射）
