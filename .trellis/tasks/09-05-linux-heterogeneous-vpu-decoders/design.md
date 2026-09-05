# Technical Design: Linux 边缘平台异构硬件解码器扩展

## 1. 系统架构与模块分层 (Architecture)

```text
                                  VideoDecoder (Trait)
                                          │
       ┌───────────────────────┬──────────┴────────────┬───────────────────────┐
       ▼                       ▼                       ▼                       ▼
VideoToolboxDecoder       MppDecoder              DvppDecoder             MockDecoder
  [macOS 原生]        [Rockchip RK3588]       [Huawei Ascend 310B]     [开发机测试/回退]
       │                       │                       │                       │
       ▼                       ▼                       ▼                       ▼
 CVPixelBuffer          DRM DMA-BUF (fd)          DeviceMemory           Host Buffer
 (Apple ANE 直达)       (RGA -> RKNN 直达)     (DVPP -> ACL 直达)      (CPU 虚拟生成)
```

### 目录与文件布局
```text
crates/media/
├── Cargo.toml                  # 增加 [features] mpp = [], dvpp = []
└── src/
    ├── decoder.rs              # VideoDecoder trait 定义 (保持稳定)
    ├── buffer_pool.rs          # [扩展] HwBufferPool + BufferPoolStats
    └── decoders/
        ├── mod.rs              # 工厂函数 create_decoder 条件编译分发
        ├── mock.rs             # MockDecoder 虚拟解码器 (CPU 回退)
        ├── videotoolbox.rs     # macOS VideoToolbox 硬件解码器
        ├── mpp.rs              # [新增] Rockchip MPP 硬件解码器
        └── dvpp.rs             # [新增] 华为昇腾 DVPP 硬件解码器
```

---

## 2. 跨解码器共性设计 (Cross-Decoder Shared Design)

### 2.1 异步-FFI 桥接线程模型

`VideoDecoder` trait 的 `decode_packet` 是 `async fn`，但 MPP / DVPP 底层均为同步阻塞 C API。
根据 `AGENTS.md` 项目契约——"任何平台 SDK、FFI 或超过约 1 ms 的 CPU 密集工作都不得直接运行在 Tokio worker 中。使用启动时确定数量的专用线程和有界通道；模型/硬件上下文应在线程内常驻"——两个硬件解码器统一采用 **专用解码线程 + 有界 channel** 架构：

```text
┌──────────────────────────┐   bounded mpsc (cap=4)   ┌───────────────────────────────┐
│  MppDecoder / DvppDecoder│ ──── DecodeCommand ────> │  专用解码线程                   │
│  (Tokio 侧 async 外壳)   │ <── oneshot Result ───── │  (常驻 MppCtx / DVPP channel)  │
└──────────────────────────┘                          └───────────────────────────────┘
```

**核心数据类型**：

```rust
/// 解码线程收到的命令
enum DecodeCommand {
    /// 送入一个压缩包并期待解码帧
    Decode {
        packet: bytes::Bytes,
        pts: i64,
        reply: oneshot::Sender<Result<Option<FrameRef>, MediaError>>,
    },
    /// 刷新残留帧
    Flush {
        reply: oneshot::Sender<Result<Vec<FrameRef>, MediaError>>,
    },
}
```

**设计要点**：

1. **构造时 spawn**：解码器 `::new()` 内 `std::thread::spawn` 一个专用线程，在线程闭包内初始化硬件上下文（`mpp_create` / `aclvdecCreateChannel`）和缓冲区池，上下文从此常驻此线程；
2. **外壳结构体仅持有通信句柄**：`Sender<DecodeCommand>` + `Option<JoinHandle<()>>`；
3. **`decode_packet` 实现**：构造 `DecodeCommand::Decode`，通过有界 channel `.send().await` 发送（满时自然反压上游），然后 `.await` oneshot 接收结果——不直接调用任何 FFI；
4. **`flush` 同理**：发送 `DecodeCommand::Flush`，等待 `Vec<FrameRef>` 回传；
5. **有界 channel 容量**：固定 4，允许 1-2 帧管线并行但不会无界堆积。满时 `.send().await` 反压上游 RTSP 拉流速率，符合"优先丢弃旧帧，不能用阻塞发送反压硬件解码"的精神——反压发生在 Tokio 异步侧，不阻塞硬件解码线程本身；
6. **`Drop` 实现**：drop `Sender`（触发线程侧 `recv` 返回 `None` → 循环退出 → 线程内执行 `mpp_destroy` / `aclvdecDestroyChannel`），再 `JoinHandle::join()` 确保硬件资源完全释放后才离开析构；
7. **错误传播**：线程侧任何 FFI 错误通过 oneshot 回传 `Err(MediaError::Decode { .. })` 或 `Err(MediaError::DecoderInit { .. })`，不 panic。

### 2.2 FrameHandle 类型擦除租约机制

**设计动机**：`FrameHandle` 定义在 `crates/types`（底层），但缓冲区归还逻辑属于 `crates/media`（上层）。
依赖方向 `types ← media` 不可逆，因此 types 层不能引入任何平台池化类型。

**解决方案**：`FrameHandle` 的 `DmaBuf` 与 `DeviceMemory` 变体各持有一个
**类型擦除的 `Arc<dyn Send + Sync>` 租约字段 (`_lease`)**。
types 层只知道"这是一个可跨线程、可共享引用计数的 opaque 对象"；
media 层注入的具体类型（`MppBufferLease`、`DvppBufferLease`）在自身的 `Drop` 中执行平台特定的归还/释放。

**目标 `FrameHandle` 定义**：

```rust
// crates/types/src/frame.rs

pub enum FrameHandle {
    /// Linux DRM DMA-BUF 文件描述符 + 硬件池租约
    #[cfg(target_os = "linux")]
    DmaBuf {
        fd: OwnedFd,
        /// 缓冲区池租约 — 持有期间底层 CMA 物理页不被 VPU 缓冲区组回收复用。
        /// None 仅用于测试或非池化场景。
        _lease: Option<Arc<dyn Send + Sync>>,
    },
    /// 设备显存地址（如 Ascend DVPP）+ 显存池租约
    DeviceMemory {
        ptr: NonNull<c_void>,
        size: usize,
        /// 显存池租约 — 最后一个 Arc 引用析构时归还显存至预分配池
        _lease: Arc<dyn Send + Sync>,
    },
    /// Apple CVPixelBuffer 原生指针封装（macOS CVPixelBufferPool 由系统管理）
    ApplePixelBuffer {
        ptr: NonNull<c_void>,
    },
    /// 主机内存切片（开发测试或 CPU 回退使用）
    Host(Arc<[u8]>),
}
```

**联动变更矩阵**：

| 变体 | Clone | Drop |
|------|-------|------|
| `DmaBuf` | `fd.try_clone()` + `Arc::clone(_lease)` | `OwnedFd` 关闭 dup'd fd → `Arc` 引用计数递减 → 最后一个释放时 lease `Drop` 归还 MPP buffer |
| `DeviceMemory` | copy `ptr`/`size` + `Arc::clone(_lease)` | `Arc` 引用计数递减 → 最后一个释放时 lease `Drop` 归还显存至池 |
| `ApplePixelBuffer` | `CVPixelBufferRetain` | `CVPixelBufferRelease`（不变） |
| `Host` | `Arc::clone` | 自动（不变） |

**关键性质**：

- `Arc` 保证所有 Clone 共享同一租约引用计数，仅最后一个析构时触发归还——无 double-free；
- `_lease` 字段以 `_` 开头表明语义为"仅持有生命周期，不直接调用"；
- 具体的归还逻辑（`MppBufferLease`、`DvppBufferLease`）由 media 层定义，types 层零平台知识。

### 2.3 异构硬件缓冲区池设计

#### 核心痛点

在嵌入式 Linux（DRM CMA 内存池）或昇腾 DVPP 上，`ioctl(DRM_IOCTL_MODE_CREATE_DUMB)` 或 `acldvppMalloc` 是内核级重型操作。如果每帧解码都临时向内核申请物理连续显存、用完立即销毁，高帧率多路并发下会导致 CMA 碎片化，引发极高 sys 负载甚至 OOM。

#### 2.3.1 MPP — 复用 MPP 内置 Buffer Group

Rockchip MPP 内置缓冲区组管理机制（`mpp_buffer_group`），无需自建池：

```text
┌─────────────────────────────────────────────────────────┐
│  MPP Buffer Group (启动时 / info_change 时配置)          │
│  ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐   ...  ┌─────┐      │
│  │ buf0│ │ buf1│ │ buf2│ │ buf3│         │bufN │      │
│  └──┬──┘ └──┬──┘ └──┬──┘ └──┬──┘         └──┬──┘      │
│     │  CMA DMA-BUF  │       │                │         │
└─────┼───────────────┼───────┼────────────────┼─────────┘
      │               │       │                │
   in_use          in_use   free             free
   (held by        (held by
    pipeline)       pipeline)
```

**工作流程**：

1. 首次 `info_change` 事件时，配置缓冲区组容量：
   ```rust
   // H.264 最大 16 参考帧 + 4 管线深度裕量 = 20
   let buf_count: u32 = 20;
   mpi->control(ctx, MPP_DEC_SET_FRAME_BUFFER_COUNT, &buf_count);
   ```
2. 解码收帧后，通过 `mpp_buffer_inc_ref(mpp_buf)` 增持缓冲区引用——
   MPP 不会回收此 buffer，底层 CMA 页锁定；
3. 同时 `dup(mpp_buffer_get_fd(buf))` 获取独立 `OwnedFd`——
   给下游 RKNN 提供独立的 DMA-BUF fd 引用；
4. 构建 `FrameHandle::DmaBuf { fd, _lease: Some(Arc::new(MppBufferLease { buf })) }`；
5. 帧在管线中流转（Clone = Arc 引用计数递增）；
6. 最后一个 FrameHandle 析构 → `MppBufferLease::drop` 调用 `mpp_buffer_put` →
   buffer 归还 Group → CMA 页可被下一帧复用。

**`MppBufferLease`**（media 层具体类型）：

```rust
// crates/media/src/decoders/mpp.rs

/// MPP 缓冲区池租约 — Drop 时归还 buffer 至 MPP buffer group
struct MppBufferLease {
    buf: *mut c_void, // MppBuffer handle
}

// SAFETY: MppBuffer 的引用计数操作本身线程安全（MPP 内部加锁），
//         且此 lease 仅在 Drop 时调用 mpp_buffer_put，不做其他访问
unsafe impl Send for MppBufferLease {}
unsafe impl Sync for MppBufferLease {}

impl Drop for MppBufferLease {
    fn drop(&mut self) {
        // SAFETY: buf 由 mpp_buffer_inc_ref 持有有效引用
        unsafe { mpp_buffer_put(self.buf); }
    }
}
```

**dup() + inc_ref 双重持有的必要性**：

- `mpp_buffer_inc_ref`：阻止 MPP buffer group 回收底层 CMA 物理页，是"不被覆写"的保证；
- `dup(fd)`：生成独立内核 fd 引用，下游 RKNN 可直接以此 fd 做 DMA 映射，OwnedFd RAII 自动关闭；
- 两者缺一不可：只 dup 不 inc_ref → MPP 覆写 CMA 页导致数据损坏；只 inc_ref 不 dup → fd 生命周期绑定在 MPP 内部，无法安全移交下游。

#### 2.3.2 DVPP — 自建预分配显存池

昇腾 DVPP 无内置缓冲区组机制，需在 media 层构建 `DvppBufferPool`：

```text
┌──────────────────────────────────────────────────────────┐
│  DvppBufferPool (解码线程启动时预分配)                      │
│  ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐   ...  ┌─────┐        │
│  │ blk0│ │ blk1│ │ blk2│ │ blk3│         │blkN │        │
│  └──┬──┘ └──┬──┘ └──┬──┘ └──┬──┘         └──┬──┘        │
│     │  Device Memory │       │                │           │
│   leased          leased   available        available     │
│   (pipeline)      (pipeline)  ▲               ▲           │
│                               │               │           │
│                    ◄──── return_buffer() ─────┘           │
│                    (from any thread via Arc<Pool>)         │
└──────────────────────────────────────────────────────────┘
```

**池实现**（位于 `crates/media/src/buffer_pool.rs`）：

```rust
use std::sync::{Arc, Condvar, Mutex};

/// DVPP 设备显存预分配池
pub(crate) struct DvppBufferPool {
    /// 预分配的显存块地址列表（固定大小，不增长）
    blocks: Vec<*mut c_void>,
    /// 每块的字节大小
    block_size: usize,
    /// 可用缓冲区索引（LIFO for cache locality）
    available: Mutex<Vec<usize>>,
    /// 有缓冲区归还时通知等待中的解码线程
    not_empty: Condvar,
}

impl DvppBufferPool {
    /// 启动时预分配 `count` 个显存块
    pub fn new(block_size: usize, count: usize) -> Result<Self, MediaError> {
        let mut blocks = Vec::with_capacity(count);
        for _ in 0..count {
            let mut ptr = std::ptr::null_mut();
            // SAFETY: acldvppMalloc 分配 DVPP 专属连续显存
            let ret = unsafe { acldvppMalloc(&mut ptr, block_size) };
            if ret != 0 {
                // 释放已分配的块
                for &p in &blocks { unsafe { acldvppFree(p); } }
                return Err(MediaError::DecoderInit { /* ... */ });
            }
            blocks.push(ptr);
        }
        let available = Mutex::new((0..count).collect());
        Ok(Self { blocks, block_size, available, not_empty: Condvar::new() })
    }

    /// 租借一个缓冲区（解码线程调用，池空时阻塞等待归还）
    pub fn acquire(&self) -> (*mut c_void, usize) {
        let mut avail = self.available.lock().unwrap();
        while avail.is_empty() {
            avail = self.not_empty.wait(avail).unwrap();
        }
        let idx = avail.pop().unwrap(); // LIFO
        (self.blocks[idx], self.block_size)
    }

    /// 归还一个缓冲区（任意线程可调用 — 通过 FrameHandle lease Drop 触发）
    pub fn return_buffer(&self, ptr: *mut c_void) {
        let idx = self.blocks.iter().position(|&p| p == ptr)
            .expect("unknown buffer returned to pool");
        let mut avail = self.available.lock().unwrap();
        avail.push(idx);
        self.not_empty.notify_one();
    }
}

impl Drop for DvppBufferPool {
    fn drop(&mut self) {
        // 池析构时批量释放所有预分配显存
        for &ptr in &self.blocks {
            // SAFETY: 所有租约已归还（解码器 Drop 先 join 线程，确保管线排空）
            unsafe { acldvppFree(ptr); }
        }
    }
}
```

**容量计算**：
- `count = decode_ref_frames + pipeline_depth + margin`
- H.264: 16 ref + 4 pipeline = 20
- H.265: 16 ref + 4 pipeline = 20
- 每块大小: `stride_w * stride_h * 3 / 2`（NV12）

**`DvppBufferLease`**（租约类型）：

```rust
// crates/media/src/decoders/dvpp.rs

/// DVPP 显存池租约 — Drop 时归还缓冲区至预分配池
struct DvppBufferLease {
    ptr: *mut c_void,
    pool: Arc<DvppBufferPool>,
}

// SAFETY: ptr 指向的 Device Memory 在进程地址空间内全局有效；
//         pool 通过 Arc 跨线程共享；归还操作内部有 Mutex 保护
unsafe impl Send for DvppBufferLease {}
unsafe impl Sync for DvppBufferLease {}

impl Drop for DvppBufferLease {
    fn drop(&mut self) {
        self.pool.return_buffer(self.ptr);
    }
}
```

#### 2.3.3 池容量与背压

| 参数 | MPP | DVPP |
|------|-----|------|
| 池来源 | MPP 内置 buffer group | 自建 `DvppBufferPool` |
| 初始化时机 | 首次 `info_change` | 解码线程启动时 |
| 容量 | 20 buffers (H.264/H.265) | 20 buffers |
| 租借阻塞 | MPP 内部排队 | `Condvar::wait` |
| 归还触发 | `mpp_buffer_put` (via `MppBufferLease::Drop`) | `pool.return_buffer` (via `DvppBufferLease::Drop`) |
| 背压传导 | 池满 → 解码线程阻塞 → mpsc channel 满 → Tokio 侧 `.send().await` 挂起 → RTSP 拉流减速 | 同左 |
| 统计上报 | `BufferPoolStats` 原子计数 | 同左 |

### 2.4 Feature 互斥编译保障

同一边缘设备不可能同时具备 Rockchip VPU 与昇腾 DVPP，两个 feature 同时启用无实际意义且会导致工厂函数 `#[cfg]` 分支歧义。在 `crates/media/src/decoders/mod.rs` 顶部增加编译期断言：

```rust
#[cfg(all(feature = "mpp", feature = "dvpp"))]
compile_error!(
    "Features `mpp` and `dvpp` are mutually exclusive — \
     a single edge device cannot have both Rockchip VPU and Ascend DVPP."
);
```

---

## 3. Rockchip MPP 解码器设计 (`mpp.rs`)

### 3.1 结构体分层

**Async 外壳**（实现 `VideoDecoder` trait，由 Tokio 任务持有）：
```rust
pub struct MppDecoder {
    tx: tokio::sync::mpsc::Sender<DecodeCommand>,
    thread: Option<std::thread::JoinHandle<()>>,
}
```

**线程内部状态**（由专用解码线程独占，不跨线程共享）：
```rust
struct MppDecoderInner {
    camera_id: String,
    codec: CodecType,
    ctx: *mut c_void,      // MppCtx
    mpi: *mut c_void,      // MppApi
    packet: *mut c_void,   // MppPacket 重用实例
    frame: *mut c_void,    // MppFrame 接收容器
    width: u32,
    height: u32,
    hor_stride: u32,       // (width + 15) & !15
    ver_stride: u32,       // (height + 15) & !15
    is_initialized: bool,
}
```

### 3.2 解码循环与 DMA-BUF 零拷贝提取

以下所有步骤均在专用解码线程内执行：

1. **初始化**（线程启动时）：
   - 调用 `mpp_create(&mut ctx, &mut mpi)` 创建 VPU 实例；
   - 调用 `mpp_init(ctx, MPP_CTX_DEC, coding_type)` 初始化 H.264/H.265 格式；
   - 配置 `MPP_DEC_SET_PARSER_SPLIT_MODE = 1`（开启 MPP 内部自动分包解析）；
   - 初始化成功后进入命令接收循环 `while let Some(cmd) = rx.blocking_recv()`。
2. **info_change 处理**（首次收帧时触发）：
   - 读取解码参数 `width`、`height`、`hor_stride`、`ver_stride`；
   - 配置 MPP buffer group 容量为 20（`MPP_DEC_SET_FRAME_BUFFER_COUNT`）；
   - 此后 MPP 内部预分配 CMA DMA-BUF 并循环复用。
3. **送包 (`DecodeCommand::Decode`)**：
   - 将接收到的 Annex B NALU 写入 `MppPacket`；
   - 调用 `mpi->decode_put_packet(ctx, packet)` 送入硬件 VPU。
4. **收帧与句柄构建**（池化零分配路径）：
   - 调用 `mpi->decode_get_frame(ctx, &frame)`；
   - 若 `frame` 有效且未出错：
     - 获取底层硬件 buffer：`mpp_buf = mpp_frame_get_buffer(frame)`；
     - **增持引用**：`mpp_buffer_inc_ref(mpp_buf)` — 阻止 MPP 回收此 CMA 页；
     - **dup fd**：`libc::dup(mpp_buffer_get_fd(mpp_buf))` → `OwnedFd` — 给下游 RKNN 独立 fd；
     - **构建租约**：`Arc::new(MppBufferLease { buf: mpp_buf })` → `_lease`；
     - 组装 `FrameHandle::DmaBuf { fd, _lease: Some(lease) }` → `FrameRef`；
     - 通过 `oneshot` 回传；
     - **释放 MppFrame**（但不释放 MppBuffer — 由 lease 管理）。
5. **资源释放**（channel 关闭、循环退出后）：
   - 调用 `mpp_destroy(ctx)`，彻底销毁底层硬件 VPU 句柄与 buffer group。

---

## 4. 华为昇腾 DVPP 解码器设计 (`dvpp.rs`)

### 4.1 结构体分层

**Async 外壳**（实现 `VideoDecoder` trait，由 Tokio 任务持有）：
```rust
pub struct DvppDecoder {
    tx: tokio::sync::mpsc::Sender<DecodeCommand>,
    thread: Option<std::thread::JoinHandle<()>>,
}
```

**线程内部状态**（由专用解码线程独占）：
```rust
struct DvppDecoderInner {
    camera_id: String,
    codec: CodecType,
    channel_desc: *mut c_void, // aclvdecChannelDesc
    pool: Arc<DvppBufferPool>, // 预分配显存池（Arc 用于注入 lease）
    width: u32,
    height: u32,
    is_initialized: bool,
}
```

### 4.2 DVPP 解码与 Device 显存池化流转

以下所有步骤均在专用解码线程内执行：

1. **初始化**（线程启动时）：
   - 计算对齐步长：`stride_w = (width + 15) / 16 * 16`，`stride_h = (height + 1) / 2 * 2`；
   - 计算帧缓冲区大小：`block_size = stride_w * stride_h * 3 / 2`；
   - **预分配显存池**：`DvppBufferPool::new(block_size, 20)`，一次性完成 20 次 `acldvppMalloc`；
   - 创建通道描述符并初始化硬件通道；
   - 进入命令接收循环。
2. **送帧与池化显存租借 (`DecodeCommand::Decode`)**：
   - 从 `pool.acquire()` 租借一个预分配显存块（**零内核开销**，池空时阻塞等待归还）；
   - 将显存块地址设为 VDEC 输出目标；
   - 调用 `aclvdecSendFrame(...)` 送入 VPU。
3. **句柄封装**（池化归还路径）：
   - 构建 `DvppBufferLease { ptr: dev_ptr, pool: Arc::clone(&self.pool) }`；
   - 封装为 `FrameHandle::DeviceMemory { ptr, size, _lease: Arc::new(lease) }`；
   - 构建 `FrameRef` 并通过 `oneshot` 回传；
   - 帧在管线中流转（Clone = Arc 引用计数递增）；
   - 最后一个 FrameHandle 析构 → `DvppBufferLease::drop` → `pool.return_buffer(ptr)` →
     显存块归还池，可被下一帧复用。**全程零 `acldvppMalloc` / `acldvppFree` 调用**。
4. **资源释放**（channel 关闭、循环退出后）：
   - 调用 `aclvdecDestroyChannel(channel_desc)` 销毁硬件通道；
   - `DvppBufferPool::drop` 批量 `acldvppFree` 所有预分配块。

---

## 5. 工厂分发与条件编译策略 (`decoders/mod.rs`)

```rust
// 编译期互斥保障
#[cfg(all(feature = "mpp", feature = "dvpp"))]
compile_error!(
    "Features `mpp` and `dvpp` are mutually exclusive — \
     a single edge device cannot have both Rockchip VPU and Ascend DVPP."
);

pub fn create_decoder(camera_id: &str, codec: CodecType) -> Box<dyn VideoDecoder + Send> {
    #[cfg(target_os = "macos")]
    {
        Box::new(VideoToolboxDecoder::new(camera_id, codec))
    }
    #[cfg(all(target_os = "linux", feature = "mpp"))]
    {
        Box::new(MppDecoder::new(camera_id, codec))
    }
    #[cfg(all(target_os = "linux", feature = "dvpp"))]
    {
        Box::new(DvppDecoder::new(camera_id, codec))
    }
    #[cfg(not(any(
        target_os = "macos",
        all(target_os = "linux", feature = "mpp"),
        all(target_os = "linux", feature = "dvpp")
    )))]
    {
        Box::new(MockDecoder::new(camera_id, codec, 1920, 1080))
    }
}
```

---

## 6. FFI 安全性与跨平台编译保障 (Safety & Portability)

1. **Unsafe 隔离**：
   - 裸指针调用（MPP C API / ACL C API）严格收敛于 `decoders/mpp.rs` 与 `decoders/dvpp.rs` 的线程内部状态中；
   - 每一个 `unsafe` 块必须带有清晰的 `// SAFETY:` 注释，严禁裸指针外溢至 safe 接口。

2. **DmaBuf Clone 安全注记**：
   - 当前 `FrameHandle::DmaBuf` 的 `Clone` 使用 `OwnedFd::try_clone()`，失败时 `panic!`；
   - 生产环境下 `dup()` 极少失败（仅 fd 耗尽），本任务不改动此逻辑（与 MPP/DVPP 无直接关联），后续可改为 fallible `try_clone()` 方法。

3. **线程安全与上下文亲和性**：
   - 硬件解码上下文（`MppCtx`、`aclvdecChannelDesc`）和缓冲区池在专用线程内创建、使用和销毁，**不跨线程转移**；
   - 仅 `FrameRef`（内含 `FrameHandle`）跨线程流转，其 `Send + Sync` 由 `OwnedFd` / `Arc<dyn Send + Sync>` / `Arc<[u8]>` 各自保证；
   - 缓冲区归还（`MppBufferLease::drop`、`DvppBufferLease::drop`）可在任意线程执行——前者 MPP 内部有锁保护，后者 `DvppBufferPool` 通过 `Mutex` 保护。

4. **CI 与开发机无硬件编译保障**：
   - 在未开启 `mpp` 或 `dvpp` feature 时，对应源文件不参与编译（`#[cfg(all(target_os = "linux", feature = "..."))]` 守卫模块声明）；
   - 保证常规 `cargo check --workspace`、`cargo clippy --all-targets -- -D warnings` 与 `cargo test --workspace` 在 macOS / 标准 x86 Linux 上保持 100% 绿灯；
   - 硬件集成测试标记 `#[ignore]`，仅在对应边缘设备上手动 `cargo test -- --ignored` 执行。
