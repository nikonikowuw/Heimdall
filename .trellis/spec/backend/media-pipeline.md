# 媒体管线规范

> 从 RTSP 拉流到推理输入之间的全部处理。**核心目标：整条路径上不发生一次 CPU 侧的帧数据拷贝。**

> ⚠️ **状态：立项约定（尚未经代码验证）**
> `FrameRef` 设计是草案，首批解码实现落地后必须回填真实类型与生命周期规则，并删除本提示。

---

## 为什么零拷贝是硬要求

一帧 1080p NV12 约 3 MB。8 路 × 15fps：

- 每多一次 CPU 拷贝 = **360 MB/s 内存带宽**
- RK3568 的内存带宽本来就紧张，几次拷贝就把整个系统压垮
- 拷贝还伴随 cache 污染，影响所有其它任务

三个目标平台都提供了硬件级的零拷贝通路。**用不上它们，Argus 就跑不动。**

---

## 平台零拷贝通路

| 平台 | 解码输出 | 传递载体 | 预处理 | 推理输入 |
|------|---------|---------|--------|---------|
| Rockchip | MPP 解码到 DMA-BUF | DMA-BUF fd | RGA（缩放/格式转换） | RKNN 直接吃 DMA-BUF fd |
| Ascend | DVPP 解码到 device 内存 | DVPP buffer 指针 | AIPP（融进模型）或 DVPP | AscendCL 直接吃 device 内存 |
| Apple | VideoToolbox 输出 CVPixelBuffer | `IOSurface` 支撑的 `CVPixelBuffer` | vImage / Metal | Core ML 直接吃 CVPixelBuffer |
| CPU 回退 | 软解到堆内存 | `Vec<u8>` | CPU 缩放 | 普通内存 |

**规则**：CPU 回退路径只用于开发机调试，不是生产路径。不要为了"简单"让设备走 CPU 路径。

各平台的具体 API 调用序列不写在 spec 里 —— 由 `rknn-pro`、`ascend-pro` 技能与 Apple 官方 VideoToolbox / Core ML 文档承载。spec 只规定抽象边界。

---

## `FrameRef`：跨平台的帧引用

```rust
// crates/types/src/frame.rs

/// 内存步长与对齐信息（严禁假设 stride == width）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrideInfo {
    /// 水平虚宽 / 步长（按字节数或像素跨度对齐后的每行跨度）
    pub hor_stride: u32,
    /// 垂直虚高（硬件分配所对齐的真实内存行数，如 1080p 在 MPP 下常为 1088）
    pub ver_stride: u32,
}

/// 一帧的持有句柄。满足 `Send + 'static`，可在跨线程通道中转移所有权。
/// 负责在 Drop 时将底层 buffer 归还到对应的预分配池。
pub struct FrameRef {
    pub camera_id: CameraId,
    /// 13 位 UTC Unix 毫秒。整个系统统一用这个时间基准。
    pub timestamp: i64,
    pub width: u32,
    pub height: u32,
    pub stride: StrideInfo,
    pub format: PixelFormat,
    /// 平台原生句柄封装
    handle: FrameHandle,
}

impl FrameRef {
    pub fn new(
        camera_id: CameraId,
        timestamp: i64,
        width: u32,
        height: u32,
        stride: StrideInfo,
        format: PixelFormat,
        handle: FrameHandle,
    ) -> Self {
        Self { camera_id, timestamp, width, height, stride, format, handle }
    }

    /// 获取底层平台句柄（供 media 解码池与 infer 驱动使用，pipeline 业务层不解构）
    pub fn handle(&self) -> &FrameHandle {
        &self.handle
    }
}

pub enum FrameHandle {
    #[cfg(target_os = "linux")]
    DmaBuf { fd: std::os::fd::OwnedFd },
    /// 设备内存地址（如 Ascend DVPP 显存指针，保持 types 纯净不引入平台专有库）
    DeviceMemory { ptr: std::ptr::NonNull<std::ffi::c_void>, size: usize },
    /// Apple CVPixelBuffer 裸指针封装
    ApplePixelBuffer { ptr: std::ptr::NonNull<std::ffi::c_void> },
    /// 主机内存切片（仅用于开发测试）
    Host(Arc<[u8]>),
}
```

约定：

- **`FrameRef` 定义在 `types`**，因为 `media` 和 `infer` 都要用到它，而这两个 crate 互不依赖。`types` 是纯净叶子 crate，不引入平台专有 SDK 依赖。
- **满足 `Send + 'static`**：`FrameRef` 必须能直接通过有界通道在跨线程间传递（从解码线程移交至推理线程），不允许包含局部栈生命周期借用 `'a`。生命周期管理必须由句柄的 RAII `Drop` 承载，归还给 buffer 池。
- **`handle` 访问收敛**：通过 `pub fn handle(&self) -> &FrameHandle` 暴露给 `media` 与 `infer`，上层业务与调度代码不解构 `FrameHandle`。Linux 下 DMA-BUF 统一使用具备所有权语义的 `OwnedFd` 避免句柄泄漏。
- **严格遵循平台对齐约束**：
  - Rockchip MPP 解码 NV12 (YUV420SP) 必须处理 `hor_stride`（16 或 64 对齐）和 `ver_stride`（16 字节虚高对齐），RGA 预处理必须传入对齐后的 stride，不可直接用 `width/height`。
  - 华为昇腾 DVPP 输出内存必须满足宽 16 对齐、高 2 对齐、行 stride 128 对齐约束。
  - Apple VideoToolbox 输出必须绑定 `IOSurface` 属性，确保 Core ML 与 Metal 能实现真正的内存直通。

---

## 解码器抽象 (`VideoDecoder`)

视频硬解能力在 `media` crate 内部通过统一 trait 抽象：

```rust
// crates/media/src/decoder.rs
pub trait VideoDecoder: Send {
    /// 喂入 H.264 / H.265 的 NALU 或裸流切片
    fn send_packet(&mut self, packet: &[u8], pts_ms: i64) -> Result<(), MediaError>;

    /// 提取解码后的池化零拷贝帧（硬件解码中则返回 Ok(None)）
    fn receive_frame(&mut self) -> Result<Option<FrameRef>, MediaError>;

    /// 刷新解码器状态（RTSP 重连或 seek 时调用）
    fn flush(&mut self) -> Result<(), MediaError>;
}
```

- **解码实例绑定专用 OS 线程**：每路摄像头拥有独立的拉流与解码线程，不放入 Tokio 异步工作线程池；
- **平台工厂收敛**：`media/src/factory.rs` 根据平台编译宏（`decoder-mpp`、`decoder-videotoolbox`、`decoder-ffmpeg`）构造对应的解码实现，上层通过 `Box<dyn VideoDecoder>` 调用。

---

## 缓冲区管理

**帧 buffer 必须来自预分配的池，不允许每帧现分配。**

```
启动时：按 (路数 × 队列深度) 预分配 DMA-BUF / CVPixelBuffer 池
运行时：解码器从池里租 → 管线使用 → Drop 时归还
```

规则：

- **池容量固定且有上界**。池空了就丢帧（并计数），不动态扩容。
- **必须显式释放平台句柄**。DMA-BUF fd 泄漏会在几小时内耗尽进程 fd 上限；CVPixelBuffer 泄漏会 OOM。用 RAII（`Drop` 实现）保证归还，但 `Drop` 里只做归还，**不做阻塞操作**。
- 跨线程传递帧时转移所有权（通过通道），**不要 clone 帧**。

---

## 抽帧与门控

不是每一帧都要送进 NPU。管线的门控顺序：

```
解码输出 (25fps)
   │
   ▼ ① 抽帧：按配置降到检测帧率（例如 5fps）
   │
   ▼ ② 运动检测：低成本的帧差/背景建模，无运动直接丢弃
   │
   ▼ ③ 区域掩码：只关心配置的检测区域
   │
   ▼ 送 NPU（实际可能只剩 0.5fps）
```

**这是设计前提，不是优化**。不做门控直接全帧推理，8 路 1080p 在任何目标设备上都跑不动。

约定：

- 抽帧、运动检测、区域掩码的参数**按摄像头配置**，不是全局常量。
- 运动检测在 CPU 上做（成本远低于 NPU），但要在**降采样后的小图**上做（例如 320×180），不在原分辨率上做。
- **DMA-BUF CPU 访问一致性**：若在 CPU 上映射 DMA-BUF 读取小图，**必须在读前后调用 `DMA_BUF_IOCTL_SYNC`**（进行 `DMA_BUF_SYNC_START` 和 `DMA_BUF_SYNC_END` 的 Cache Invalidation），否则 CPU 会读取到 L1/L2 Cache 脏数据。
- 门控逻辑在 `pipeline`，不在 `media`。解码器只负责解码。

---

## 预处理（硬件专属与内聚原则）

预处理（缩放、色彩空间转换 NV12->RGB、步长对齐）优先使用芯片硬件 2D 加速器：

| 平台 | 硬件预处理单元 | 直通方式 | 回退方案 |
|------|--------------|---------|---------|
| Rockchip | **RGA (2D 硬件引擎)** | DMA-BUF fd 直通 RGA 缩放为目标尺寸 fd | CPU SIMD（仅本地调试） |
| Ascend | **AIPP**（优先融进模型）/ **DVPP VPC** | 设备内存地址直接缩放对齐 | CPU |
| Apple | **Core ML 预处理层** / **Metal Shaders** | `CVPixelBuffer` 零拷贝直通 | CPU |
| CPU 回退 | **fast_image_resize** (NEON/AVX2) | 堆内存双线性插值 | 纯标量计算 |

约定：

- **预处理内聚在 Backend 内部**：硬件专属预处理算子（RGA/VPC）必须由对应的 `InferenceBackend` 实现自行调度（因为只有后端清楚具体的模型输入 Stride/尺寸/通道偏置要求），**严禁在 `pipeline` 层搞通用的全量 CPU 内存转换**。
- **能融进模型的归一化就融进模型**（Ascend 的 AIPP、Core ML 的 preprocessing layer），零额外计算开销。
- **保留缩放比例与 padding 偏移**：letterbox 缩放产生的边界填充与缩放系数必须随 `Detection` 一同保留，供后处理将坐标精确还原到原始分辨率（必须有单元测试覆盖）。

---

## 摄像头接入、双轨健康感知与三态防抖

- **RTSP 断流与网络波动是常态**：
  - 每路独立重连，**必须有指数退避**（1s → 2s → 4s → 上限 30s）。
  - 重连时释放旧的解码器与 buffer 池租约，杜绝内存与句柄泄漏。
- **三态防抖健康模型（Anti-Flapping 3-State Model）**：
  - 避免网络单包丢失导致状态在红绿灯之间频繁横跳：
    - `Healthy 🟢`：码流接收正常，探活连续成功；
    - `Degraded 🟡`：发生首次瞬时网络抖动或正在重连，容错缓冲窗口（连续重试 3 次或持续 10 秒内不向用户报死亡）；
    - `Failed 🔴`：连续 3 次探活失败或重连超过容错上限无数据，才正式判定为离线并记录错误日志。
- **双轨感知机制**：
  - **活跃拉流流**：由 RTSP 接收 Actor 实时感知断线并触发重连；
  - **静默待机流**：由后台定时巡检任务（30s 周期）轮询执行轻量探活并同步状态。
- 连续失败超过阈值 → `warn!` 记录并上报状态，但**不退出进程**，也不影响其它路。
- 单路故障不允许拖垮共享资源（NPU、数据库连接池）。

---

## 禁止事项

- ❌ 用 `Vec<u8>` 在管线各阶段之间传帧数据
- ❌ 每帧调用 `malloc` / `Vec::with_capacity` 分配 buffer
- ❌ 在原分辨率上做运动检测
- ❌ CPU 访问 DMA-BUF 未做 `DMA_BUF_IOCTL_SYNC`
- ❌ 忽略硬件虚高步长（直接把 `width/height` 当作 stride 传给硬件加速器）
- ❌ 跳过门控直接全帧送 NPU
- ❌ `FrameHandle` 在 `media` / `infer` 之外被 match
- ❌ 无退避的重连循环
- ❌ 忘记释放平台句柄（fd / CVPixelBuffer / device 内存）

---

## 待验证事项

- [ ] `FrameHandle` 的 `#[cfg]` 组合方式（按 `target_os` 还是按 feature，两者语义不同）
- [ ] 帧池容量与队列深度的真机取值
- [ ] 各平台实际步长对齐参数（如 RK3568/RK3576 MPP 在 1080p 下的 `hor_stride/ver_stride`）
- [ ] Ascend DVPP buffer 是否需要显式的 device 上下文绑定才能跨线程使用
- [ ] 运动检测算法选型（帧差 vs MOG2）与其 CPU 开销实测
