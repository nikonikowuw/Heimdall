# 设计文档: API 层架构重构

## 架构变更总览

```
Before                          After
state.rs (AppState + 5方法)  →  state.rs (纯数据) + camera_probe.rs (探活服务)
algo.rs  (~700行混合)         →  routes/algo.rs (handler) + algo/{archive,upload_service}.rs
system.rs (~350行混合)        →  routes/system/{mod,overview,network,storage,time}.rs
```

## 模块边界

### 1. CameraProbeService

**归属**: `crates/api/src/`（与 `state.rs` 同级）

```rust
// crates/api/src/camera_probe.rs
pub struct CameraProbeService {
    pub db: DatabaseConnection,
    pub pipeline: Arc<PipelineManager>,
    pub stream_hub: Arc<StreamHub>,
    pub event_broadcaster: broadcast::Sender<WsBroadcastEvent>,
}

impl CameraProbeService {
    pub async fn probe_single_camera(&self, cam: db::entity::camera::Model) { ... }
    pub async fn update_and_broadcast_probe(&self, camera_id: &str, params: db::ProbeUpdateParams<'_>) { ... }
    pub fn start_periodic_probe_worker(self: Arc<Self>, interval: Duration) { ... }
}
```

**调用方变更**:
- `app` 层（`crates/app/src/main.rs` 或 `reconcile.rs`）创建 `CameraProbeService` 实例
- `AppState` 不再持有探活相关方法

### 2. algo 模块拆分

**归属**: `crates/api/src/algo/`（新目录）

```
algo/
├── mod.rs                 # pub use re-exports
├── archive.rs             # ArchiveFormat, detect_archive_format, extract_*, copy_dir_all, RAII guards
└── upload_service.rs      # process_uploaded_package_archive_sync, ProcessedUploadResult/Error
```

`routes/algo.rs` 通过 `use crate::algo::{archive::*, upload_service::*}` 导入。

### 3. system 子路由拆分

**归属**: `crates/api/src/routes/system/`（从单文件升级为目录）

```
routes/system/
├── mod.rs       # router() 组装 + re-export DbEvictionStoreAdapter
├── overview.rs  # get_overview + detect_npu_metrics
├── network.rs   # get/update/diagnose/confirm/cancel
├── storage.rs   # get/update status+config + trigger cleanup + DbEvictionStoreAdapter
└── time.rs      # get/set status+config + sync
```

每个子文件持有自己的 handler 函数 + 辅助函数。`mod.rs` 只做路由拼装：

```rust
pub fn router() -> Router<AppState> {
    Router::new()
        .merge(overview::router())
        .merge(network::router())
        .merge(storage::router())
        .merge(time::router())
}
```

## 数据流不变性

- `AppState` 字段结构不变，下游 handler 的 `State(state): State<AppState>` 提取不变
- `CameraProbeService` 通过 `app` 层独立装配，不改变 `AppState` 的 Clone/Share 语义
- `algo::archive` 和 `algo::upload_service` 是纯函数模块，不持有状态

## 小修补设计

### route_layer 注释

在 `routes/mod.rs` 的 `route_layer` 链上方加注释，说明洋葱模型执行顺序。

### 证据图片 access log

在 `serve_evidence_image` handler 入口添加 `tracing::info!`，记录 camera_id（从 path 提取）+ 用户名 + 客户端 IP。

### WS Lagged 区分

在 `ws.rs` 的 `handle_socket` 中，将 `Err(_)` 拆为 `Err(RecvError::Lagged(n))`（warn 日志 + continue）和 `Err(RecvError::Closed)`（break）。

## 风险评估

| 风险 | 影响 | 缓解 |
|------|------|------|
| import 路径遗漏 | 编译失败 | 编译即验证，clippy 兜底 |
| 方法可见性遗漏 | 编译失败 | 同上 |
| 测试中的 AppState 构造 | 测试失败 | AppState 字段不变，构造器不变 |
| algo 模块循环依赖 | 编译失败 | algo 不依赖 routes，单向依赖 |

## 回滚策略

所有改动为纯文件移动 + visibility 调整。若回滚，`git checkout` 恢复原文件即可，零数据影响。
