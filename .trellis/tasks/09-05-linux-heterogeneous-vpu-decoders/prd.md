# PRD: Linux 边缘平台异构硬件解码器扩展 (Rockchip MPP 与华为昇腾 DVPP)

## 1. 背景与现状 (Context & Problem)

### 现状定位
当前工程在 `crates/media/src/decoders/mod.rs` 中：
```rust
#[cfg(target_os = "macos")]
pub use videotoolbox::VideoToolboxDecoder;
#[cfg(not(target_os = "macos"))]
pub use mock::MockDecoder;
```
- macOS 平台已具备基于 VideoToolbox 的硬件解码实现，能够零拷贝输出 `CVPixelBuffer`；
- **非 macOS 平台（即 Linux 生产主战场）目前均回退至 `MockDecoder`**，仅能生成带时间戳的虚拟灰色测试帧。

### 核心痛点
Heimdall 的工业级核心部署环境是边缘嵌入式 Linux 算力盒，主要涵盖：
1. **Rockchip 平台**（RK3588、RK3568、RK3576 等工控边缘设备）；
2. **华为昇腾平台**（Atlas 200I DK A2、Atlas 500、Ascend 310B 等 CANN 异构算力模组）；
3. **通用/开发机 Linux 环境**（无专有 VPU 时需保持平滑回退）。

在 Linux 上仅提供 `MockDecoder` 导致系统在边缘设备真实运行时：
- 无法将 RTSP 子码流实时送入硬件 VPU 进行 H.264/H.265 高效解码；
- 无法解出原生的 **DRM DMA-BUF fd** 或 **Ascend Device Memory 指针**；
- 破坏了“**解码 -> 预处理 -> NPU 推理 全链路端到端零拷贝**”的核心架构约束，导致后续无法实现直接喂入 RKNN / ACL。

---

## 2. 目标与范围 (Goals & Scope)

### 目标 (Goals)
1. **构建 Rockchip MPP 硬件解码器 (`MppDecoder`)**：
   - 适配 Linux 环境下的 Rockchip Media Process Platform (MPP) C API；
   - 支持 H.264 / H.265 码流逐包异步送入并解出原生 DRM DMA-BUF 文件描述符 (`OwnedFd`)；
   - 严格维护硬件步长对齐元数据（如水平 16 字节对齐、垂直 16 行对齐，1080P 对齐为 1920x1088）；
   - 输出符合 `types::FrameRef` 契约的 `FrameHandle::DmaBuf`。

2. **构建华为昇腾 DVPP 硬件解码器 (`DvppDecoder`)**：
   - 适配 AscendCL CANN 架构下的 Digital Video Pre-Processing (DVPP) VDEC API；
   - 从 DVPP 专属内存池（`acldvppMalloc`）分配并解出连续 Device Memory 指针；
   - 严格遵循昇腾 DVPP 16×2 宽跨步约束（如宽 16 对齐、高 2 对齐）；
   - 输出符合 `types::FrameRef` 契约的 `FrameHandle::DeviceMemory`。

3. **模块化条件编译与工厂分发 (`create_decoder`)**：
   - 在 `crates/media/Cargo.toml` 引入平台特性标志：`mpp`、`dvpp`（或 `ascend`）；
   - 在未开启硬件 feature 或开发测试环境下，平滑回退至 `MockDecoder`；
   - 保证开发机 `cargo test --workspace` 与 CI 构建 100% 通过且无编译阻断。

### 明确不做 (Non-Goals)
- 本任务不负责 RGA 缩放或 AIPP 预处理算子编写（属于 `crates/infer` 与管线层后续对接范围）；
- 本任务不包含直接向 NPU 喂图的推理逻辑，聚焦于从原始 NALU 到原生显存句柄的硬件解码闭环；
- 本任务不在 Rust 安全层直接暴露裸指针或 C ABI 结构体，所有 unsafe 边界严格封锁在解码实现内部。

---

## 3. 硬件平台规范与契约约束 (Hardware & Platform Contracts)

### 3.1 Rockchip MPP 解码器规范
- **C API 依托**：MPP (`MppCtx`, `MppApi`, `MPP_CTX_DEC`, `MPP_VIDEO_CodingAVC`, `MPP_VIDEO_CodingHEVC`)；
- **内存载体**：输出缓冲区必须配置为输出 `MPP_BUFFER_TYPE_DRM` / `MPP_BUFFER_TYPE_ION`；
- **文件描述符提取**：通过 `mpp_buffer_get_fd(buf)` 获取 `int fd`，并转换为 Rust `std::os::fd::OwnedFd`（`dup` 保证生命周期独立，防止析构双重释放）；
- **对齐约束**：
  - 水平 Stride: `(width + 15) & !15` (16 对齐)；
  - 垂直 Stride: `(height + 15) & !15` (16 对齐，1080P 对应 1088)；
- **格式**：默认解出 NV12 格式。

### 3.2 华为昇腾 DVPP 解码器规范
- **C API 依托**：AscendCL DVPP (`aclvdecCreateChannel`, `aclvdecSendFrame`, `acldvppMalloc`, `acldvppFree`)；
- **内存载体**：`FrameHandle::DeviceMemory { ptr, size }`，内存必须由 `acldvppMalloc` 分配并在 FrameRef 生命周期终结时安全释放；
- **对齐约束**：
  - 水平跨度: 16 字节对齐；
  - 垂直跨度: 2 字节对齐；
- **格式**：输出 YUV420SP (NV12)。

---

## 4. 验收标准 (Acceptance Criteria)

1. [ ] **MPP 解码模块**：
   - 提供 `MppDecoder` 结构体，实现 `VideoDecoder` trait；
   - 包含流解析、送包 (`mpp_api->decode_put_packet`)、收帧 (`mpp_api->decode_get_frame`) 与 DMA-BUF fd 提取逻辑；
   - 完整的 RAII 生命周期管理（通道销毁 `mpp_destroy`、Buffer 释放）；
   - 在启用 `mpp` feature 的 Linux 环境下成功编译。
2. [ ] **DVPP 解码模块**：
   - 提供 `DvppDecoder` 结构体，实现 `VideoDecoder` trait；
   - 包含异步回调或轮询处理通道、内存池管理与 DeviceMemory 提取；
   - 完整的 RAII 资源释放（`aclvdecDestroyChannel`、`acldvppFree`）；
   - 在启用 `dvpp` feature 的 Linux 环境下成功编译。
3. [ ] **架构隔离与门禁兼容**：
   - macOS 下继续保持 `VideoToolboxDecoder` 零回退；
   - Linux 默认未指定专有硬件 feature 时平滑使用 `MockDecoder`；
   - 宿主机（无硬件环境）执行 `cargo check --workspace`、`cargo clippy --all-targets -- -D warnings` 与 `cargo test --workspace` 全绿无告警。
