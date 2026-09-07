# 日志规范 (Logging Guidelines)

> 核心铁律：**日志量必须与视频帧率解耦**。边缘设备写日志到 eMMC，严禁在常驻每帧路径上高频写日志磨损存储。

---

## 1. 技术栈与初始化入口

- **核心技术栈**：`tracing` + `tracing-subscriber` + `tracing-appender`。
- **环境初始化**：在 `crates/app/src/main.rs` 启动最早阶段执行 `let _ = dotenvy::dotenv();`。
- **职责边界**：订阅器与 Appender 的构建收敛于 `crates/app`，其他 crate 仅调用 `tracing` 宏，严禁触碰订阅器。
- **环境变量控制**：
  - `ARGUS_LOG_MODE=dev|prod`：开发模式输出 pretty 彩色终端，不写文件；生产模式输出 compact 单行并在后台滚动写入文件。
  - `RUST_LOG=模块=级别`：细粒度模块过滤（如 `RUST_LOG=pipeline=debug,info`）。

---

## 2. 日志级别与逐帧路径硬性规则

| 级别 | 适用场景 | 硬性约束 |
|------|---------|---------|
| `error!` | 需要人工介入的致命故障（模型加载失败、写盘超限） | 必须附加 `error = ?e` 或格式化错误链 |
| `warn!`  | 自动降级但值得关注的异常（网络重连、连续丢帧、推理超时） | 严禁常态高频触发 |
| `info!`  | 系统生命周期低频事件（启动、上下线、模型加载完成） | **严禁出现在每帧推理或解码路径上** |
| `debug!` | 业务排查（告警事件生成、视频片段落盘） | 默认生产环境关闭 |
| `trace!` | 帧级细节与微观耗时统计 | **必须采样（如每 N 帧一次）**，生产编译期可裁剪 |

---

## 3. 结构化字段与跨 Crate 统一字典

- **基本格式**：固定消息用**中文短语**，变量一律进**英文 snake_case 结构化字段**，禁止使用字符串插值（`format!` / `{}`）。
- **统一字段字典**（严禁同义不同名）：

| 字段名 | 类型/格式 | 含义 | 示例 |
|-------|----------|------|------|
| `camera` | `%cam.id` (Display) | 摄像头 ID | `camera = %cam.id` |
| `event_id` | `%ev.id` (Display) | 告警/事件 ID | `event_id = %ev.id` |
| `backend` | `&str` | 推理后端名 | `backend = "rknn"` |
| `model` | `%spec.name` | 模型标识 | `model = %spec.name` |
| `frame_ts` | `i64` | 13 位 UTC 毫秒时间戳 | `frame_ts = ts` |
| `elapsed_ms` | `u128 / f64` | 耗时时长（毫秒） | `elapsed_ms = t.elapsed().as_millis()` |
| `error` | `?e` (Debug) | 错误对象 | `error = ?e` |

- **错误链展开**：打印错误时必须保留其 `source()` 原因链（如调用 `format_error_chain(&e)`），禁止只打顶层无用错误。

---

## 4. Span 与上下文规范

- **每路流工作线程独立 Span**：每路摄像头 worker 线程创建长生命周期 Span：
  ```rust
  let span = tracing::info_span!("camera_worker", camera = %cam.id);
  let _guard = span.enter();
  ```
- **耗时子 Span**：单次推理调用使用 `trace` 级别子 span，跨线程异步分发必须使用 `.instrument(span)`。

---

## 5. 存储、轮转与保留策略 (生产模式)

- **路径与命名**：`data/logs/heimdall.log`（历史文件滚动为 `heimdall.log.1` ... `heimdall.log.5`）。
- **硬性配额约束**：
  - 单文件上限 **10 MB**，最多保留 **5 个** 历史文件；
  - 日志最长保留 **30 天**，全目录硬顶上限 **100 MB**（超过由后台 worker 清理最旧文件）；
  - 文件 Appender 必须关闭 ANSI 颜色码（`with_ansi(false)`），且落盘 I/O 绝不阻塞业务线程。

---

## 6. 日志查询与导出 API 契约

- **查询接口**：`GET /api/v1/logs?level=info&before=<ms>&limit=50&camera=<id>&search=<kw>`
  - 响应遵循标准根信封，`data` 包含 `logs: LogItem[]` 与 `hasMore: bool`。
  - 日志数据不入 SQLite（防止并发锁争用），由后端按内存时间索引直接读取文件并做游标分页。
- **导出接口**：`GET /api/v1/logs/export?fromMs=<ms>&toMs=<ms>&level=<lvl>`
  - 导出纯文本文件 `heimdall-logs-<timestamp>.txt`。
- **前端查看性能契约**：必须采用虚拟滚动（如 `react-window`），内存保留上限 1000 条，WebSocket 推送限流（≤10 条/秒）。

---

## 7. Wrong vs Correct

### 7.1 结构化字段与帧率解耦

#### ❌ Wrong (字符串插值、逐帧 info 打爆磁盘)
```rust
tracing::info!("摄像头 {} 处理第 {} 帧成功", cam.id, frame_id);
```

#### ✅ Correct (固定中文消息、结构化字段、帧路径采样)
```rust
if frame_id % 100 == 0 {
    tracing::trace!(camera = %cam.id, frame_id, "视频帧采样处理");
}
```

### 7.2 错误链记录

#### ❌ Wrong (丢失底层原因)
```rust
tracing::error!(error = ?e, "推理失败"); // 输出: error=BackendError(Timeout)
```

#### ✅ Correct (完整展开 source 链)
```rust
tracing::error!(camera = %cam.id, error = %format_error_chain(&e), "推理失败");
```

---

## 8. 禁止事项 (Iron Rules)

- ❌ `println!` / `eprintln!` —— 统一使用 `tracing`
- ❌ 日志打印完整图像像素、Tensor 矩阵、Base64 图片数据
- ❌ 日志打印包含密码的原始 RTSP URL（必须脱敏）
- ❌ 在 `Drop` 中调用 `tracing`（进程退出时订阅器可能已提前销毁）
- ❌ 同一错误在自下而上的调用链路中重复 `error!` 打印（仅在顶层 handler 或边界处理处记录一次）
- ❌ 日志落盘同步阻塞 Tokio 异步运行时
