# 日志规范

日志量与视频帧率解耦。初始化由 [app/main.rs](../../../crates/app/src/main.rs) 负责，其他 crate 只调用 `tracing`。

## 级别与字段

| 级别    | 用途                                     |
| ------- | ---------------------------------------- |
| `error` | 需介入的故障，保留完整原因链             |
| `warn`  | 重连、降级、持续丢帧；避免常态高频输出   |
| `info`  | 启动、上下线、模型加载等低频生命周期事件 |
| `debug` | 业务排查，生产默认关闭                   |
| `trace` | 帧细节，必须采样或在生产裁剪             |

- 固定中文短消息，变量放英文 snake_case 结构化字段；不逐帧 `format!` 或 `info!`。
- 统一字段：`camera`、`event_id`、`backend`、`model`、`frame_ts`（UTC 毫秒）、`elapsed_ms`（耗时毫秒）、`error`。
- 错误只在最终处理处记录一次，必要时用 `format_error_chain` 展开 `source()`。

```rust
if frame_id % 100 == 0 {
    tracing::trace!(camera = %camera_id, frame_id, "视频帧采样处理");
}
```

## 上下文与敏感数据

每路 Worker 建立 `camera_worker` span，单次推理使用采样 trace 子 span；异步任务用 `.instrument(span)` 传递上下文。
禁止打印密码、原始带凭据 RTSP URL、像素/Tensor 全量或 Base64 图片；RTSP 日志统一脱敏。
不使用 `println!/eprintln!` 替代 tracing，不在 `Drop` 中依赖日志订阅器。

## 配置与文件日志

当前 `RUST_LOG` 覆盖 `logging.filter`；完整配置见 [目录与配置](./directory-structure.md#配置)。
旧 `ARGUS_LOG_MODE=dev|prod`、文件轮转和应用日志查询/导出尚未接入当前启动路径，不能当作可用功能。

落地文件日志时保留以下约束：

- dev 输出 pretty 终端、不写文件；prod 输出 compact，文件关闭 ANSI，后台 IO 不阻塞业务线程。
- `data/logs/heimdall.log` 单文件 10 MB，最多 5 个历史文件；保留 30 天，目录总量封顶 100 MB，超限删最旧文件。
- 应用日志从文件按时间索引查询，不写 SQLite；分页/导出与 [操作审计接口](../../../crates/api/src/routes/oplog.rs) 区分。
- 日志 UI 虚拟滚动，最多保留 1000 条，实时推送不超过 10 条/秒；未来端点在实现任务中定义，避免沿用未实现的接口草案。

验证采样、脱敏、原因链、轮转限额及 IO 不反压业务。
