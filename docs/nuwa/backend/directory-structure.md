# 后端目录与配置

依赖方向和职责见 [架构概览](../guides/architecture-overview.md#crate-依赖方向)。

## 代码归属

| 路径                   | 内容                                                           |
| ---------------------- | -------------------------------------------------------------- |
| `crates/types/src/`    | 领域类型、帧契约、共享枚举                                     |
| `crates/db/src/`       | `entity/`、`repository/`、`migration/`                         |
| `crates/media/src/`    | RTSP、解复用、解码器、缓冲池、流分发                           |
| `crates/infer/src/`    | `backend.rs`、`backends/`、`worker.rs`、`c_abi/`、包加载与沙箱 |
| `crates/pipeline/src/` | 管线编排、geometry/ROI/rules、跟踪、抓拍、存储清理             |
| `crates/api/src/`      | `routes/`、协议服务、错误/信封、middleware、i18n、静态资源     |
| `crates/app/src/`      | `main.rs`、`config.rs`、装配与生命周期                         |
| `crates/algo-sdk/src/` | 插件 trait、C ABI、帧视图、预处理、结果发射与测试工具          |
| `algo-packages/`       | 各平台算法包及模型、转换/运行工具                              |
| `native/`（按需）      | 必要的薄硬件垫片，不放业务与调度                               |

当前 API DTO 位于所属路由/服务模块；没有统一的 `src/dto/` 目录。新代码跟随真实归属，不按旧示例新增空目录。

## 模块约定

- `lib.rs` 负责模块声明和 re-export；领域错误集中于 `error.rs`。
- 一个模块选择 `foo.rs` 或 `foo/mod.rs` + 子模块，保持 crate 内一致；单文件超过 500 行时按职责拆分。
- 共享领域契约可进入 `types`，共享实现留在正确领域层；装配放 `app`，不为复用破坏单向依赖。
- crate 用简短名词，文件/模块用 snake_case，类型用 PascalCase，错误名用 `<Domain>Error`。
- 平台 feature 使用 `backend-<platform>` / `decoder-<platform>`；避免 `utils/common/helpers` 等无职责模块。
- 依赖版本集中在 workspace，`Cargo.lock` 入版本控制；硬件垫片的构建规则见 [FFI](./ffi-guidelines.md)。

## 配置

实现以 [config.rs](../../../crates/app/src/config.rs) 和 [config.example.toml](../../../config.example.toml) 为准，不在 spec 复制完整配置结构或默认值。

- 优先级：环境变量 > `.env` > 配置文件 > 代码默认值。
- 环境变量使用 `ARGUS_` 前缀，`__` 分层，例如 `ARGUS_SERVER__PORT`、`ARGUS_DATABASE__PATH`。
- `RUST_LOG` 覆盖日志过滤；日志实际初始化见 [main.rs](../../../crates/app/src/main.rs)。
- 配置模板可入库，`.env` 不入库；新增字段给出默认值与入口校验，保持旧配置兼容。
- 包上传限额使用 `server.max_package_size_mb`，嵌套环境变量优先于兼容键 `ARGUS_MAX_PACKAGE_SIZE_MB`；拒绝零值及字节换算溢出。

## 测试位置

单元测试放被测模块的 `#[cfg(test)]`，public API 集成测试放 `crates/<crate>/tests/`，小型固定数据放 `tests/fixtures/`。
真实 NPU/摄像头测试必须 `#[ignore]` 并按平台 feature 隔离；断言重点见 [质量规范](./quality-guidelines.md)。
