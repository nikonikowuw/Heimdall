# Implementation Plan: Linux 边缘平台异构硬件解码器扩展

## 阶段规划与验证清单 (Execution Steps)

### Step 1: types 层 — FrameHandle 租约字段扩展
- **目标**：在 `crates/types/src/frame.rs` 中为 `DmaBuf` 和 `DeviceMemory` 变体增加 `_lease: Arc<dyn Send + Sync>` 字段
- **改动**：
  - `DmaBuf` 变体增加 `_lease: Option<Arc<dyn Send + Sync>>`（Option 兼容测试/非池化场景）
  - `DeviceMemory` 变体增加 `_lease: Arc<dyn Send + Sync>`
  - 同步更新 `Clone` impl：`DmaBuf` 分支 clone fd + `Arc::clone(_lease)`；`DeviceMemory` 分支 copy ptr/size + `Arc::clone(_lease)`
  - 同步更新 `Drop` impl：`DeviceMemory` 分支移除空 `_ => {}` 占位（Arc 自动管理）
  - 同步更新 `Debug` impl 中的 `DeviceMemory` 分支
  - 确保 `MockDecoder` 构建 `Host` 变体不受影响
- **验证**：`cargo check -p types`、`cargo test -p types` 通过

### Step 2: media 层 — Cargo Features + DecodeCommand + DvppBufferPool
- **目标**：基础设施搭建
- **改动**：
  - `crates/media/Cargo.toml` 增加 `[features]` 段：`mpp = []`、`dvpp = []`
  - `crates/media/src/decoders/mod.rs` 顶部增加 `compile_error!` 互斥断言
  - 新增 `DecodeCommand` enum（`Decode { packet, pts, reply }` / `Flush { reply }`），供两个解码器共用
  - `crates/media/src/buffer_pool.rs` 扩展：增加 `DvppBufferPool`（预分配 + acquire/return + Condvar 背压 + Drop 批量释放），在 `#[cfg(all(target_os = "linux", feature = "dvpp"))]` 守卫下编译
- **验证**：
  - `cargo check -p media` 通过
  - `cargo check -p media --features mpp,dvpp` 触发 `compile_error!`

### Step 3: 实现 Rockchip MPP 解码器 (`crates/media/src/decoders/mpp.rs`)
- **目标**：
  - 声明极薄的 MPP C FFI 函数原型与宏常量（`extern "C"` 块，不引入 bindgen）
  - 实现 `MppBufferLease`（`Drop` 调用 `mpp_buffer_put` 归还 buffer group）
  - 实现 `MppDecoderInner`（线程内部状态）：
    - `mpp_create` / `mpp_init` / 配置 `PARSER_SPLIT_MODE`
    - `info_change` 处理：读取步长、配置 `FRAME_BUFFER_COUNT = 20`
    - `decode_put_packet` / `decode_get_frame` 收发循环
    - 收帧路径：`mpp_buffer_inc_ref` → `dup(fd)` → `MppBufferLease` → `FrameHandle::DmaBuf { fd, _lease }`
    - 循环退出后 `mpp_destroy`
  - 实现 `MppDecoder`（async 外壳）：`::new()` spawn 专用线程 + 有界 mpsc (cap=4)
  - `VideoDecoder` trait impl：`decode_packet` / `flush` 通过 command-reply 模式
  - `Drop`：drop sender → join thread
- **FFI 绑定策略**：`extern "C"` 内联声明（接口稳定且数量有限）
- **验证**：
  - 步长对齐公式单测：`assert_eq!((1920 + 15) & !15, 1920)`、`assert_eq!((1080 + 15) & !15, 1088)`
  - 状态机转换单测（不依赖硬件）
  - 无 `mpp` feature 时模块不参与编译

### Step 4: 实现华为昇腾 DVPP 解码器 (`crates/media/src/decoders/dvpp.rs`)
- **目标**：
  - 声明 AscendCL DVPP VDEC C FFI 函数原型与枚举（`extern "C"` 块）
  - 实现 `DvppBufferLease`（`Drop` 调用 `pool.return_buffer` 归还预分配池）
  - 实现 `DvppDecoderInner`（线程内部状态）：
    - 启动时：计算步长 → `DvppBufferPool::new(block_size, 20)` 预分配 → 创建 VDEC 通道
    - 解码路径：`pool.acquire()` 租借缓冲区 → 设为 VDEC 输出目标 → `aclvdecSendFrame`
    - 收帧路径：`DvppBufferLease { ptr, pool }` → `FrameHandle::DeviceMemory { ptr, size, _lease }`
    - 循环退出后 `aclvdecDestroyChannel` → `DvppBufferPool::drop` 批量 `acldvppFree`
  - 实现 `DvppDecoder`（async 外壳）：同 MppDecoder 对称
  - `VideoDecoder` trait impl / `Drop` 同上
- **FFI 绑定策略**：同 Step 3
- **验证**：
  - DVPP 显存对齐公式单测：`assert_eq!((1920 + 15) / 16 * 16, 1920)`、`assert_eq!((1081 + 1) / 2 * 2, 1082)`
  - `DvppBufferPool` acquire/return 逻辑单测（mock 内存，不依赖硬件）
  - 无 `dvpp` feature 时模块不参与编译

### Step 5: 更新工厂分发与模块导出 (`crates/media/src/decoders/mod.rs`)
- **目标**：
  - 条件编译引入 `mpp` 和 `dvpp` 模块
  - `create_decoder` 四分支条件编译分发（见 design.md §5）
- **验证**：
  - 无 feature 环境默认回退 `MockDecoder`
  - macOS 下继续走 `VideoToolboxDecoder`

### Step 6: 全面门禁与质量检查
- **目标**：
  ```bash
  cargo fmt --all -- --check
  cargo clippy --all-targets -- -D warnings
  cargo test --workspace
  ```
- **验证**：全部通过，0 warnings，0 errors
- **额外确认**：
  - 所有 `unsafe` 块均有 `// SAFETY:` 注释
  - 无 `dbg!`、`println!`、`todo!()`
  - 硬件集成测试已标记 `#[ignore]`

## API 契约跳过说明

本任务为纯后端（`crates/media`）改动，不涉及 HTTP/WebSocket API 变更，不涉及前端，跳过 `api.md` 契约步骤。
