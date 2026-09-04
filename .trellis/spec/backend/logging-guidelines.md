# 日志规范

> 基于 `tracing`。核心原则：**日志量必须和帧率解耦**。边缘设备写日志到 eMMC，每帧一条就是在磨损存储。

> ⚠️ **状态：立项约定（尚未经代码验证）**
> 首批代码落地后需回填真实 span 命名与订阅器配置，并删除本提示。

---

## 级别语义

| 级别 | 用于 | Argus 中的例子 |
|------|------|---------------|
| `error!` | 需要人介入的故障 | 模型加载失败、磁盘写满且清理无效 |
| `warn!` | 自动降级了，但值得关注 | 某路摄像头重连、连续丢帧超阈值、推理超时 |
| `info!` | 生命周期事件，低频 | 进程启动、摄像头上线/下线、模型加载完成、配置重载 |
| `debug!` | 排查用，默认关闭 | 每次事件生成、每次录像片段落盘 |
| `trace!` | 帧级细节，默认关闭且必须采样 | 单帧耗时、单次推理输入形状 |

**硬性规则**：

- **`info!` 及以上级别禁止出现在每帧路径上**。判断标准：这行日志的输出频率会不会随帧率线性增长？会就降到 `trace!`。
- `trace!` 在帧路径上也要加采样（例如每 N 帧一次或每 M 秒一次），不能靠"反正生产环境不开 trace"来兜底 —— 排查真机问题时一定会开。

---

## 结构化字段，不要字符串拼接

```rust
// ❌ 字符串拼接：无法检索，每次调用都有堆分配
tracing::info!("摄像头 {} 已连接，分辨率 {}x{}", cam.id, w, h);

// ✅ 结构化字段
tracing::info!(camera = %cam.id, width = w, height = h, "摄像头已连接");
```

约定：

- **消息文本用中文，字段名用英文 snake_case**。
- 消息是固定短语，不插值。变量一律进字段。
- `%` 用于 `Display`，`?` 用于 `Debug`。错误一律用 `error = ?e`。

---

## 统一字段名

跨 crate 共用同一套字段名，否则日志无法关联：

| 字段 | 含义 | 示例 |
|------|------|------|
| `camera` | 摄像头 ID | `camera = %cam.id` |
| `event_id` | 事件 ID | `event_id = %ev.id` |
| `backend` | 推理后端名 | `backend = "rknn"` |
| `model` | 模型标识 | `model = %spec.name` |
| `frame_ts` | 帧时间戳（毫秒） | `frame_ts = ts` |
| `elapsed_ms` | 耗时 | `elapsed_ms = t.elapsed().as_millis()` |
| `error` | 错误对象 | `error = ?e` |

新增字段前先在这张表里找有没有同义的。**同一个概念两个名字**（`cam_id` 和 `camera`）会让日志检索失效。

---

## Span：每路摄像头一个

管线是多路并发的，没有 span 就分不清哪条日志属于哪一路：

```rust
// crates/pipeline/src/worker.rs
let span = tracing::info_span!("camera_worker", camera = %cam.id);
let _guard = span.enter();
```

约定：

- **每路摄像头的工作线程建立一个长生命周期 span**，该线程内所有日志自动带上 `camera` 字段。
- 推理调用建 `trace` 级别的子 span 用于耗时统计，不要用 `info` 级别（会随帧率刷屏）。
- 跨线程传递时用 `span.in_scope()` 或 `Instrument`，不要手动重复写 `camera = ...`。

---

## 订阅器配置

在 `app` 中初始化，其它 crate 不碰订阅器。

### 开发模式 vs 生产模式

通过 `config.toml` + `.env` 文件配置，环境变量可覆盖：

```toml
# config.toml（默认值）
[logging]
level = "info"
mode = "prod"
```

```bash
# .env（本地覆盖，不入 git）
ARGUS_LOG_MODE=dev
RUST_LOG=argus::pipeline=debug,info
```

| 配置项 | 值 | 说明 |
|--------|-----|------|
| `ARGUS_LOG_MODE` | `dev` / `prod` | 开发模式：pretty 彩色终端；生产模式：compact + 文件 |
| `RUST_LOG` | `级别` 或 `模块=级别` | 日志级别过滤，支持按模块细粒度控制 |

| 模式 | stdout 格式 | file 格式 | 默认级别 |
|------|------------|----------|---------|
| 开发 | pretty（彩色 + 树形 span） | 不写文件 | `debug` |
| 生产 | compact（单行） | 写文件 | `info` |

```bash
# 开发时（.env 自动加载）
cargo run

# 临时覆盖 .env
RUST_LOG=trace cargo run

# 生产部署（不带 .env）
ARGUS_LOG_MODE=prod cargo run
```

### .env 文件加载

使用 `dotenvy`（`dotenv` 的活跃维护 fork）在启动时加载 `.env` 文件：

```rust
// crates/app/src/main.rs
fn main() {
    // 加载 .env（如果存在），不覆盖已有环境变量
    let _ = dotenvy::dotenv();

    // 初始化日志
    logging::init();

    // ... 启动应用
}
```

| 规则 | 说明 |
|------|------|
| **加载时机** | `main()` 最早执行，在任何配置读取之前 |
| **不覆盖已有环境变量** | `.env` 是默认值，命令行 `RUST_LOG=xxx cargo run` 可覆盖 |
| **`.env` 不入 git** | 加入 `.gitignore`，每个开发者有自己的配置 |
| **`.env.example` 入 git** | 提供模板，包含所有配置项和默认值 |
| **生产环境不依赖 `.env`** | 部署时通过 systemd environment 或 Docker `--env` 注入 |

```
# .env.example（入 git，提供模板）
ARGUS_LOG_MODE=dev
RUST_LOG=info

# 数据库
ARGUS_DB_PATH=data/argus.db

# RTSP 摄像头（测试用）
# ARGUS_RTSP_URL=rtsp://admin:pass@192.168.1.100:554/stream
```

```
# .gitignore
.env
```

### 依赖

```toml
# Cargo.toml
[dependencies]
dotenvy = "0.15"
```

选择 `dotenvy` 而非 `dotenv` 的理由：
- `dotenv` 已停止维护（最后更新 2021 年）
- `dotenvy` 是其活跃 fork，API 兼容，修复了多个 bug
- 支持 `.env` 文件的多行值、注释、变量引用

---

## 开发模式日志

开发模式的核心目标：**一眼看清日志属于哪个 span、哪个模块、哪条线程**。

### 终端输出格式

```
2026-09-03 14:30:12.345  INFO argus::pipeline::worker: 摄像头已连接 camera=cam-01
  in argus::pipeline::worker::camera_worker { camera: "cam-01" }
  in argus::pipeline::main_loop { session: "sess-001" }

2026-09-03 14:30:12.346 DEBUG argus::pipeline::inference: 推理完成
  camera=cam-01 elapsed_ms=42 model=yolov8n
  in argus::pipeline::worker::camera_worker { camera: "cam-01" }

2026-09-03 14:30:12.400 ERROR argus::pipeline::worker: 推理失败
  camera=cam-01 error=BackendError(Timeout("rknn timed out after 5000ms"))
  in argus::pipeline::worker::camera_worker { camera: "cam-01" }
  Caused by: ...（完整错误链展开）
```

### 开发模式特性

| 特性 | 说明 |
|------|------|
| **彩色输出** | 级别用颜色区分：ERROR=红、WARN=黄、INFO=绿、DEBUG=蓝、TRACE=灰 |
| **树形 span** | 显示完整的 span 嵌套关系（`in camera_worker { ... } in main_loop { ... }`） |
| **时间戳精简** | 只显示时分秒毫秒，不显示日期（开发时不需要） |
| **线程 ID** | 显示线程名或 ID，便于排查并发问题 |
| **错误链展开** | `thiserror` / `anyhow` 的 `source()` 链自动展开，不用手动 `.map_err()` 打印 |
| **源码位置** | 每条日志显示 `file:line`（可选，通过 `RUST_LOG` 控制） |

### 实现方式

```rust
// crates/app/src/logging.rs
use tracing_subscriber::{fmt, EnvFilter, Layer};

pub fn init_dev_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("debug"));

    // stdout: pretty 格式，彩色，树形 span
    let stdout_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .pretty()                              // 树形 span 展开
        .with_ansi(true)                       // 彩色输出
        .with_target(true)                     // 显示模块路径
        .with_thread_ids(true)                 // 显示线程 ID
        .with_file(true)                       // 显示文件名
        .with_line_number(true)                // 显示行号
        .with_filter(filter);

    tracing_subscriber::registry()
        .with(stdout_layer)
        .init();
}
```

### 按模块过滤

开发时经常需要只看特定模块的日志：

```bash
# 只看 pipeline 模块的 debug
RUST_LOG=argus::pipeline=debug cargo run

# pipeline debug + 其它 info
RUST_LOG=argus::pipeline=debug,info cargo run

# 只看推理相关
RUST_LOG=argus::infer=trace cargo run

# 排除某个模块（太吵了）
RUST_LOG=argus::media=warn cargo run
```

`tracing` 的 `EnvFilter` 原生支持这种语法，无需额外代码。

### 错误链展示

```rust
// ❌ 错误链丢失
tracing::error!(error = ?e, "推理失败");
// 只看到：error=BackendError(Timeout("rknn timed out"))

// ✅ 展开完整错误链
tracing::error!(error = %format_error_chain(&e), "推理失败");
// 看到：
//   BackendError(Timeout("rknn timed out after 5000ms"))
//   Caused by: RknnError(DeviceTimeout)
//   Caused by: IoError(ConnectionReset)

/// 格式化错误链，用 " Caused by: " 连接
fn format_error_chain(e: &dyn std::error::Error) -> String {
    let mut chain = vec![format!("{}", e)];
    let mut source = e.source();
    while let Some(err) = source {
        chain.push(format!("Caused by: {}", err));
        source = err.source();
    }
    chain.join("\n  ")
}
```

---

## 生产模式日志

生产模式的核心目标：**日志量可控、可存储、可查询**。

- 默认级别 `info`，通过 `RUST_LOG` 环境变量覆盖。
- 输出到 stdout（由 systemd / 容器收集）+ 文件（滚动存储）。
- 启用 `tracing` 的编译期级别裁剪，把 `trace!` 直接编译掉，避免帧路径上的运行时判断开销。

详见下方「日志存储与轮转」章节。

---

## 日志存储与轮转

边缘设备没有 systemd 的情况很常见（裸机、Docker、嵌入式 Linux），需要应用内自行管理日志文件。

### 双通道输出

```
tracing subscriber
├── stdout  →  开发模式：pretty 彩色（终端调试）
│             生产模式：compact 单行（外部收集）
└── file    →  滚动日志文件（仅生产模式，开发模式不写文件）
```

| 模式 | stdout 格式 | file 格式 | 目标 |
|------|------------|----------|------|
| 开发 | pretty（彩色 + 树形 span） | 不写文件 | 终端实时调试 |
| 生产 | compact（单行） | full（多行 + span 展开） | 外部收集 + 本地存储 |

### 文件轮转策略

| 参数 | 值 | 说明 |
|------|-----|------|
| 文件路径 | `data/logs/argus.log` | 和数据库、录像同属 data 目录 |
| 单文件上限 | 10 MB | 超过自动切到下一个文件 |
| 保留文件数 | 5 个 | 最多占用 50 MB |
| 保留天数 | 30 天 | 超过自动删除 |
| 总大小上限 | 100 MB | 超过时删最旧的文件直到低于上限 |

```
data/logs/
├── argus.log          # 当前写入
├── argus.log.1        # 上一个文件
├── argus.log.2
├── argus.log.3
├── argus.log.4
└── argus.log.5        # 最旧的文件
```

### 实现方式

使用 `tracing-appender` 的 `RollingFileAppender`：

```rust
// crates/app/src/logging.rs
use tracing_appender::rolling::{RollingFileAppender, Rotation};

pub fn init_file_appender(data_dir: &Path) -> (RollingFileAppender, impl tracing_subscriber::layer::Layer) {
    let log_dir = data_dir.join("logs");

    // tracing-appender 原生支持按时间轮转并保留指定数量；按大小限额由后台 Retention Worker 周期巡检清理
    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)       // 按天轮转
        .filename_prefix("argus")
        .max_log_files(5)                // 保留最多 5 个时间周期文件
        .build(&log_dir)
        .expect("无法创建日志目录");

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_appender)
        .with_ansi(false)               // 文件中不写 ANSI 颜色码
        .with_target(true)
        .with_thread_ids(true);

    (file_appender, file_layer)
}
```

### 保留策略执行

- **启动时清理**：应用启动时扫描 `data/logs/`，删除超过 30 天或总大小超过 100 MB 的旧文件。
- **运行时检查**：每小时检查一次文件大小，超过上限时删除最旧文件。
- **不阻塞写入**：清理逻辑在后台线程执行，不阻塞日志写入。

---

## 日志查询 API

前端日志查看页需要后端提供查询接口。

### 接口定义

```
GET /api/v1/logs
```

| 参数 | 类型 | 说明 |
|------|------|------|
| `level` | `string` | 过滤级别：`error`、`warn`、`info`、`debug`、`trace` |
| `before` | `number` | 游标分页，返回此时间戳之前的日志（UTC 毫秒） |
| `limit` | `number` | 返回条数，默认 100，上限 500 |
| `camera` | `string` | 过滤摄像头 ID |
| `search` | `string` | 全文搜索（消息文本 + 字段值） |

### 响应格式

```json
{
  "logs": [
    {
      "timestamp": 1693735200000,
      "level": "warn",
      "target": "argus::pipeline::worker",
      "message": "摄像头重连",
      "fields": {
        "camera": "cam-01",
        "elapsed_ms": 3200
      }
    }
  ],
  "hasMore": true
}
```

### 实现方式

- **不把日志存数据库**：日志量大，写数据库会和业务数据争锁。
- **直接解析日志文件**：启动时建立日志文件的内存索引（时间戳 → 文件偏移量），查询时按时间范围定位文件并读取。
- **内存索引**：只索引 `error`/`warn`/`info` 级别（低频），`debug`/`trace` 不索引（高频，需要时直接搜文件）。

---

## 日志导出

支持将日志导出为文件供远程分析。

### 导出接口

```
GET /api/v1/logs/export
```

| 参数 | 类型 | 说明 |
|------|------|------|
| `fromMs` | `number` | 起始时间 |
| `toMs` | `number` | 结束时间 |
| `level` | `string` | 最低级别（`error` 只导出 error 及以上） |

### 导出格式

- **格式**：纯文本（`tracing` 的 full 格式，包含 span 展开）
- **文件名**：`argus-logs-20260903-143000.txt`
- **不压缩**：日志本身是文本，压缩收益低，且用户可能直接 `tail -f`

---

## 前端日志查看页

系统设置 → 日志 Tab。

### 功能

| 功能 | 说明 |
|------|------|
| 实时流 | WebSocket 订阅，新日志实时滚动显示 |
| 级别过滤 | 按 error / warn / info / debug / trace 筛选 |
| 摄像头过滤 | 按摄像头 ID 筛选 |
| 全文搜索 | 搜索消息文本和字段值 |
| 导出 | 按时间范围导出为 txt 文件 |
| 暂停 | 暂停实时滚动，手动翻阅历史 |

### 性能约束

- **虚拟滚动**：日志列表用虚拟滚动（`react-window` 或类似方案），不渲染不可见行。
- **内存上限**：前端最多保留 1000 条日志在内存中，超出时丢弃最旧的。
- **WebSocket 限流**：实时日志最多每秒推送 10 条，超出时在前端做采样合并（如 "过去 1 秒有 47 条 debug 日志"）。

---

## 禁止事项

- ❌ `println!` / `eprintln!` —— 一律用 `tracing`
- ❌ 日志里输出完整帧数据、张量内容、base64 图片
- ❌ 日志里输出摄像头 RTSP URL 的密码部分（脱敏后再打）
- ❌ 在 `Drop` 里打日志（关机时序不确定，可能在订阅器销毁后执行）
- ❌ 同一个错误在传播路径上每层都打一次，见 [error-handling.md](./error-handling.md#错误日志的位置)
- ❌ 日志写入阻塞业务线程（文件 I/O 必须在独立层/线程）
- ❌ 日志文件无限增长（必须配置轮转和保留策略）
- ❌ 前端日志页渲染全量日志（必须虚拟滚动）

---

## 待验证事项

- [ ] 是否引入 `metrics` crate 做计数器/直方图（帧率、丢帧率、推理延迟），还是靠日志聚合
- [ ] `tracing-appender` 的 `max_log_files` 是否支持按大小轮转（需验证 API）
- [ ] 日志文件索引的内存占用：10 万条索引约占多少内存
- [ ] 边缘设备的 stdout 收集方案：systemd-journald vs Docker logs vs 裸机 crontail
- [ ] 开发模式 `pretty()` 格式的性能影响（span 嵌套深时是否有明显延迟）
- [ ] 错误链展开是否需要引入 `anyhow`，还是用 `thiserror` + 手动 `source()` 遍历
- [ ] `.env` 文件中是否需要支持变量引用（如 `ARGUS_DB_PATH=${ARGUS_DATA_DIR}/db`）
