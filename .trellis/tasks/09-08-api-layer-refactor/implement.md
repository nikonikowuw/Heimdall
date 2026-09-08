# 实施计划: API 层架构重构

纯后端任务（`crates/api`），无前端改动，跳过 api.md 契约。

## 执行步骤

### Step 1: CameraProbeService 提取 → state.rs 瘦身

1. 新建 `crates/api/src/camera_probe.rs`
2. 将 `AppState` 中的以下方法迁移至 `CameraProbeService`：
   - `probe_single_camera`
   - `start_periodic_probe_worker`
   - `update_and_broadcast_probe`
3. `CameraProbeService` 定义独立结构体，持有 `db`、`pipeline`、`stream_hub`、`event_broadcaster` 的 Arc 引用
4. 从 `AppState` 删除上述 3 个方法
5. 调整 `app` crate 装配逻辑：创建 `CameraProbeService` 实例，注入到需要的位置
6. 从 `lib.rs` 导出 `CameraProbeService`
7. 验证：`cargo check --workspace`

### Step 2: algo 模块拆分

1. 新建 `crates/api/src/algo/` 目录
2. 从 `routes/algo.rs` 提取以下内容至 `algo/archive.rs`：
   - `ArchiveFormat` 枚举
   - `detect_archive_format`
   - `extract_archive_package_from_file`
   - `extract_zip`
   - `extract_tar`
   - `find_extracted_package_root`
   - `copy_dir_all`
   - `TempDirGuard`、`TempFileGuard`
3. 从 `routes/algo.rs` 提取以下内容至 `algo/upload_service.rs`：
   - `ProcessedUploadResult`、`ProcessedUploadError`
   - `process_uploaded_package_archive_sync`
   - 相关辅助常量和函数（`BYTES_PER_MEGABYTE`、`is_multipart_limit_exceeded`、`map_multipart_error`、`upload_size_limit_error`、`write_upload_field_to_temp_file`）
4. 创建 `algo/mod.rs`，做 `pub mod` 声明和 `pub use` re-export
5. 在 `routes/algo.rs` 中更新 import 路径：`use crate::algo::*`
6. 将 `algo.rs` 中的单元测试迁移至对应模块文件
7. 验证：`cargo check --workspace && cargo test -p api`

### Step 3: system.rs 拆分为子目录

1. 将 `routes/system.rs` 转为 `routes/system/mod.rs`
2. 创建子文件：
   - `routes/system/overview.rs` — `get_overview` + `detect_npu_metrics` + `round_1dp`
   - `routes/system/network.rs` — 所有 network 相关 handler
   - `routes/system/storage.rs` — `get_storage_status`、`get_storage_config`、`update_storage_config`、`trigger_storage_cleanup`、`DbEvictionStoreAdapter`、`load_storage_config_from_db`
   - `routes/system/time.rs` — 所有 time 相关 handler
3. 每个子文件定义自己的 `pub fn router() -> Router<AppState>`（overview 除外，它只有一个路由，直接合并到 mod.rs 或自带）
4. `mod.rs` 负责拼装：
   ```rust
   mod overview;
   mod network;
   mod storage;
   mod time;
   
   pub use storage::DbEvictionStoreAdapter;
   
   pub fn router() -> Router<AppState> {
       Router::new()
           .route("/overview", get(overview::get_overview))
           .merge(network::router())
           .merge(storage::router())
           .merge(time::router())
   }
   ```
5. 更新 `lib.rs` 中 `DbEvictionStoreAdapter` 的 re-export 路径
6. 验证：`cargo check --workspace && cargo test -p api`

### Step 4: 小修补

1. **routes/mod.rs 注释**：在 `route_layer` 链上方添加洋葱模型执行顺序注释
2. **evidence.rs access log**：在 `serve_evidence_image` handler 入口添加 `tracing::info!`
3. **ws.rs Lagged 处理**：拆分 `Err(_)` 为 `RecvError::Lagged`（warn + continue）和 `RecvError::Closed`（break）
4. 验证：`cargo check --workspace`

### Step 5: 全量验证

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

逐项确认：
- [x] `routes/algo.rs` < 200 行（实际 117 行）
- [x] `routes/system/mod.rs` < 50 行（实际 24 行）
- [x] `state.rs` 无 async fn（已完全下沉至业务模块）
- [x] 所有测试全绿（277 个测试全部通过）

## 回滚点

每一步完成后均可 `cargo check` 验证。若某步引入问题，`git checkout` 恢复该步涉及的文件即可。

## 预估

- Step 1: ~80 行移动
- Step 2: ~400 行移动
- Step 3: ~350 行移动 + ~30 行新 mod 拼装
- Step 4: ~15 行新增
- Step 5: 验证
