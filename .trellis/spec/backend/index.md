# 后端开发规范

> Argus 后端 = Rust workspace + 三个平台的 C++ SDK 胶合层。

> ⚠️ **状态：立项约定（尚未经代码验证）**
> 本目录全部文件基于立项技术栈编写，尚未经真实代码验证。每份文件末尾列有「待验证事项」，
> 首批代码落地后需逐项确认并回填真实文件路径与示例，同时删除各文件顶部的状态提示。

---

## 先读这个

写任何后端代码前，先读 [../guides/architecture-overview.md](../guides/architecture-overview.md)，确认你要改的东西属于哪个 crate、依赖方向对不对。

---

## 规范索引

| 规范 | 什么时候读 |
|------|-----------|
| [目录结构](./directory-structure.md) | 新建文件/crate，不确定代码该放哪 |
| [错误处理](./error-handling.md) | 定义错误类型、决定是否 panic、做错误转换 |
| [日志规范](./logging-guidelines.md) | 加日志、加 span |
| [数据库规范](./database-guidelines.md) | 加表、写 migration、写查询 |
| [API 规范](./api-guidelines.md) | 加 HTTP 端点、WebSocket 消息 |
| [并发模型](./concurrency-guidelines.md) | 决定 async 还是线程、选通道、加共享状态 |
| [推理后端](./inference-backends.md) | 加/改推理后端、动模型加载 |
| [媒体管线](./media-pipeline.md) | 动解码、帧缓冲、零拷贝、预处理、门控 |
| [FFI 边界](./ffi-guidelines.md) | 写 Rust ↔ C++ 绑定、动 `unsafe` |
| [质量规范](./quality-guidelines.md) | 提交前、写测试、做 code review |

---

## Argus 后端的五条铁律

这五条是本项目区别于普通 Rust 后端的地方，违反任何一条都是 bug 而非风格问题：

1. **阻塞调用不进 async** —— 平台 SDK 全是阻塞的，跑进 tokio 会卡死整个 runtime。见 [并发模型](./concurrency-guidelines.md)。
2. **设备侧零拷贝与三路径分离（硬性规范）** —— 解码输出到推理输入严格维持设备侧零拷贝（VPU/硬解 -> 2D 硬件引擎 RGA/VPC/Metal -> NPU/ANE），物理显存（DMA-BUF / CVPixelBuffer / DeviceMem）直通流转，常驻推理流水线上严禁任何 CPU 像素拷贝与 CPU 色彩转换。严禁笼统宣称“全链路零拷贝”，严格切分 `infer_fast_path`、`snapshot_readback_path` 与 `debug_cpu_fallback_path` 三大路径，低频证据生成路径（Device-to-Host readback -> JPEG encode -> disk）仅作为告警触发时的显式特例。见 [媒体管线](./media-pipeline.md)。
3. **平台差异不外泄** —— `#[cfg(feature = "backend-*")]` 只允许出现在 `infer` / `media` 内部。见 [推理后端](./inference-backends.md)。
4. **`unsafe` 收敛在 `ffi.rs`** —— 且每块都有 `// SAFETY:` 注释。见 [FFI 边界](./ffi-guidelines.md)。
5. **能降级不 panic** —— 设备无人值守，单路故障不能拖垮进程。见 [错误处理](./error-handling.md)。

---

## 配置架构

使用 TOML 作为配置格式，支持环境变量覆盖。

### 配置文件

| 文件 | 入 git | 说明 |
|------|--------|------|
| `config.toml` | ✅ | 应用配置默认值 |
| `config.example.toml` | ✅ | 带注释的配置模板 |
| `.env` | ❌ | 本地环境变量覆盖 |
| `.env.example` | ✅ | 环境变量模板 |

### 优先级

```
环境变量（最高）> .env 文件 > config.toml > 代码默认值（最低）
```

### config.example.toml 示例

```toml
# Argus 配置文件
# 复制为 config.toml 并根据环境修改

[server]
host = "0.0.0.0"        # 监听地址
port = 8080              # 监听端口

[database]
path = "data/argus.db"   # SQLite 数据库路径
max_connections = 4      # 连接池大小（SQLite 写是串行的，够用就行）

[storage]
data_dir = "data"        # 数据根目录（数据库、日志、录像都在这下面）

[storage.retention]
log_days = 30            # 日志保留天数
log_max_size_mb = 100    # 日志总大小上限
alarm_days = 90          # 告警记录保留天数
recording_days = 30      # 录像保留天数

[logging]
level = "info"           # 默认日志级别（被 RUST_LOG 环境变量覆盖）
mode = "prod"            # dev = pretty 彩色终端，prod = compact + 文件

[rtsp]
# 摄像头 RTSP 地址在数据库中管理，这里只放全局超时配置
connect_timeout_ms = 5000
read_timeout_ms = 10000
```

### 环境变量覆盖

环境变量采用双下划线 `__` 作为层级分隔符映射到嵌套结构体：

| 环境变量 | 覆盖的配置项 | 示例 |
|---------|-------------|------|
| `RUST_LOG` | `logging.level` | `RUST_LOG=debug` |
| `ARGUS_LOGGING__MODE` | `logging.mode` | `ARGUS_LOGGING__MODE=dev` |
| `ARGUS_DATABASE__PATH` | `database.path` | `ARGUS_DATABASE__PATH=/mnt/sd/argus.db` |
| `ARGUS_SERVER__PORT` | `server.port` | `ARGUS_SERVER__PORT=9090` |

### 实现

使用 `dotenvy` 加载 `.env` + `config` crate 解析 TOML 与环境变量覆盖：

```rust
// crates/app/src/config.rs
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub storage: StorageConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_path")]
    pub path: PathBuf,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
}

#[derive(Debug, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_mode")]
    pub mode: String,
}

// 默认值函数
fn default_host() -> String { "0.0.0.0".to_string() }
fn default_port() -> u16 { 8080 }
fn default_db_path() -> PathBuf { "data/argus.db".into() }
fn default_data_dir() -> PathBuf { "data".into() }
fn default_max_connections() -> u32 { 4 }
fn default_log_level() -> String { "info".to_string() }
fn default_log_mode() -> String { "prod".to_string() }

pub fn load_config() -> Result<AppConfig, config::ConfigError> {
    // 1. 加载本地 .env 到系统环境变量（若存在）
    let _ = dotenvy::dotenv();

    // 2. 级联加载配置
    let config = config::Config::builder()
        // 加载 config.toml（默认值）
        .add_source(config::File::with_name("config").required(false))
        // 环境变量覆盖（ARGUS_SERVER__PORT -> server.port）
        .add_source(
            config::Environment::with_prefix("ARGUS")
                .prefix_separator("_")
                .separator("__"),
        )
        .build()?;

    config.try_deserialize()
}
```

### 依赖

```toml
[dependencies]
config = "0.14"
dotenvy = "0.15"
serde = { version = "1.0", features = ["derive"] }
toml = "0.8"
```

---

## 平台细节去哪找

三个平台的具体 API 用法、模型转换参数、性能调优不写在 spec 里，由专门的技能或平台官方文档承载：

| 平台 | 载体 |
|------|------|
| Apple Silicon（Core ML / ANE / Metal / VideoToolbox） | Apple 官方 Developer 文档与 `objc2` 绑定规范 |
| Huawei Ascend（CANN / ATC / AscendCL / DVPP） | `ascend-pro` 技能 |
| Rockchip（RKNN / RGA / MPP / RK3568 / RK3576） | `rknn-pro` 技能 |

spec 规定**边界在哪、抽象长什么样**；技能与文档提供**怎么调那些 API**。

---

## Pre-Development Checklist

写任何后端代码之前，自查以下清单：

- [ ] 确认代码所属 crate：领域核心在 `types`，拉流解码在 `media`，推理在 `infer`，调度跟踪在 `pipeline`，数据在 `db`，接口在 `api`
- [ ] 阻塞 FFI / 重型图像计算是否已隔离到专用 OS 线程，严禁侵入 Tokio 工作线程
- [ ] 帧处理是否维持端到端零拷贝（持有 `FrameRef`，无 CPU 像素内存分配或 memcpy）
- [ ] 通道与缓冲区是否全部显式设定固定容量，是否有丢旧帧防积压机制
- [ ] SQLite 操作是否已批量化，是否开启 WAL 模式

---

## Quality Check

提交后端代码前，必须通过以下检查：

- [ ] **执行代码自动格式化**：`cargo fmt --all` 与 `clang-format -i $(find native -name '*.c' -o -name '*.h')`
- [ ] `cargo fmt --all -- --check` 代码格式化验证全绿
- [ ] `cargo clippy --all-targets -- -D warnings` 无任何警告
- [ ] `cargo test --workspace` 单元测试全部通过（硬件相关测试标记 `#[ignore]`）
- [ ] `clang-format --dry-run --Werror $(find native -name '*.c' -o -name '*.h')` 格式化校验通过
- [ ] 无任何未记录的裸 `unsafe` 块（必须附有 `// SAFETY:` 注释）

---

**文档语言**：中文。字段名、类型名、变体名用英文。
