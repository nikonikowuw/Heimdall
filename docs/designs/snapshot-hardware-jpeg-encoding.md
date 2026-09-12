# Design: Snapshot 路径全链路硬件 JPEG 编码与设备侧裁剪

> **状态**: Draft  
> **作者**: Heimdall Engineering  
> **日期**: 2025-07-21  
> **关联规范**: [媒体管线](../nuwa/backend/media-pipeline.md)、[算法 SDK](../nuwa/backend/algo-sdk-guidelines.md)、[并发模型](../nuwa/backend/concurrency-guidelines.md)、[FFI 边界](../nuwa/backend/ffi-guidelines.md)

---

## 1. 问题陈述

### 1.1 当前 Snapshot 路径的性能瓶颈

当前 `snapshot_readback_path` 实现链路：

```
设备帧 (DMA-BUF / CVPixelBuffer / DeviceMemory)
  → D2H readback（拷贝全尺寸原始像素 ~3MB for 1080p NV12）
  → CPU NV12→RGB 色彩空间转换（读 3MB → 写 6MB）
  → CPU JPEG 编码 (image::codecs::jpeg::JpegEncoder)
  → 原子写盘
```

**有检测目标时**，还需额外执行特写裁剪：

```
  RGB 图像 → crop_with_padding (CPU 像素裁切) → CPU JPEG 编码(q90) → 写盘
```

**瓶颈分析**（RK3568，1080p，有裁剪场景）：

| 阶段                  | 数据量                 | 耗时          | 说明             |
| ------------------- | ------------------- | ----------- | -------------- |
| D2H readback        | 3.2MB NV12          | 2-5ms       | 与 NPU/RGA 竞争总线 |
| CPU NV12→RGB        | 读 3.2MB → 写 6.2MB   | 3-8ms       | 定点数 BT.601 转换  |
| 全景 CPU JPEG(q85)    | 读 6.2MB → 输出 ~200KB | 3-10ms      | libjpeg-turbo  |
| 特写 crop + JPEG(q90) | 裁切 + 编码 ~50KB       | 3-10ms      | 复用 RGB，再编码一次   |
| **端到端**             |                     | **11-33ms** |                |

### 1.2 优化机会

三个平台均具备设备侧 JPEG 编码能力，部分平台还支持设备侧裁剪（RGA/DVPP crop）：

| 平台           | JPEG 编码                     | 设备侧 Crop                   | DMA-BUF 串联          | 现有封装 |
| ------------ | --------------------------- | -------------------------- | ------------------- | ---- |
| Rockchip MPP | VPU JPEG 模式                 | RGA `crop_and_scale`       | MPP ← DMA-BUF ← RGA | 仅解码器 |
| Apple VT     | `VTCompressionSession` JPEG | Core Image / Metal compute | CVPixelBuffer 零拷贝   | 仅解码器 |
| Ascend DVPP  | `acldvppJpegEncode`         | `acldvppCrop`              | DVPP ← DeviceMemory | 仅解码器 |
| CPU fallback | libjpeg-turbo               | `crop_with_padding`        | N/A                 | 已实现  |

**决策：采用全链路硬件方案（方案 B）**，全景大图与特写裁剪均走设备侧处理，消除所有 CPU 像素操作。

### 1.3 方案对比与决策依据

| 维度       | 方案 A（仅全景 HW）        | **方案 B（全景+特写 HW）**                   |
| -------- | ------------------- | ------------------------------------ |
| 全景路径     | D2H → HW JPEG       | HW JPEG（无 D2H）                       |
| 特写路径     | CPU crop → CPU JPEG | RGA/DVPP crop → HW JPEG              |
| 端到端延迟    | 9-26ms（改善 15-25%）   | **2.5-7ms（改善 60-75%）**               |
| D2H 数据量  | 3.2MB（仍需 RGB）       | **~200KB + ~50KB（仅 JPEG bitstream）** |
| CPU 像素操作 | NV12→RGB 仍存在        | **完全消除**                             |
| 实现复杂度    | 低                   | 中高（需 RGA/DVPP crop 串联）               |

方案 B 的关键收益：**将 D2H 数据量从 3.2MB 缩减到 ~250KB（13× 减少），端到端延迟从 11-33ms 降至 2.5-7ms**。在 4 路同时告警场景下，每路节省 8-26ms，累计减少 CPU 占用约 40-100ms。

---

## 2. 架构约束与设计原则

### 2.1 三路径边界不可违背

本设计**严格限定在 `snapshot_readback_path` 内**：

- `infer_fast_path`：设备帧 → 设备预处理 → NPU，零 CPU 像素拷贝（不变）
- `snapshot_readback_path`：设备帧 → **设备 crop + 设备 JPEG 编码** → 仅压缩后 D2H → 写盘（本设计）
- `debug_cpu_fallback_path`：Host 内存 → CPU 软件 crop + CPU JPEG（不变，同时作为硬件路径的 fallback）

### 2.2 平台差异收敛在 `media` 层

- 裁剪 + 编码能力抽象为 `media` 层的 trait
- 上层 `pipeline::snapshot` 不感知底层是硬件还是 CPU 回退
- 遵循 [FFI 规范](../nuwa/backend/ffi-guidelines.md)：`unsafe`、裸指针集中在 FFI/sys 边界

### 2.3 并发与线程模型

- 编码器上下文（MPP 编码实例、DVPP 编码通道）**常驻专用 OS Worker**，不逐帧创建
- 裁剪（RGA/DVPP crop）与编码（MPP/DVPP JPEGE）的 FFI 调用在 OS Worker 内同步执行
- 同一编码实例不并发调用；多路告警时排队或使用独立编码实例池

### 2.4 错误恢复与多级降级

```
Level 0（最优）：设备 crop + 设备 JPEG 编码（全链路零 CPU 像素操作）
    ↓ 失败
Level 1（降级）：D2H readback → CPU crop → 设备 JPEG 编码（部分硬件加速）
    ↓ 失败
Level 2（兜底）：D2H readback → CPU crop → CPU JPEG 编码（纯 CPU 路径）
```

任何一级失败均按能力边界回退；固定容量队列满时当前低频请求会被拒绝并记录，调用方必须保留可重试/告警失败语义。

---

## 3. 平台硬件能力详析

### 3.1 Rockchip：MPP JPEGE + RGA Crop

#### 3.1.1 MPP JPEG 编码

MPP VPU 原生支持 JPEG 编码，与解码器共享 VPU 硬件单元但使用独立 MPP 上下文：

```c
// 创建编码上下文
mpp_create(&ctx, &mpi);
mpp_init(&ctx, MPP_CTX_ENC, MPP_VIDEO_CodingMJPEG);

// 配置 JPEG q_factor（当前 BSP 为 1-99），并设置 prep:colorrange=2、prep:format=MPP_FMT_YUV420SP
MppEncCfg cfg;
mpp_enc_cfg_init(&cfg);
mpp_enc_cfg_set_s32(cfg, "jpeg:q_factor", quality > 99 ? 99 : quality);
mpp_enc_cfg_set_s32(cfg, "prep:colorrange", MPP_FRAME_RANGE_JPEG);
mpp_enc_cfg_set_s32(cfg, "prep:format", MPP_FMT_YUV420SP);

// 输入：按目标 SDK 的 MppBufferInfo 导入外部 DMA-BUF
MppBufferInfo info = {
    .type = MPP_BUFFER_TYPE_EXT_DMA,
    .size = nv12_size(hor_stride, ver_stride),
    .fd = dma_buf_fd,
};
MppBuffer input_buf = NULL;
mpp_buffer_import_with_tag(group, &info, &input_buf, "heimdall", "snapshot");

// 编码执行：异步提交后必须轮询 output port，再取 packet
mpi->encode_put_frame(ctx, frame);
mpi->poll(ctx, MPP_PORT_OUTPUT, 100);
mpi->encode_get_packet(ctx, &packet);

// 输出：JPEG bitstream
void *ptr = mpp_packet_get_data(out_packet);
size_t len = mpp_packet_get_length(out_packet);
```

**约束**：

- 输入 NV12 (`MPP_FMT_YUV420SP`)，与解码输出天然匹配；`mpp_init` 的 coding 参数必须是 `MPP_VIDEO_CodingMJPEG`
- DMA-BUF fd 通过 `MppBufferInfo` + `mpp_buffer_import_with_tag` 导入，不调用不存在于目标 BSP 的 `mpp_buffer_import_dma_buf`
- `encode_put_frame` 后使用有界 output poll（100ms），超时释放 RAII 资源并转 CPU fallback

#### 3.1.2 RGA 设备侧 Crop

RGA 2D 硬件加速器原生支持 crop + scale + border fill：

```c
// 源帧：完整 DMA-BUF
rga_set_src(src_fd, full_width, full_height, NV12);

// 目标帧：裁剪后的 DMA-BUF（由 RGA 输出池分配）
rga_set_dst(dst_fd, crop_w, crop_h, NV12);

// 裁剪 ROI（像素坐标）
rga_set_crop(src_rect, pixel_x1, pixel_y1, crop_w, crop_h);

// 超出边界区域填充黑色
rga_set_fill_color(dst_fd, 0, 0, 0);

// 执行（DMA-BUF → DMA-BUF，零 CPU 拷贝）
rga_run();
```

**约束**：

- RGA 输出 DMA-BUF 需 16 字节对齐（宽）× 16 字节对齐（高）
- crop 后尺寸可能与输入不同，需确保输出 buffer 容量足够
- RGA 与 MPP/DMA-BUF 共享 DRM 子系统，输出可直接作为 MPP 编码输入

#### 3.1.3 全链路串联（零 CPU 拷贝）

```
MPP 解码输出 DMA-BUF
  ├──[路径 1: 全景]──→ MPP JPEGE (全帧) → JPEG bitstream → D2H → 写盘
  │
  └──[路径 2: 特写]──→ RGA crop+pad → 输出 DMA-BUF
                              ↓
                        MPP JPEGE (裁剪帧) → JPEG bitstream → D2H → 写盘
```

**RGA → MPP 串联关键**：RGA 输出的 DMA-BUF 通过目标 SDK 的 `MppBufferInfo` 与 `mpp_buffer_import_with_tag` 导入 MPP 编码上下文；CPU 只在硬件路径失败时执行显式单帧 readback。

### 3.2 Apple：VideoToolbox JPEG + Core Image Crop

#### 3.2.1 VideoToolbox JPEG 编码

```c
// 创建 JPEG 压缩会话
VTCompressionSessionCreate(
    allocator, width, height,
    kCMVideoCodecType_JPEG,
    NULL, NULL, NULL, NULL, NULL,
    &session
);

// 设置质量
VTSessionSetProperty(session, kVTCompressionPropertyKey_Quality, @(quality / 100.0));

// 编码 CVPixelBuffer（零拷贝，IOSurface 直通）
VTCompressionSessionEncodeFrame(session, pixelBuffer, ...);

// 提取 JPEG 数据
CMBlockBufferGetDataPointer(blockBuffer, 0, &lengthAtOffset, &totalLength, &data);
```

#### 3.2.2 设备侧 Crop

macOS/iOS 的 CVPixelBuffer 通过 IOSurface 实现零拷贝 crop：

```
方案 1（推荐）：CVPixelBufferIOFacet
  - IOSurface 原生支持 sub-rectangle crop
  - 零拷贝提取子区域 CVPixelBuffer
  - 直接送入 VTCompressionSession 编码

方案 2：Metal Compute Shader
  - 自定义 compute kernel 执行 crop + padding
  - 输入/输出均为 CVPixelBuffer（IOSurface backing）
  - 适合需要复杂 padding 逻辑的场景
```

**全链路**：

```
解码输出 CVPixelBuffer
  ├──[全景]──→ VTCompressionSession JPEG → bitstream → 写盘
  └──[特写]──→ CVPixelBuffer crop (IOSurface facet)
                     ↓
               VTCompressionSession JPEG → bitstream → 写盘
```

### 3.3 华为 Ascend：DVPP Crop + DVPP JPEGE

#### 3.3.1 DVPP JPEG 编码

```c
// 创建编码通道
acldvppJpegEncodeInit(encConfig, &jpegEncoder);

// 设置输入（NV12 设备内存）
jpegEncodeDesc->inputBuffer = devicePtr;
jpegEncodeDesc->width = width;
jpegEncodeDesc->height = height;

// 执行编码
acldvppJpegEncode(jpegEncoder, jpegEncodeDesc, outputData);

// 输出：DVPP 专属内存（64 字节对齐）→ D2H 拷贝 JPEG bitstream
acldvppMalloc(&outputPtr, outputSize);
aclrtMemcpy(hostPtr, outputSize, outputPtr, outputSize, ACL_MEMCPY_DEVICE_TO_HOST);
```

#### 3.3.2 DVPP Crop

```c
// 创建裁剪通道
acldvppCreatePicDesc(&srcDesc);
acldvppCreatePicDesc(&dstDesc);

// 设置裁剪区域
srcDesc->inputBuffer = devicePtr;
srcDesc->roi.left = pixel_x1;
srcDesc->roi.top = pixel_y1;
srcDesc->roi.width = crop_w;
srcDesc->roi.height = crop_h;

// 执行裁剪（DeviceMemory → DeviceMemory，零 CPU 拷贝）
acldvppCrop(srcDesc, dstDesc);
```

**全链路**：

```
DVPP 解码输出 DeviceMemory
  ├──[全景]──→ acldvppJpegEncode → JPEG bitstream → D2H → 写盘
  └──[特写]──→ acldvppCrop → 裁剪后 DeviceMemory
                     ↓
               acldvppJpegEncode → JPEG bitstream → D2H → 写盘
```

**DVPP 约束**：

- 输入/输出必须通过 `acldvppMalloc` 分配（64 字节对齐）
- crop ROI 宽度 16 字节对齐，高度 2 字节对齐
- 编码与裁剪使用不同 DVPP 通道，可并行

### 3.4 CPU Fallback

硬件不可用或初始化失败时，降级为现有纯 CPU 路径：

```rust
fn encode_snapshot_cpu(frame: &FrameRef, bbox: Option<BoundingBox>, q_full: u8, q_crop: u8)
    -> Result<(Vec<u8>, Option<Vec<u8>>), MediaError>
{
    let rgb = snapshot_readback_to_rgb_image(frame)?;  // D2H + NV12→RGB
    let full_jpeg = encode_jpeg(&rgb, q_full)?;
    let crop_jpeg = bbox.map(|b| {
        let cropped = crop_with_padding(&rgb, b, 0.1);
        encode_jpeg(&cropped, q_crop)
    }).transpose()?;
    Ok((full_jpeg, crop_jpeg))
}
```

---

## 4. 设计方案

### 4.1 整体架构

```
pipeline::snapshot::SnapshotEngine
  │
  ├── decode_target_frame()              // 已有：帧解码
  │
  └── save_snapshot_async()              // 修改：全链路硬件编码
        │
        └── encode_and_save_snapshot()
              │
              ├── select_encoder(frame)  // NEW: 按平台选择编码器
              │     │
              │     ├── MppSnapEncoder  (Linux + RKNN: RGA crop + MPP JPEGE)
              │     ├── VtSnapEncoder   (macOS: IOSurface crop + VT JPEG)
              │     ├── DvppSnapEncoder (Linux + Ascend: DVPP crop + DVPP JPEGE)
              │     └── CpuSnapEncoder  (CPU fallback: CPU crop + CPU JPEG)
              │
              ├── encoder.encode_full_frame()    // 全景大图
              │     └── JPEG bitstream → 写盘
              │
              ├── encoder.encode_crop()          // 特写裁剪（有 bbox 时）
              │     └── JPEG bitstream → 写盘
              │
              └── atomic_write_file()            // 已有：写盘
```

### 4.2 新增模块结构

```
crates/
├── media/src/
│   ├── encoders/
│   │   ├── mod.rs              // DeviceSnapEncoder trait + 工厂函数
│   │   ├── cpu.rs              // CPU 编码 (现有 snapshot_readback_to_rgb_image + encode_jpeg 迁移)
│   │   ├── mpp_snap.rs         // Rockchip: RGA crop + MPP JPEGE 全链路
│   │   ├── videotoolbox_snap.rs // Apple: IOSurface crop + VT JPEG 全链路
│   │   └── dvpp_snap.rs        // Ascend: DVPP crop + DVPP JPEGE 全链路
│   └── image_convert.rs        // 现有文件保留，用于 CPU fallback 路径
├── pipeline/src/
│   └── snapshot.rs             // 修改：SnapshotConfig 新增质量字段，encode_and_save_snapshot 读取配置
├── api/src/routes/system/
│   └── snapshot.rs             // 新增：GET/PUT /api/v1/system/snapshot/config
└── types/src/system.rs         // 修改：新增 SnapshotSystemConfig (API DTO)

web/src/
└── features/system/
    ├── SettingsPage.tsx         // 不变：无新增 Tab
    ├── StorageSettings.tsx      // 修改：新增「图片编码」SettingsSection
    └── (不新增 SnapshotSettings.tsx)
```

### 4.3 核心 Trait 设计

```rust
/// 设备侧快照编码器统一抽象
///
/// 封装「全景大图编码」与「设备侧裁剪 + 特写编码」两个原子操作。
/// 各平台实现负责在设备侧完成裁剪和 JPEG 编码，仅将压缩后
/// JPEG bitstream 拷贝至 CPU 内存用于写盘。
///
/// ## 路径约束
/// 仅用于 `snapshot_readback_path`，不得在 `infer_fast_path` 中调用。
pub trait DeviceSnapEncoder: Send {
    /// 编码器名称（用于日志和降级标识）
    fn name(&self) -> &'static str;

    /// 将设备原生帧编码为全景 JPEG 字节流
    ///
    /// 设备侧零 CPU 像素操作：直接从 DMA-BUF / CVPixelBuffer / DeviceMemory
    /// 硬件编码为 JPEG bitstream，输出在 CPU 内存中。
    fn encode_full_frame(
        &self,
        frame: &FrameRef,
        quality: u8,
    ) -> Result<Vec<u8>, MediaError>;

    /// 设备侧裁剪 + JPEG 编码
    ///
    /// 在设备侧执行 crop（含 padding 扩展）和 JPEG 编码，
    /// 全程无 CPU 像素操作。
    ///
    /// - `bbox`: 归一化裁剪区域 [0.0, 1.0]
    /// - `padding_ratio`: 边界扩展比例（如 0.1 = 10%）
    /// - `quality`: JPEG 质量 (1-100)
    fn encode_crop(
        &self,
        frame: &FrameRef,
        bbox: BoundingBox,
        padding_ratio: f32,
        quality: u8,
    ) -> Result<Vec<u8>, MediaError>;

    /// 编码器是否就绪（已初始化且硬件可用）
    fn is_ready(&self) -> bool;
}
```

### 4.4 Rockchip 实现：MppSnapEncoder

```rust
/// Rockchip MPP 全链路快照编码器
///
/// 全景：DMA-BUF → MPP JPEGE → JPEG bitstream
/// 特写：DMA-BUF → RGA crop+pad → DMA-BUF → MPP JPEGE → JPEG bitstream
pub struct MppSnapEncoder {
    /// MPP 编码上下文（常驻）
    mpp_ctx: MppEncContext,
    /// RGA 上下文（常驻，用于 crop 操作）
    rga_ctx: RgaContext,
    /// RGA 输出单块预分配 DMA-BUF 画板（Scratchpad，固定最大分辨率，杜绝 CMA 碎片）
    crop_scratchpad: DmaBufScratchpad,
}

impl DeviceSnapEncoder for MppSnapEncoder {
    fn name(&self) -> &'static str { "mpp-snap" }

    fn encode_full_frame(&self, frame: &FrameRef, quality: u8) -> Result<Vec<u8>, MediaError> {
        let fd = frame.handle_dmabuf_fd()?;
        // 1. DMA-BUF fd → mpp_buffer_import (零拷贝)
        // 2. MPP 编码全帧 → JPEG bitstream
        // 3. 提取 JPEG 字节到 Vec<u8>
        self.mpp_encode(fd, frame.width, frame.height, quality)
    }

    fn encode_crop(
        &self,
        frame: &FrameRef,
        bbox: BoundingBox,
        padding_ratio: f32,
        quality: u8,
    ) -> Result<Vec<u8>, MediaError> {
        let src_fd = frame.handle_dmabuf_fd()?;

        // 1. 计算 crop ROI（满足 RGA NV12 对齐约束）
        //    crop_w = RGA 逻辑尺寸 = MPP 可见尺寸（偶数对齐）
        //    w_stride = RGA3 stride 对齐后的宽度（16像素对齐）
        let (sx, sy, crop_w, crop_h, w_stride) = compute_crop_roi(
            frame.width, frame.height, bbox, padding_ratio
        );

        // 2. RGA crop + pad → 输出到常驻 Scratchpad DMA-BUF
        //    wrapbuffer_fd 使用 w_stride，逻辑尺寸用 crop_w × crop_h
        let cropped_fd = self.rga_crop_and_pad(
            src_fd, frame.width, frame.height,
            sx, sy, crop_w, crop_h, w_stride
        )?;

        // 3. MPP JPEGE 编码：动态更新 prep 配置并编码
        let jpeg = self.mpp_encode_with_stride(
            cropped_fd, crop_w, crop_h, w_stride, crop_h, quality
        )?;

        // 4. Scratchpad 常驻复用，无需释放或关闭 fd（单实例串行保证独占）

        Ok(jpeg)
    }

    fn is_ready(&self) -> bool {
        self.mpp_ctx.is_valid() && self.rga_ctx.is_valid()
    }
}

impl MppSnapEncoder {
    /// RGA crop + border fill（设备侧 DMA-BUF → DMA-BUF）
    ///
    /// - crop_w, crop_h: RGA 逻辑尺寸（偶数对齐，由 compute_crop_roi 保证）
    /// - w_stride: RGA3 stride 对齐后的宽度（由 compute_crop_roi 保证）
    fn rga_crop_and_pad(
        &self,
        src_fd: RawFd,
        src_w: u32, src_h: u32,
        sx: u32, sy: u32,
        crop_w: u32, crop_h: u32,
        w_stride: u32,
    ) -> Result<RawFd, MediaError> {
        // 1. 获取常驻预分配画板的 DMA-BUF（单实例串行独占，无 CMA 碎片风险）
        let dst_fd = self.crop_scratchpad.fd();

        // 2. RGA 操作：src crop → dst scratchpad
        //    - src_info: 完整帧 (src_fd, src_w, src_h, NV12)
        //    - src_rect: 裁剪区域 (sx, sy, crop_w, crop_h) — 偶数对齐
        //    - dst_info: 输出帧 (dst_fd, crop_w, crop_h, NV12, w_stride) — stride 对齐
        //    - fill_color: 黑色填充（超出边界区域）
        rga_blit(&self.rga_ctx, src_fd, src_w, src_h, sx, sy, dst_fd, crop_w, crop_h, w_stride)?;

        // 3. RGA 使用 IM_SYNC 返回前确保 blit 完成；随后由 MPP 以设备栅障顺序消费
        //    （CPU DMA_BUF_IOCTL_SYNC 仅用于显式 readback 路径，不作为设备 fence）

        Ok(dst_fd)
    }

    /// MPP JPEG 编码（DMA-BUF 零拷贝输入）
    fn mpp_encode(
        &self,
        fd: RawFd, width: u32, height: u32, quality: u8
    ) -> Result<Vec<u8>, MediaError> {
        // 1. MppBufferInfo + mpp_buffer_import_with_tag → MppBuffer
        // 2. MppBuffer → MppFrame
        // 3. encode_put_frame → poll(output, 100ms) → encode_get_packet
        // 4. 提取 JPEG bitstream → Vec<u8>
        // ...
    }
}
```

### 4.5 Apple 实现：VtSnapEncoder

> **注**：在 macOS / iOS 上，`VTCompressionSessionCreate` 为较重系统调用（XPC + Metal 驱动初始化，耗时 5-15ms）。由于全景大图分辨率固定，全景 Session 可常驻永久复用（单次硬件编码 < 2ms）；针对变动尺寸的特写框，特写路径应采用固定尺寸画板（Letterbox）送入特写专属 Session，或针对极端异形小图走 Core Image / Metal 预处理，严禁逐帧重建 `VTCompressionSession`。

```rust
/// Apple VideoToolbox 全链路快照编码器
///
/// 全景：CVPixelBuffer → VTCompressionSession JPEG → bitstream
/// 特写：CVPixelBuffer → IOSurface crop facet → VT JPEG → bitstream
pub struct VtSnapEncoder {
    panoramic_session: VTCompressionSession,
    crop_session: VTCompressionSession,
}

impl DeviceSnapEncoder for VtSnapEncoder {
    fn name(&self) -> &'static str { "vt-snap" }

    fn encode_full_frame(&self, frame: &FrameRef, quality: u8) -> Result<Vec<u8>, MediaError> {
        let pixel_buffer = frame.handle_cvpixelbuffer()?;
        // 1. VTSessionSetProperty(Quality, quality / 100.0)
        // 2. VTCompressionSessionEncodeFrame(pixel_buffer) — 零拷贝
        // 3. 输出回调提取 CMBlockBuffer → Vec<u8>
        self.vt_encode(pixel_buffer, quality)
    }

    fn encode_crop(
        &self,
        frame: &FrameRef,
        bbox: BoundingBox,
        padding_ratio: f32,
        quality: u8,
    ) -> Result<Vec<u8>, MediaError> {
        let pixel_buffer = frame.handle_cvpixelbuffer()?;

        // 1. 计算像素坐标（macOS CVPixelBuffer 无 RGA stride 约束，只需偶数对齐）
        let (sx, sy, crop_w, crop_h, _w_stride) = compute_crop_roi(
            frame.width, frame.height, bbox, padding_ratio
        );

        // 2. IOSurface crop：零拷贝提取子区域 CVPixelBuffer
        let cropped_buffer = iosurface_crop(pixel_buffer, sx, sy, crop_w, crop_h)?;

        // 3. VTCompressionSession JPEG 编码
        self.vt_encode(cropped_buffer, quality)
    }

    fn is_ready(&self) -> bool { !self.session.is_null() }
}
```

### 4.6 Ascend 实现：DvppSnapEncoder

```rust
/// 华为 Ascend DVPP 全链路快照编码器
///
/// 全景：DeviceMemory → acldvppJpegEncode → JPEG bitstream
/// 特写：DeviceMemory → acldvppCrop → DeviceMemory → acldvppJpegEncode → bitstream
pub struct DvppSnapEncoder {
    jpeg_encoder: AcldvppJpegEncoder,
    crop_channel: AcldvppChannel,
    /// 固定 2 个 output buffer（crop_buf + jpeg_buf），单实例串行无竞争
    output_pool: DvppOutputPool,
}

impl DeviceSnapEncoder for DvppSnapEncoder {
    fn name(&self) -> &'static str { "dvpp-snap" }

    fn encode_full_frame(&self, frame: &FrameRef, quality: u8) -> Result<Vec<u8>, MediaError> {
        let (ptr, size) = frame.handle_device_memory()?;
        // 1. acldvppJpegEncodeDesc 构造
        // 2. acldvppJpegEncode() — 硬件编码
        // 3. acldvppMalloc → 输出 JPEG buffer (DVPP 专属内存，64 字节对齐)
        // 4. aclrtMemcpy(D2H) — 仅拷贝压缩后 JPEG 字节流
        self.dvpp_encode(ptr, frame.width, frame.height, quality)
    }

    fn encode_crop(
        &self,
        frame: &FrameRef,
        bbox: BoundingBox,
        padding_ratio: f32,
        quality: u8,
    ) -> Result<Vec<u8>, MediaError> {
        let (src_ptr, src_size) = frame.handle_device_memory()?;

        // 1. 计算像素坐标（DVPP 无 RGA stride 约束，只需偶数对齐）
        let (sx, sy, crop_w, crop_h, _w_stride) = compute_crop_roi(
            frame.width, frame.height, bbox, padding_ratio
        );

        // 2. acldvppCrop：DeviceMemory → DeviceMemory（零 CPU 拷贝）
        let dst_buf = self.output_pool.acquire(crop_w, crop_h)?;
        self.dvpp_crop(src_ptr, src_size, frame.width, frame.height, sx, sy, dst_buf, crop_w, crop_h)?;

        // 3. acldvppJpegEncode：裁剪后的 DeviceMemory → JPEG bitstream
        let jpeg = self.dvpp_encode(dst_buf.ptr(), crop_w, crop_h, quality)?;

        // 4. 归还输出 buffer
        self.output_pool.release(dst_buf);

        Ok(jpeg)
    }

    fn is_ready(&self) -> bool {
        self.jpeg_encoder.is_valid() && self.crop_channel.is_valid()
    }
}
```

### 4.7 CPU Fallback 实现：CpuSnapEncoder

```rust
/// CPU 兜底快照编码器
///
/// 当硬件编码器不可用时，降级为现有 CPU 路径：
/// D2H readback → CPU NV12→RGB → CPU crop → CPU JPEG
pub struct CpuSnapEncoder;

impl DeviceSnapEncoder for CpuSnapEncoder {
    fn name(&self) -> &'static str { "cpu-snap" }

    fn encode_full_frame(&self, frame: &FrameRef, quality: u8) -> Result<Vec<u8>, MediaError> {
        let rgb = crate::snapshot_readback_to_rgb_image(frame)?;
        encode_jpeg_from_rgb(&rgb, quality)
    }

    fn encode_crop(
        &self,
        frame: &FrameRef,
        bbox: BoundingBox,
        padding_ratio: f32,
        quality: u8,
    ) -> Result<Vec<u8>, MediaError> {
        let rgb = crate::snapshot_readback_to_rgb_image(frame)?;
        let cropped = crop_with_padding(&rgb, bbox, padding_ratio);
        encode_jpeg_from_rgb(&cropped, quality)
    }

    fn is_ready(&self) -> bool { true }
}
```

### 4.8 编码器选择与降级

```rust
/// 根据帧的平台句柄类型和硬件可用性，选择最优快照编码器
pub fn select_snap_encoder(frame: &FrameRef) -> Box<dyn DeviceSnapEncoder> {
    match frame.handle() {
        #[cfg(target_os = "linux")]
        FrameHandle::DmaBuf { .. } => {
            if let Some(enc) = MppSnapEncoder::try_acquire() {
                return Box::new(enc);
            }
        }
        #[cfg(target_os = "macos")]
        FrameHandle::ApplePixelBuffer { .. } => {
            if let Some(enc) = VtSnapEncoder::try_acquire() {
                return Box::new(enc);
            }
        }
        FrameHandle::DeviceMemory { .. } => {
            if let Some(enc) = DvppSnapEncoder::try_acquire() {
                return Box::new(enc);
            }
        }
        _ => {}
    }
    Box::new(CpuSnapEncoder)
}
```

### 4.9 修改 `encode_and_save_snapshot` 核心函数

```rust
/// 同步高效执行设备侧裁剪、JPEG 编码与原子文件落盘
///
/// ## 全链路硬件路径
/// 全景大图与特写裁剪均在设备侧完成：
/// - 全景：DMA-BUF → MPP/VT/DVPP JPEGE → JPEG bitstream → D2H → 写盘
/// - 特写：DMA-BUF → RGA/DVPP/IOSurface crop → DMA-BUF → JPEGE → bitstream → 写盘
///
/// ## 降级路径
/// 硬件不可用时自动降级至 CPU 路径（现有逻辑不变）。
pub(crate) fn encode_and_save_snapshot(
    camera_id: &str,
    frame: FrameRef,
    target_bbox: Option<BoundingBox>,
    base_evidence_dir: &std::path::Path,
    is_fallback: bool,
) -> Result<SnapshotResult, PipelineError> {
    // 0. 存储熔断器检查（保持不变）
    // ...

    // 1. 选择最优编码器（按平台句柄 + 硬件可用性）
    let encoder = media::encoders::select_snap_encoder(&frame);

    // 2. [snapshot_readback_path] 全景大图编码
    let quality_cfg = snapshot_config.load();
    let full_jpeg_bytes = match encoder.encode_full_frame(&frame, panoramic_q) {
        Ok(jpeg) => jpeg,
        Err(e) => {
            tracing::debug!(
                camera_id = %camera_id,
                encoder = %encoder.name(),
                error = %e,
                "硬件全景编码失败，降级至 CPU 路径"
            );
            let fallback = CpuSnapEncoder;
            fallback.encode_full_frame(&frame, panoramic_q)
                .map_err(|e| PipelineError::Snapshot(format!("全景编码失败: {e}")))?
        }
    };

    let width = frame.width;
    let height = frame.height;

    // 3. 生成图片 ID 与落盘路径（保持不变）
    let image_id = format!("img_{}_{}", now_compact_ts(), uuid::Uuid::new_v4().simple());
    let crop_image_id = format!("crop_{}_{}", now_compact_ts(), uuid::Uuid::new_v4().simple());
    // ...

    // 4. 全景 JPEG 落盘
    atomic_write_file(&full_path, &full_jpeg_bytes)
        .map_err(|e| PipelineError::Snapshot(format!("写入全景抓拍图片失败: {e}")))?;

    // 5. [snapshot_readback_path] 特写裁剪 + 编码（设备侧 crop → 设备侧 JPEG）
    let crop_jpeg_bytes = match target_bbox {
        Some(bbox) => {
            match encoder.encode_crop(&frame, bbox, quality_cfg.crop_padding_ratio, crop_q) {
                Ok(jpeg) => jpeg,
                Err(e) => {
                    tracing::debug!(
                        camera_id = %camera_id,
                        encoder = %encoder.name(),
                        error = %e,
                        "硬件特写编码失败，降级至 CPU 路径"
                    );
                    let fallback = CpuSnapEncoder;
                    fallback.encode_crop(&frame, bbox, quality_cfg.crop_padding_ratio, crop_q)
                        .map_err(|e| PipelineError::Snapshot(format!("特写编码失败: {e}")))?
                }
            }
        }
        None => full_jpeg_bytes.clone(),
    };
    atomic_write_file(&crop_path, &crop_jpeg_bytes)
        .map_err(|e| PipelineError::Snapshot(format!("写入特写抠图图片失败: {e}")))?;

    tracing::info!(
        camera_id = %camera_id,
        encoder = %encoder.name(),
        has_crop = target_bbox.is_some(),
        "靶向证据高清抓拍完成 (全链路硬件编码)"
    );

    Ok(SnapshotResult {
        image_id,
        crop_image_id,
        image_rel_path,
        crop_image_rel_path,
        file_size_bytes: full_jpeg_bytes.len(),
        width,
        height,
        is_fallback_sub_stream: is_fallback,
    })
}
```

### 4.10 设备侧 Crop ROI 计算

裁剪坐标计算仅需整数运算，不碰像素数据，可复用于所有平台：

```rust
/// 根据归一化 bbox 和 padding_ratio 计算像素坐标
///
/// 输出经过 clamp 保护，保证不超过源帧有效区域。
/// 此函数不访问像素数据，纯数学计算。
fn compute_crop_roi(
    frame_w: u32, frame_h: u32,
    bbox: BoundingBox,
    padding_ratio: f32,
) -> (u32, u32, u32, u32) {
    let w = frame_w as f32;
    let h = frame_h as f32;

    let pad_w = (bbox.x2 - bbox.x1).max(0.0) * padding_ratio;
    let pad_h = (bbox.y2 - bbox.y1).max(0.0) * padding_ratio;

    let x1 = ((bbox.x1 - pad_w).clamp(0.0, 1.0) * w).floor() as u32;
    let y1 = ((bbox.y1 - pad_h).clamp(0.0, 1.0) * h).floor() as u32;
    let x2 = ((bbox.x2 + pad_w).clamp(0.0, 1.0) * w).ceil() as u32;
    let y2 = ((bbox.y2 + pad_h).clamp(0.0, 1.0) * h).ceil() as u32;

    // 确保通用 CPU crop 的最小裁剪尺寸；RGA 目标 BSP 更严格时由硬件适配层拒绝并回退 CPU
    let crop_w = (x2 - x1).max(32);
    let crop_h = (y2 - y1).max(32);

    // 确保不超出帧边界
    let x1 = x1.min(frame_w.saturating_sub(crop_w));
    let y1 = y1.min(frame_h.saturating_sub(crop_h));

    (x1, y1, crop_w, crop_h)
}
```

---

## 5. 并发与资源管理

### 5.1 编码器实例模型：单实例串行（确定决策）

**决策：所有平台均采用单编码实例串行模型，不创建多实例并行池。**

**决策依据**：

1. **VPU 硬件是单通道**：RK3568/RK3576/RK3588 的 VPU 是单一硬件编码单元，多个 MPP_CTX_ENC 上下文串行排队等硬件，不会并行。多实例的唯一效果是增加上下文切换开销
2. **告警频率远低于 VPU 吞吐上限**：典型部署 8 路 × 1-5 次告警/秒 × 2 张/次 × 3ms/张 = 24-30ms/秒，VPU 编码负载仅 ~3%
3. **CMA 内存是硬约束**：RK3568 CMA 仅 16MB，每个 MPP 编码上下文消耗 ~2-4MB。多实例直接威胁 CMA 可用性，与项目已有约束（"禁止按摄像头重复初始化导致 CMA OOM"）冲突
4. **Apple/Ascend 同理**：VideoToolbox 单 session 编码 1080p JPEG 延迟 < 2ms，DVPP 同理。低频场景下多 session 无收益

```rust
/// 快照编码器：单实例串行模型
///
/// 硬件编码器常驻一个实例，所有告警抓拍串行排队。
/// 告警为低频事件（1-5次/秒），单实例 1-3ms/帧的吞吐绰绰有余。
/// 多实例不会提升吞吐（VPU 硬件单通道），只会消耗额外 CMA 内存。
pub struct SnapEncoder {
    /// 平台硬件编码器（常驻单实例）
    hw_encoder: Option<Box<dyn DeviceSnapEncoder>>,
    /// CPU fallback 编码器（共享）
    cpu_encoder: CpuSnapEncoder,
    /// 串行化锁：确保同一时刻只有一个编码操作在执行
    encode_lock: std::sync::Mutex<()>,
}

impl SnapEncoder {
    /// 编码全景大图（串行执行）
    pub fn encode_full_frame(&self, frame: &FrameRef, quality: u8) -> Result<Vec<u8>, MediaError> {
        let _guard = self.encode_lock.lock().map_err(|_| MediaError::Encode {
            reason: "编码器锁中毒".into(),
        })?;

        if let Some(ref enc) = self.hw_encoder {
            if enc.is_ready() {
                return enc.encode_full_frame(frame, quality);
            }
        }
        self.cpu_encoder.encode_full_frame(frame, quality)
    }

    /// 编码特写裁剪（串行执行）
    pub fn encode_crop(
        &self, frame: &FrameRef, bbox: BoundingBox,
        padding_ratio: f32, quality: u8,
    ) -> Result<Vec<u8>, MediaError> {
        let _guard = self.encode_lock.lock().map_err(|_| MediaError::Encode {
            reason: "编码器锁中毒".into(),
        })?;

        if let Some(ref enc) = self.hw_encoder {
            if enc.is_ready() {
                return enc.encode_crop(frame, bbox, padding_ratio, quality);
            }
        }
        self.cpu_encoder.encode_crop(frame, bbox, padding_ratio, quality)
    }
}
```

**容量规划**：

| 平台                  | 编码实例数 | CMA 占用  | 吞吐能力     | 告警负载 |
| ------------------- | ----- | ------- | -------- | ---- |
| RK3568              | **1** | ~2-4MB  | ~330 帧/秒 | ~3%  |
| RK3588              | **1** | ~2-4MB  | ~500 帧/秒 | ~2%  |
| macOS Apple Silicon | **1** | N/A     | ~500 帧/秒 | ~2%  |
| Ascend 910B         | **1** | DVPP 通道 | ~200 帧/秒 | ~5%  |
| x86 (CPU fallback)  | **1** | N/A     | ~100 帧/秒 | ~10% |

### 5.2 线程模型

```
  └── dedicated snapshot OS worker（固定容量队列，线程内常驻编码器）
        │
        └── encode_and_save_snapshot()
              │
              ├── [HW 全景] encoder.encode_full_frame()
              │     └── MPP/RGA/DVPP FFI 同步调用（单实例串行）
              │
              ├── [HW 特写] encoder.encode_crop()
              │     ├── RGA/DVPP/IOSurface crop
              │     └── MPP/VT/DVPP JPEG 编码
              │
              └── [CPU fallback] 单次 snapshot_readback_to_rgb_image + encode_jpeg
                    └── D2H + CPU 像素操作
```

编码调用在固定容量的专用 snapshot OS Worker 中执行，不阻塞 Tokio Worker；队列满时立即返回过载错误。

### 5.3 RGA 输出画板管理（Scratchpad 模式）

**设计演进**：放弃动态 Hash Key 的 `DmaBufPool`，改用**单块最大预分配 Scratchpad DMA-BUF**。

**决策原因**：

- 检测目标的 BoundingBox 尺寸离散度极大，若按 `(w, h)` 作为 Key 进行池化，命中率极低，会导致底层不断触发 DRM/dma-heap 分配
- RK3568 等嵌入式平台 CMA 区域通常仅 16MB ~ 64MB，频繁动态分配释放不同尺寸的物理连续内存，会在数小时内引发严重的 **CMA 碎片化**，最终触发 `page allocation failure`
- 在单一专用 Worker 串行执行，固定分配一个最大 4K 的 Scratchpad；RGA 仅写入有效区域并指定逻辑宽高，DMA32 heap 优先用于兼容 RK3568 RGA2。

```rust
/// RGA 输出单画板缓冲（固定最大容量，杜绝 CMA 碎片）
pub struct DmaBufScratchpad {
    fd: RawFd,
    max_width: u32,
    max_height: u32,
    stride: u32,
    size_bytes: usize,
}

impl DmaBufScratchpad {
    /// 启动时预分配一次最大 4096x2160 NV12 DMA-BUF，优先使用 system-dma32 heap
    pub fn new(max_width: u32, max_height: u32) -> Result<Self, MediaError> {
        let stride = (max_width + 15) & !15;
        let size = (stride * max_height * 3 / 2) as usize;
        let fd = allocate_dmabuf_dma_heap(size)?;
        Ok(Self { fd, max_width, max_height, stride, size_bytes: size })
    }

    pub fn fd(&self) -> RawFd { self.fd }
    pub fn stride(&self) -> u32 { self.stride }
}

impl Drop for DmaBufScratchpad {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd); }
    }
}
```

### 5.4 停机与清理

- 编码器上下文在 OS Worker 停机时按 RAII 析构顺序释放
- MPP 编码上下文：`mpp_destroy(ctx)` + 释放输出 packet 池
- RGA 上下文：释放输出 DMA-BUF 池（`close(fd)` 所有缓存的 buffer）
- VideoToolbox session：`VTCompressionSessionInvalidate` + `CFRelease`
- DVPP 编码器：`acldvppJpegEncodeDeinit` + 释放 DVPP 内存池

---

## 6. 测试策略

### 6.1 单元测试

```rust
// crates/media/src/encoders/tests.rs

#[test]
fn test_compute_crop_roi_basic() {
    // 验证归一化 bbox → 像素坐标的正确性
    let (x, y, w, h, stride) = compute_crop_roi(1920, 1080, BoundingBox::new(0.3, 0.3, 0.7, 0.7), 0.1);
    assert!(w > 0 && h > 0);
    assert!(x + w <= 1920);
    assert!(y + h <= 1080);
    // NV12 约束：宽高必须偶数
    assert_eq!(w % 2, 0, "crop_w 必须偶数");
    assert_eq!(h % 2, 0, "crop_h 必须偶数");
    assert_eq!(x % 2, 0, "x 必须偶数");
    assert_eq!(y % 2, 0, "y 必须偶数");
    // RGA3 stride 对齐：w_stride 必须16像素对齐
    assert_eq!(stride % 16, 0, "w_stride 必须16像素对齐");
    assert!(stride >= w, "w_stride >= crop_w");
}

#[test]
fn test_compute_crop_roi_edge_cases() {
    // 边界 bbox (0.0, 0.0, 1.0, 1.0) + padding → 不超出帧
    let (x, y, w, h, stride) = compute_crop_roi(1920, 1080, BoundingBox::new(0.0, 0.0, 1.0, 1.0), 0.1);
    assert_eq!(x, 0);
    assert_eq!(y, 0);
    assert!(w <= 1920);
    assert!(h <= 1080);
    assert_eq!(w % 2, 0);
    assert_eq!(stride % 16, 0);
}

#[test]
fn test_compute_crop_roi_odd_dimensions() {
    // 验证奇数尺寸被正确对齐为偶数
    // 501×301 → 500×300, x=101→100, y=51→50
    let (x, y, w, h, stride) = compute_crop_roi(
        1920, 1080, BoundingBox::new(0.05, 0.05, 0.31, 0.33), 0.0
    );
    assert_eq!(w % 2, 0, "奇数宽度必须被截断为偶数");
    assert_eq!(h % 2, 0, "奇数高度必须被截断为偶数");
    assert_eq!(x % 2, 0, "x 必须偶数对齐");
    assert_eq!(y % 2, 0, "y 必须偶数对齐");
    assert_eq!(stride % 16, 0, "w_stride 必须16像素对齐");
}

#[test]
fn test_cpu_snap_encoder_full_and_crop() {
    // 构造 Host 内存帧 → CPU 全景 + 特写 → 验证 JPEG 输出合法
}

#[test]
fn test_encoder_fallback_chain() {
    // 模拟硬件不可用 → 验证自动降级到 CPU
}

#[test]
fn test_jpeg_quality_range() {
    // 验证 quality 1-100 均产生合法 JPEG
}

#[test]
fn test_empty_frame_rejected() {
    // 验证 0×0 帧被拒绝
}
```

### 6.2 集成测试

```rust
// crates/pipeline/tests/snapshot_hw_jpeg_tests.rs

#[tokio::test]
#[ignore]  // 需要真实硬件
async fn test_snapshot_hw_full_and_crop() {
    // 1. 构造模拟 DMA-BUF/CVPixelBuffer 帧
    // 2. 调用 capture_snapshot（带 bbox）
    // 3. 验证全景 JPEG 文件落盘
    // 4. 验证特写 JPEG 文件落盘
    // 5. 验证两张图均可被 image::open 解码
    // 6. 验证特写裁剪区域在 bbox 范围内
    // 7. 验证全景图尺寸 == 帧原始尺寸
}

#[tokio::test]
async fn test_snapshot_no_bbox_skips_crop() {
    // 无 bbox 时特写 = 全景（不再次编码）
}

#[tokio::test]
async fn test_snapshot_cpu_fallback_when_hw_unavailable() {
    // 构造 Host 内存帧 → 验证走 CPU 路径并成功
}

#[tokio::test]
async fn test_snapshot_hw_crop_fallback_to_cpu() {
    // 模拟硬件 crop 失败 → 验证降级到 CPU crop + HW/JPEG
}
```

### 6.3 真机验证矩阵

| 平台                  | 全景 HW      | 特写 HW crop          | 1080p 全景延迟 | 1080p 特写延迟 |
| ------------------- | ---------- | ------------------- | ---------- | ---------- |
| RK3568              | MPP VPU    | RGA → MPP           | < 3ms      | < 5ms      |
| RK3588              | MPP VPU    | RGA → MPP           | < 2ms      | < 4ms      |
| macOS Apple Silicon | VT JPEG    | IOSurface crop → VT | < 2ms      | < 3ms      |
| Ascend 910B         | DVPP JPEGE | DVPP crop → DVPP    | < 5ms      | < 7ms      |
| x86 开发机             | CPU only   | CPU only            | 8-15ms     | 10-18ms    |

---

## 7. 迁移与兼容性

### 7.1 API 兼容

- `SnapshotResult` 结构体不变
- `capture_snapshot` / `save_snapshot_async` 签名不变
- 上层 `PipelineManager` 和告警生成逻辑无感知

### 7.2 Nuwa 规范更新

| 文件                          | 更新内容                                                                      |
| --------------------------- | ------------------------------------------------------------------------- |
| `media-pipeline.md`         | 三路径表格中 `snapshot_readback_path` 更新为"设备 crop + 设备 JPEG 编码 → 仅压缩后 D2H → 写盘" |
| `api-guidelines.md`         | 新增 `GET/PUT /api/v1/system/snapshot/config` 端点                            |
| `concurrency-guidelines.md` | 新增编码器 Worker 归属说明                                                         |
| `ffi-guidelines.md`         | 新增 MPP/RGA/VT/DVPP JPEG 编码与 crop FFI 边界约束                                 |

### 7.3 Feature Flag

```toml
# crates/media/Cargo.toml
[features]
default = ["backend-cpu"]

# 硬件快照编码（crop + JPEG 全链路）
hw-snap-mpp = ["mpp", "rga"]     # Rockchip: RGA crop + MPP JPEGE
hw-snap-vt = []                    # Apple: IOSurface crop + VT JPEG
hw-snap-dvpp = ["dvpp"]           # Ascend: DVPP crop + DVPP JPEGE
```

编译时按平台启用对应 feature，运行时通过 `is_ready()` 进一步判断硬件可用性。

---

## 8. 风险与缓解

| 风险                           | 影响                    | 缓解措施                                                          |
| ---------------------------- | --------------------- | ------------------------------------------------------------- |
| MPP 编码与解码共享 VPU              | 高帧率解码期间编码需排队          | VPU 编解码可在不同上下文并行；告警低频（~1-5次/秒），单实例串行 1-3ms/帧，排队延迟可忽略          |
| RGA 输出动态分配引发 CMA 碎片          | 长时间运行后 CMA 耗尽致 Crash  | 采用固定最大容量的 `DmaBufScratchpad` 单画板模式，启动常驻，不随尺寸动态分配              |
| RGA → MPP DMA-BUF 跨 IP 缓存不一致 | 抓拍图片局部花屏、斑马条纹或绿屏      | RGA 写入后显式执行 `DMA_BUF_IOCTL_SYNC` 发起硬件 Cache Flush             |
| 色彩空间未做 Full Range 重映射        | 抓拍图片发灰泛白，对比度降低        | MPP/DVPP 编码配置中显式指定 `MPP_FRAME_RANGE_JPEG` / BT.601 Full Range |
| VideoToolbox session 频繁创建    | macOS 特写尺寸动态变化导致编码变慢  | 全景 Session 常驻复用；特写采用固定尺寸 Letterbox 或 Metal 预处理，杜绝逐帧 Create    |
| DVPP crop 内存对齐不满足            | 输入帧不满足 16×2 对齐        | 编码前检查并拒绝不合规帧；fallback 到 CPU                                   |
| JPEG 编码器上下文内存泄漏              | 长时间运行后内存增长            | RAII 管理 + 停机时强制释放 + 生命周期测试                                    |
| 特写裁剪尺寸低于硬件编码下限               | 极小 bbox 导致 VPU 驱动报错拒编 | `compute_crop_roi` 强制增加 $\ge 32\times 32$ 偶数对齐钳位与平移保护         |

---

## 9. 实现优先级

| 阶段     | 内容                                                                                                                                 | 预期收益                    |
| ------ | ---------------------------------------------------------------------------------------------------------------------------------- | ----------------------- |
| **P0** | `DeviceSnapEncoder` trait + `CpuSnapEncoder` + `SnapEncoder`(单实例串行) + `compute_crop_roi` + CPU fallback 重构 + `SnapshotConfig` 质量字段 | 架构就绪，CPU 路径行为不变         |
| **P1** | **Rockchip `MppSnapEncoder`**：MPP JPEGE + RGA crop 串联 + RGA 输出 DMA-BUF 池                                                           | **全景 + 特写全链路硬件化（主力平台）** |
| **P2** | **Apple `VtSnapEncoder`**：VT JPEG + IOSurface crop                                                                                 | macOS/iOS 全链路硬件化        |
| **P3** | **Ascend `DvppSnapEncoder`**：DVPP crop + DVPP JPEGE                                                                                | 昇腾全链路硬件化                |
| **P4** | `GET/PUT /api/v1/system/snapshot/config` + `StorageSettings.tsx` 新增图片编码 Section                                                    | 用户可配置                   |

---

## 10. JPEG 质量配置

### 10.1 设计决策

JPEG 质量参数**必须暴露为可配置项**。原因：

- **存储预算**：边缘设备存储空间有限，质量从 85 降到 75 可减少 ~30-40% 文件体积，直接影响告警保留天数
- **带宽约束**：远程抓拍图片通过 HTTP API 传输时，质量影响响应延迟
- **场景差异**：园区周界入侵需要高清特写（q90+），仓库烟感报警只需可辨识画面（q75 即可）
- **工业惯例**：Hikvision/Dahua 等 NVR 均提供 1-100 质量滑块，这是用户期望的基础能力

### 10.2 配置结构

在现有 `SnapshotConfig` 中新增质量参数，复用 `SystemConfigRepo` 持久化 + API 热更新路径：

```rust
/// 证据快照引擎配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotConfig {
    // ── 原有字段 ──
    /// 极速单帧与精准追帧模式的时间戳相位差阈值（毫秒）
    pub phase_diff_threshold_ms: i64,
    /// 前向追帧硬解的最大允许包数量
    pub max_burst_packets: usize,
    /// 前向追帧硬解的最大允许耗时预算（毫秒）
    pub max_burst_timeout_ms: u64,
    /// 抓拍策略模式
    pub capture_mode: SnapshotCaptureMode,

    // ── 新增：JPEG 编码质量（按码流类型区分） ──
    /// 主码流全景 JPEG 质量 (1-100)，默认 90（高清取证）
    pub main_stream_panoramic_quality: u8,
    /// 主码流特写 JPEG 质量 (1-100)，默认 95（高清取证）
    pub main_stream_crop_quality: u8,
    /// 子码流全景 JPEG 质量 (1-100)，默认 80（低功耗场景）
    pub sub_stream_panoramic_quality: u8,
    /// 子码流特写 JPEG 质量 (1-100)，默认 85（低功耗场景）
    pub sub_stream_crop_quality: u8,
    /// 设备侧裁剪边界扩展比例，默认 0.1 (10%)
    pub crop_padding_ratio: f32,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            phase_diff_threshold_ms: 500,
            max_burst_packets: 30,
            max_burst_timeout_ms: 80,
            capture_mode: SnapshotCaptureMode::AdaptiveDualMode,
            main_stream_panoramic_quality: 90,
            main_stream_crop_quality: 95,
            sub_stream_panoramic_quality: 80,
            sub_stream_crop_quality: 85,
            crop_padding_ratio: 0.1,
        }
    }
}

impl SnapshotConfig {
    pub fn validate(&self) -> Result<(), String> {
        for (name, val) in [
            ("mainStreamPanoramicQuality", self.main_stream_panoramic_quality),
            ("mainStreamCropQuality", self.main_stream_crop_quality),
            ("subStreamPanoramicQuality", self.sub_stream_panoramic_quality),
            ("subStreamCropQuality", self.sub_stream_crop_quality),
        ] {
            if val == 0 || val > 100 {
                return Err(format!("{name} must be 1-100"));
            }
        }
        if !(0.0..=0.5).contains(&self.crop_padding_ratio) {
            return Err("cropPaddingRatio must be 0.0-0.5".into());
        }
        Ok(())
    }

    /// 根据码流类型选择质量参数
    pub fn quality_for_stream(&self, stream_type: StreamType) -> (u8, u8) {
        match stream_type {
            StreamType::Main => (
                self.main_stream_panoramic_quality,
                self.main_stream_crop_quality,
            ),
            StreamType::Sub => (
                self.sub_stream_panoramic_quality,
                self.sub_stream_crop_quality,
            ),
        }
    }
}
```

### 10.3 数据流：配置从持久化到编码器

```
前端设置页 (React)
  │
  PUT /api/v1/system/snapshot/config
  │
  └── api::routes::system::snapshot::update_snapshot_config()
        │
        ├── SnapshotConfig::validate()         // 校验参数范围
        ├── SystemConfigRepo::set("snapshot_config", json)  // SQLite 持久化
        └── SnapshotEngine::update_config(new_config)        // 运行时热更新
              │
              └── AtomicSnapshotConfig  // seqlock 无锁读，整份配置一致替换
                    │
                    └── encode_and_save_snapshot()
                          │
                          ├── (panoramic_q, crop_q) = config.quality_for_stream(stream_type)
                          ├── encoder.encode_full_frame(&frame, panoramic_q)
                          └── encoder.encode_crop(&frame, bbox, config.crop_padding_ratio, crop_q)
```

### 10.4 API 端点

复用现有 `SystemConfigRepo` + `routes/system/` 模式：

| 方法  | 端点                               | 说明          |
| --- | -------------------------------- | ----------- |
| GET | `/api/v1/system/snapshot/config` | 获取当前快照配置    |
| PUT | `/api/v1/system/snapshot/config` | 更新快照配置（热生效） |

```rust
// crates/api/src/routes/system/snapshot.rs

/// `GET /api/v1/system/snapshot/config`
pub async fn get_snapshot_config(
    State(state): State<AppState>,
) -> Result<ApiResponse<SnapshotConfig>, ApiError> {
    let config = load_snapshot_config_from_db(&state).await?;
    Ok(ApiResponse::success(config))
}

/// `PUT /api/v1/system/snapshot/config`
pub async fn update_snapshot_config(
    State(state): State<AppState>,
    Json(new_config): Json<SnapshotConfig>,
) -> Result<ApiResponse<SnapshotConfig>, ApiError> {
    new_config.validate().map_err(|e| ApiError::BadRequest(e))?;

    let json = serde_json::to_string(&new_config)
        .map_err(|e| ApiError::Internal(format!("序列化配置失败: {e}")))?;
    db::SystemConfigRepo::set(&state.db, "snapshot_config", &json)
        .await
        .map_err(|e| ApiError::Internal(format!("保存配置失败: {e}")))?;

    // 运行时热更新：通知 SnapshotEngine 切换配置
    state.pipeline_manager.update_snapshot_config(new_config.clone()).await;

    Ok(ApiResponse::success(new_config))
}
```

### 10.5 前端配置 UI

图片编码配置**合并到存储设置页面**（`StorageSettings.tsx`），不创建独立 Tab。

**设计依据**：

- 工业惯例：Hikvision iVMS-4200、DaHua SmartPSS、Blue Iris 等 NVR/VMS 均将图像质量放在存储/录像设置中
- 用户心理模型："磁盘快满了 → 降低图片质量以延长保留天数" 是单一决策链，不应拆分到两个页面
- 当前 `StorageSettings` 已有保留天数、配额、覆盖策略，JPEG 质量是"每张图多大"的直接控制参数，与保留策略天然联动

**页面布局**：在现有 `StorageSettings` 的保留策略下方，新增「图片编码」`SettingsSection`

```
StorageSettings 页面布局：
┌─────────────────────────────────────────────────────┐
│  磁盘状态                           [刷新]           │
│  [进度条] [总容量] [已用] [可用] [健康等级]           │
│  [告警图] [识别图] [抓拍图]                           │
├─────────────────────────────────────────────────────┤
│  保留策略                             [手动清理]      │
│  [告警图   30天  不限配额]                            │
│  [识别图   14天  不限配额]                            │
│  [抓拍图    7天  不限配额]                            │
│  覆盖策略: ○循环覆盖 ○写满停止                        │
│  自动清理: [====]                                     │
│  ▸ 高级设置（水位阈值、批删数量）                      │
├─────────────────────────────────────────────────────┤
│  图片编码 ← 新增 Section                              │
│                                                      │
│  全景图片质量   [═══════════●══]  85   ~200KB/张      │
│  (越高质量越好，文件越大)                              │
│                                                      │
│  特写裁剪质量   [═════════════●]  90   ~280KB/张      │
│  (特写通常需要更高清晰度)                              │
│                                                      │
│  裁剪边界扩展   [═══●══════════]  10%                 │
│  (避免目标贴边裁切)                                   │
│                                                      │
│  编码器状态:  MPP VPU ✓   RGA ✓   CPU fallback ○     │
│                                                      │
│                              [保存]  [恢复默认]       │
└─────────────────────────────────────────────────────┘
```

质量滑块范围 1-100，步进 1，**实时预览文件大小估算**（根据当前分辨率和质量计算）：

- q75: ~120KB/张 (1080p)
- q85: ~200KB/张 (1080p，推荐默认)
- q95: ~350KB/张 (1080p)

**关键交互**：调整质量滑块时，实时显示预估单张文件大小和 30 天总存储占用，让用户直观看到质量与存储的 trade-off。

### 10.6 质量与存储的量化关系

1080p 全景大图在不同质量下的文件体积（RK3568 实测基线）：

| 质量           | 文件体积       | 相对 q85 | 30 天 × 100 路 × 5 次/天 |
| ------------ | ---------- | ------ | -------------------- |
| q75          | ~120KB     | -40%   | ~54GB                |
| q80          | ~160KB     | -20%   | ~72GB                |
| **q85 (默认)** | **~200KB** | **基准** | **~90GB**            |
| q90          | ~280KB     | +40%   | ~126GB               |
| q95          | ~350KB     | +75%   | ~158GB               |

边缘设备典型存储预算 256GB-1TB，q85 全景 + q90 特写的默认配置在多数场景下经济合理。用户可根据存储水位灵活调整。

### 10.7 配置变更的运行时生效机制

```rust
// PipelineManager 新增方法
impl PipelineManager {
    pub fn update_snapshot_config(&self, config: SnapshotConfig) -> Result<(), String> {
        // SnapshotEngine 内部使用 seqlock 保证无锁读取与整份配置一致替换
        self.snapshot_engine.update_config(config)
    }
}

// SnapshotEngine 热更新
impl SnapshotEngine {
    pub fn update_config(&self, config: SnapshotConfig) -> Result<(), String> {
        config.validate()?;
        self.config.store(config);
        Ok(())
    }
}
```

配置变更不需要重启服务，**下次抓拍自动使用新参数**。当前正在执行的抓拍使用入队时复制的完整配置快照，避免中途变更导致数据不一致。

---

## 11. DVPP 输出 Buffer 池设计

### 11.1 Buffer 数量：固定 2 个（确定决策）

单实例串行模型下，buffer 数量由管线时序决定，与并发告警数无关：

| Buffer     | 用途                       | 生命周期                          |
| ---------- | ------------------------ | ----------------------------- |
| `crop_buf` | `acldvppCrop` 裁剪输出       | crop 完成 → 送入 JPEGE → 编码开始后可释放 |
| `jpeg_buf` | `acldvppJpegEncode` 编码输出 | 编码完成 → D2H 拷贝 → 写盘后可释放        |

**全景→特写串行执行时的 buffer 复用**：全景 JPEG 输出 buffer 在 D2H 完成后立即释放，被特写 crop 复用。峰值持有量始终为 2。

### 11.2 Buffer 尺寸预算

DVPP 输出 buffer 必须通过 `acldvppMalloc` 分配（64 字节对齐）：

| 分辨率               | crop_buf (NV12) | jpeg_buf (压缩后 10%) | 合计         | 占 CMA 比例 (16MB) |
| ----------------- | --------------- | ------------------ | ---------- | --------------- |
| 1080p (1920×1088) | 3.1MB           | ~300KB             | **3.4MB**  | 21%             |
| 2K (2560×1440)    | 5.5MB           | ~550KB             | **6.1MB**  | 38%             |
| 4K (3840×2160)    | 12.4MB          | ~1.2MB             | **13.6MB** | 85%             |

4K 场景接近 CMA 上限，初始化时需检查剩余 CMA，不足则降级到 CPU 路径。

### 11.3 实现

```rust
/// DVPP 输出 buffer 池（单实例串行，固定 2 个 buffer）
pub struct DvppOutputPool {
    crop_buf: Option<DvppBuffer>,   // acldvppCrop 输出
    jpeg_buf: Option<DvppBuffer>,   // acldvppJpegEncode 输出
}

impl DvppOutputPool {
    pub fn new(max_width: u32, max_height: u32) -> Result<Self, MediaError> {
        let nv12_size = (max_width as usize) * ((max_height + 1) & !1) as usize * 3 / 2;
        Ok(Self {
            crop_buf: Some(DvppBuffer::alloc(nv12_size)?),
            jpeg_buf: Some(DvppBuffer::alloc(nv12_size / 10)?),
        })
    }
}

impl Drop for DvppOutputPool {
    fn drop(&mut self) {
        // RAII：acldvppFree 释放所有预分配 buffer
        self.crop_buf.take().map(|b| b.free());
        self.jpeg_buf.take().map(|b| b.free());
    }
}
```

---

## 12. RGA Crop 对齐策略

### 12.1 RGA 硬件对齐约束（权威来源）

来源：rknn-pro skill `rga-api-reference.md` Alignment Rules 节，RK3576/RK3588 生产环境已验证。

**NV12 stride 对齐**：

| Core family   | Byte stride align | NV12 w_stride 对齐 |
| ------------- | ----------------- | ---------------- |
| RGA2 (RK3568) | 4 bytes           | **4 px**         |
| RGA3 (RK3588) | 16 bytes          | **16 px**        |

**NV12 逻辑尺寸约束**（原文）：

> For raster NV12/NV21, **x/y offsets, logical width/height, and height stride must also be even**. An odd logical width such as 1281 remains invalid even if the backing stride is rounded up.

**已验证的生产 bug 模式**：

> NV12 odd logical height → `imcheck` "Error yuv not align to 2". A 600×423 NV12 source with correctly aligned `wstride=608`/`hstride=424` still failed because the *logical* rect height 423 was odd.

**结论：约束分两层，作用在不同对象上**：

| 约束            | 作用对象                                    | 要求                    | 影响                     |
| ------------- | --------------------------------------- | --------------------- | ---------------------- |
| **逻辑尺寸偶数**    | src_rect/dst_rect 的 x, y, width, height | **RGA2 + RGA3 均强制**   | crop 宽高必须偶数，x/y 坐标必须偶数 |
| **stride 对齐** | buffer 的 w_stride                       | RGA2: 4px; RGA3: 16px | DMA-BUF 分配容量           |

### 12.2 crop 坐标的完整对齐流程

假设裁剪 501×301 区域（bbox 计算出的奇数尺寸）：

```
1. compute_crop_roi 计算（任意精度）：
   sx=101, sy=51, crop_w=501, crop_h=301

2. NV12 逻辑尺寸偶数约束（RGA2+RGA3 均强制）：
   crop_w = 501 & !1 = 500    ← 强制偶数
   crop_h = 301 & !1 = 300    ← 强制偶数
   sx = 101 & !1 = 100        ← x 必须偶数（NV12 色度平面 2×2 采样）
   sy = 51 & !1 = 50          ← y 必须偶数

3. RGA3 stride 对齐（仅影响 buffer 分配，不影响逻辑尺寸）：
   rga_w_stride = (500 + 15) & !15 = 512  ← w_stride 对齐
   rga_h_stride = (300 + 1) & !1 = 300    ← h_stride 对齐

4. RGA 实际处理：
   src_rect: (100, 50, 500, 300)   ← 逻辑尺寸（偶数，合法）
   dst_rect: (0, 0, 500, 300)      ← 逻辑尺寸（偶数，合法）
   dst w_stride: 512                ← buffer stride（16对齐，合法）

5. MPP 帧描述符：
   width = 500, height = 300           ← 可见尺寸（= RGA 逻辑尺寸）
   hor_stride = 512, ver_stride = 300  ← RGA 输出的 stride

6. MPP JPEG 编码器：
   编码 [0..500] × [0..300] 的全部可见像素（无 padding 列参与编码）
```

**关键区分**：

| 层面               | 值       | 对齐要求                  | 由谁决定         |
| ---------------- | ------- | --------------------- | ------------ |
| **RGA 逻辑宽度**     | 500     | **必须偶数**              | NV12 格式约束    |
| **RGA 逻辑高度**     | 300     | **必须偶数**              | NV12 格式约束    |
| **RGA x/y 坐标**   | 100, 50 | **必须偶数**              | NV12 色度平面约束  |
| **RGA w_stride** | 512     | RGA2: 4px; RGA3: 16px | 硬件 stride 约束 |
| **MPP 可见尺寸**     | 500×300 | **不需要额外对齐**           | = RGA 逻辑尺寸   |
| **MPP stride**   | 512     | 跟随 RGA 输出             |              |

### 12.3 compute_crop_roi 实现

```rust
/// 计算 crop ROI，满足 RGA NV12 对齐约束
///
/// 返回 (x, y, crop_w, crop_h, w_stride)
/// - x, y: 偶数对齐的裁剪起点
/// - crop_w, crop_h: 偶数对齐的逻辑裁剪尺寸（RGA 逻辑尺寸 = MPP 可见尺寸）
/// - w_stride: RGA3 stride 对齐后的宽度（≥ crop_w，供 buffer 分配和 wrapbuffer 使用）
fn compute_crop_roi(
    frame_w: u32, frame_h: u32,
    bbox: BoundingBox,
    padding_ratio: f32,
) -> (u32, u32, u32, u32, u32) {
    // 1. 归一化 bbox → 像素坐标（任意精度）
    let (x1_f, y1_f, x2_f, y2_f) = bbox_to_pixels_f32(frame_w, frame_h, bbox, padding_ratio);

    // 2. clamp 到帧边界
    let x1 = (x1_f.max(0.0) as u32).min(frame_w.saturating_sub(2));
    let y1 = (y1_f.max(0.0) as u32).min(frame_h.saturating_sub(2));
    let x2 = (x2_f.ceil() as u32).min(frame_w);
    let y2 = (y2_f.ceil() as u32).min(frame_h);

    // 3. NV12 逻辑尺寸必须偶数（RGA2+RGA3 均强制）
    //    x/y 坐标也必须偶数（NV12 色度平面 2×2 采样）
    let mut x1 = x1 & !1;
    let mut y1 = y1 & !1;
    let mut crop_w = ((x2 - x1) & !1).max(2);
    let mut crop_h = ((y2 - y1) & !1).max(2);

    // 4. 通用 CPU/几何路径的最小裁剪尺寸为 32x32；目标 RGA BSP 的安全公共下限为 68x68。
    // RGA 适配层对低于 68x68 的 ROI 返回错误，由上层复用一次 CPU readback 完成特写编码。
    const MIN_CPU_DIM: u32 = 32;
    if crop_w < MIN_CPU_DIM || crop_h < MIN_CPU_DIM {
        crop_w = crop_w.max(MIN_CPU_DIM);
        crop_h = crop_h.max(MIN_CPU_DIM);
    }

    // 5. 确保不超出帧边界（用偶数与下限钳位后的尺寸检查，超出向左上平移）
    if x1 + crop_w > frame_w {
        x1 = frame_w.saturating_sub(crop_w) & !1;
    }
    if y1 + crop_h > frame_h {
        y1 = frame_h.saturating_sub(crop_h) & !1;
    }

    // 6. RGA3 w_stride 对齐（仅影响 buffer 分配，不影响逻辑尺寸）
    //    RGA2: 4px align; RGA3: 16px align
    //    用16对齐兼容两种核心（16 是 4 的超集）
    let w_stride = (crop_w + 15) & !15;

    (x1, y1, crop_w, crop_h, w_stride)
}
```

### 12.4 调用方使用

```rust
let (sx, sy, crop_w, crop_h, w_stride) = compute_crop_roi(...);

// RGA: wrapbuffer_fd 使用 w_stride（对齐后的 stride）
let dst = wrapbuffer_fd(dst_fd, crop_w, crop_h, RK_FORMAT_YCbCr_420_SP, w_stride, crop_h);
rga_blit(src, sx, sy, crop_w, crop_h, dst, 0, 0, crop_w, crop_h);

// MPP: 帧描述符使用 crop_w（= RGA 逻辑尺寸，偶数，合法）
mpp_frame_set_width(frame, crop_w);
mpp_frame_set_height(frame, crop_h);
mpp_frame_set_hor_stride(frame, w_stride);
mpp_frame_set_ver_stride(frame, crop_h);
```

**crop_w 既是 RGA 逻辑尺寸，也是 MPP 可见尺寸**——不再需要区分两个宽度。w_stride 仅用于 buffer 分配和 `wrapbuffer_fd` 参数。

### 12.5 imcheck 验证（开发阶段必做）

```rust
// 开发/调试阶段运行 imcheck 验证对齐合法性
let ret = imcheck(src, dst, src_rect, dst_rect, 0);
if ret != IM_STATUS_SUCCESS {
    tracing::error!("RGA imcheck 失败: {}", imStrError(ret));
    // 降级到 CPU 路径
}
```

生产环境中 `imcheck` 可选开启（有微量性能开销），但建议在首次部署时启用以验证对齐参数正确性。

---

## 13. Feature Flag 策略

### 13.1 决策：按平台独立控制

**`hw-snap-mpp` / `hw-snap-vt` / `hw-snap-dvpp` 独立 feature，不合并为 `hw-snap`。**

原因：

1. **依赖关系不同**：`hw-snap-mpp` 依赖 `mpp` + `rga`，`hw-snap-dvpp` 依赖 `dvpp`，`hw-snap-vt` 无额外依赖。合并后在非目标平台上会引入无用的 FFI 声明
2. **项目惯例**：`mpp`、`dvpp`、`rga` 已经是独立 feature，新增 feature 保持一致模式
3. **CI 可控**：每个 feature 是独立编译目标，互不干扰，不会增加矩阵复杂度

### 13.2 Feature 定义

```toml
# crates/media/Cargo.toml
[features]
# 已有
mpp = []
dvpp = []
rga = []

# 新增：硬件快照编码（各自依赖对应平台 feature）
hw-snap-mpp = ["mpp", "rga"]     # Rockchip: RGA crop + MPP JPEGE
hw-snap-vt = []                    # Apple: IOSurface crop + VT JPEG (macOS only)
hw-snap-dvpp = ["dvpp"]           # Ascend: DVPP crop + DVPP JPEGE
```

### 13.3 CI 矩阵

| CI job               | feature        | 平台            | 编码器               |
| -------------------- | -------------- | ------------- | ----------------- |
| `build-linux-rknn`   | `hw-snap-mpp`  | Linux aarch64 | MPP + RGA         |
| `build-macos-arm64`  | `hw-snap-vt`   | macOS aarch64 | VideoToolbox      |
| `build-linux-ascend` | `hw-snap-dvpp` | Linux x86_64  | DVPP              |
| `build-x86-dev`      | (none)         | Linux x86_64  | CPU fallback only |

### 13.4 条件编译模式

```rust
// crates/media/src/encoders/mod.rs

pub fn select_snap_encoder(frame: &FrameRef) -> Box<dyn DeviceSnapEncoder> {
    match frame.handle() {
        #[cfg(all(target_os = "linux", feature = "hw-snap-mpp"))]
        FrameHandle::DmaBuf { .. } => {
            if let Some(enc) = MppSnapEncoder::try_acquire() {
                return Box::new(enc);
            }
        }
        #[cfg(all(target_os = "macos", feature = "hw-snap-vt"))]
        FrameHandle::ApplePixelBuffer { .. } => {
            if let Some(enc) = VtSnapEncoder::try_acquire() {
                return Box::new(enc);
            }
        }
        #[cfg(all(target_os = "linux", feature = "hw-snap-dvpp"))]
        FrameHandle::DeviceMemory { .. } => {
            if let Some(enc) = DvppSnapEncoder::try_acquire() {
                return Box::new(enc);
            }
        }
        _ => {}
    }
    Box::new(CpuSnapEncoder)
}
```

---

## 14. Per-Stream-Type 质量配置

### 14.1 决策：per-stream-type，不 per-camera

**按主/子码流类型区分质量，不按摄像头。**

原因：

- Heimdall 已有主/子双流架构（`main` / `sub` / `auto` 模式），质量差异的根源是**码流类型不同**，不是摄像头不同
- Per-camera 质量配置 = 8 路 x 手动调参 = 运维噩梦，用户不会逐路调质量
- 主码流抓拍用于高清取证（人脸/车牌），需要高清晰度；子码流用于场景回溯，低质量即可

### 14.2 配置结构

```rust
pub struct SnapshotConfig {
    // ... 已有字段 ...

    /// 主码流全景 JPEG 质量（高清取证，默认 90）
    pub main_stream_panoramic_quality: u8,
    /// 主码流特写 JPEG 质量（高清取证，默认 95）
    pub main_stream_crop_quality: u8,
    /// 子码流全景 JPEG 质量（低功耗场景，默认 80）
    pub sub_stream_panoramic_quality: u8,
    /// 子码流特写 JPEG 质量（低功耗场景，默认 85）
    pub sub_stream_crop_quality: u8,
}
```

**默认值策略**：

| 码流  | 全景质量 | 特写质量 | 理由             |
| --- | ---- | ---- | -------------- |
| 主码流 | 90   | 95   | 高清取证，人脸/车牌需要细节 |
| 子码流 | 80   | 85   | 低分辨率图，高质量无意义   |

### 14.3 自动路由

```rust
fn select_quality(config: &SnapshotConfig, stream_type: StreamType) -> (u8, u8) {
    match stream_type {
        StreamType::Main => (
            config.main_stream_panoramic_quality,
            config.main_stream_crop_quality,
        ),
        StreamType::Sub => (
            config.sub_stream_panoramic_quality,
            config.sub_stream_crop_quality,
        ),
    }
}
```

`encode_and_save_snapshot` 调用方根据当前帧所属码流类型选择对应质量参数。

### 14.4 前端 UI

存储设置页「图片编码」Section 变为两个子区：

```
+---------------------------------------------------+
|  图片编码                                           |
|                                                     |
|  主码流（高清取证）                                   |
|  全景质量    [===========--]  90   ~280KB/张          |
|  特写质量    [============-]  95   ~400KB/张          |
|                                                     |
|  子码流（低功耗场景）                                 |
|  全景质量    [========-----]  80   ~160KB/张          |
|  特写质量    [=========----]  85   ~200KB/张          |
|                                                     |
|  裁剪边界扩展   [===---------]  10%                   |
|  编码器: MPP VPU OK  RGA OK                          |
|                              [保存]  [恢复默认]       |
+---------------------------------------------------+
```

用户一眼看到「主码流高清、子码流省空间」的 trade-off，比 per-camera 清晰得多。

---

## 15. 待讨论项

无。所有设计决策已关闭。
