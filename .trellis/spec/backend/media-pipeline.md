# 媒体管线规范 (Media Pipeline Guidelines)

> 从 RTSP 拉流、解复用、硬解码、抽帧门控到推理输入之间的全链路媒体处理规范。
> 核心铁律：**解码输出到推理输入严格维持设备侧零拷贝，全常驻推理路径严禁 CPU 像素拷贝与色彩转换**。

---

## 1. 三大核心数据路径 (Three Data Paths)

Heimdall 在架构与代码中严格切分三条独立路径，杜绝性能评估误导与伪硬件加速：

```text
1. 常驻推理主路径 (infer_fast_path) —— 纯物理设备侧零拷贝：
   VPU / DVPP (硬件解码输出)
       │ (物理设备显存 / DMA-BUF / CVPixelBuffer 零拷贝直通流转)
       ▼
   RGA / VPC / AIPP (2D 硬件缩放 / 色度转换 / 步长对齐)
       │ (设备侧显存直达 NPU)
       ▼
   RKNN / ACL / ANE (异构 NPU 极速推理)

2. 低频证据生成路径 (snapshot_readback_path) —— 告警显式特例：
   VPU / DVPP (告警触发按需解码单帧) ──> Device-to-Host readback (mmap/D2H) ──> CPU JPEG 编码落盘

3. 开发调试回退路径 (debug_cpu_fallback_path) —— 仅在物理无 NPU 时保底，带显式告警日志
```

### 平台设备侧零拷贝通路一览

| 平台 | 硬件解码输出 | 传递载体 | 2D 硬件预处理 | 推理输入载体 |
|------|------------|---------|-------------|-------------|
| **Rockchip** | MPP 解码到 DMA-BUF | `DMA-BUF fd` | RGA (格式转换/缩放) | RKNN 直接绑定 DMA-BUF fd |
| **Huawei Ascend** | DVPP 解码到 Device 内存 | `acldvppPicDesc` | AIPP (融进模型) / VPC | AscendCL 直接吃 Device 内存 |
| **Apple Silicon** | VideoToolbox 解码输出 | `CVPixelBuffer` (IOSurface) | vImage / Metal | Core ML / ANE 零拷贝直通 |
| **CPU 回退** | FFmpeg 软解到堆内存 | `Vec<u8>` | fast_image_resize | 内存切片（开发机测试专用） |

---

## 2. `FrameRef` 与跨平台帧内存契约

```rust
// crates/types/src/frame.rs
pub struct FrameRef {
    pub camera_id: CameraId,
    pub timestamp: i64,          // 13 位 UTC Unix 毫秒时间戳（与 PTS 对齐）
    pub width: u32,
    pub height: u32,
    pub stride: StrideInfo,      // 硬件对齐步长（hor_stride 虚宽 / ver_stride 虚高）
    pub format: PixelFormat,
    handle: FrameHandle,         // 平台原生句柄（OwnedFd / DeviceMemory / ApplePixelBuffer）
}
```

- **生命周期与所有权**：`FrameRef` 满足 `Send + 'static`，可在跨线程通道中安全转移所有权；Drop 时通过 RAII 自动归还池槽位，不解构泄漏；
- **步长虚宽虚高铁律**：严禁假设 `stride == width`！Rockchip MPP 要求 `hor_stride` 16/64 字节对齐且 `ver_stride` 16 字节对齐（如 1080p 虚高为 1088）；昇腾 DVPP 要求宽 16、高 2、行跨度 128 对齐。传入硬件单元必须使用 `StrideInfo`，禁止直接传宽高。

---

## 3. 工业级 RTSP 接入内核 (Retina) 与 Annex B 直通

- **选型与解复用**：全面采用纯 Rust `retina` 异步客户端，`SetupOptions` 配置 `frame_format(SIMPLE)`，原生注入 Annex B 起始码，`VideoFrame::into_data()` 直接包装为 `Bytes` 存入 `EncodedPacket`，全链路零 CPU 重拷贝；
- **单调时钟看门狗**：基于系统单调时钟结合帧增量换算 13 位 UTC 毫秒 `pts_ms`，内置看门狗过滤，杜绝安防摄像头时间戳回跳导致播放器卡死；
- **秒开关键帧缓存 (`KeyframeCache`)**：内存常驻缓存最新 H.264 (SPS/PPS/IDR) 与 H.265 (VPS/SPS/PPS/IRAP) 参数集，客户端建立连接首刻直接注入，实现 `<100ms` 秒开首帧。

---

## 4. RTSP 脏 URL 逆向锚点清洗与凭证隔离

监控强密码常包含 `@`, `:`, `#` 等保留字符，标准 `Url::parse` 会将第一个 `@` 误判为分隔符导致崩溃或密码截断泄露：
- **逆向锚点切分契约**：基于 Host 绝不包含 `@` 的数学不变性，从最后一个 `@` 倒推定位 Host/Port，提取原始明文账号密码（保留特殊字符）；
- **凭据彻底剥离**：输出给 Retina 的 `Url` 必须彻底剔除凭证信息（密码转为独立 `retina::client::Credentials` 注入），日志输出统一调用 `mask_rtsp_url()` 将密码脱敏为 `***`。

---

## 5. 实时流媒体分发引擎：HTTP-FLV / Enhanced FLV

系统收敛于 **HTTP-FLV / Enhanced FLV (MSE 硬件解码流)** 架构：
- **端点**：`GET /api/v1/live/{cameraId}.flv?stream=main|sub&token={jwt}`；
- **协议兼容**：支持标准 H.264（`AVCDecoderRecord`）与 **Enhanced FLV H.265 (FourCC `hvc1`)**；
- **前端消费**：基于 `mpegts.js`（MSE 架构），浏览器通过 GPU 硬件解码原生播放 4K/1080P H.265 与 H.264，延迟压至 150~200ms，杜绝 WebRTC H.265 软解黑屏。

---

## 6. 解码器线程隔离与超时停机安全 (Graceful Shutdown)

- **专用 OS 线程**：每路摄像头的解码实例绑定专属 OS 线程，**严禁进入 Tokio 异步工作线程**；
- **带超时的退出等待 (Join Timeout 500ms)**：
  - 协同停机：置位 `shutdown_flag`，唤醒阻塞的 buffer pool 并退出循环；
  - 超时隔离：外壳在 `stop()` 中通过 `exit_rx.recv_timeout(500ms)` 等待；若驱动在内核态挂死超时，记录 `error!` 并放弃 `thread.join()`，执行线程隔离，严防守护进程死锁。

---

## 7. 工业级温控安全闭环 (Thermal Safety Engine)

| 状态等级 | 触发条件 | 动作策略 |
|---------|---------|---------|
| `Normal` | 结温在安全阈值以下 | 全速推理，准入新任务 |
| `Warning` | 达到预警阈值 (如 RK3588 80℃, Ascend 75℃) | 50% 阶梯跳帧，产生设备预警 |
| `Critical` | 达到临界阈值 (如 RK3588 90℃, Ascend 83℃) | 75% 应急削峰，**阻断新任务准入** |
| `Emergency` | 超出临界极限持续升温 | **切断非关键辅流，暂停高负载常规推理**，防止硬件热关机 |
| `Conservative` | 温度传感器读取失败/sysfs 损坏 | 50% 保守限流，阻断新任务，上报传感器维护告警 |

- **迟滞防抖 (Hysteresis)**：降温恢复必须低于 `Threshold - 5℃` 且维持冷却观察期，杜绝阈值附近剧烈冷热震荡。

---

## 8. 动态分辨率安全重配 (DVPP/MPP)

摄像头动态切换码流分辨率时必须满足：
1. **尺寸白名单**：$128 \le w \le 3840$, $128 \le h \le 2160$，偶数对齐，超限直接拒绝；
2. **防抖与熔断**：5 秒变更冷却期；60 秒内变更达 3 次触发熔断降级（`is_degraded = true`），锁定分辨率防 DoS；
3. **Drain-Before-Switch**：切换前先调用 `drain_in_flight_frames` 排空旧硬件通道在途任务，确保 `in_flight == 0` 后再销毁旧通道并重建新显存池。

---

## 9. 抽帧门控与健康检测

- **门控级联顺序**：解码 (25fps) ──> 抽帧降频 (5fps) ──> 降采样小图 (320x180) CPU 运动检测 ──> ROI 区域过滤 ──> 送 NPU 推理（实际吞吐 ~0.5fps）；
- **DMA-BUF Cache 一致性**：CPU 映射读取 DMA-BUF 做运动检测时，前后必须显式调用 `DMA_BUF_IOCTL_SYNC` (START/END)，严防脏 Cache；
- **健康检测三态防抖**：
  - `StreamHub::is_healthy_streaming` 校验最近 4000ms 内必须有真实 NALU 数据流入（非单纯协程存活）；
  - `Healthy 🟢` ──> `Degraded 🟡` (10s 容错缓冲) ──> `Failed 🔴` (连续 3 次探活失败正式判定离线)。

---

## 10. 禁止事项 (Iron Rules)

- ❌ 在常驻推理流水线上使用 CPU 像素拷贝或 CPU 色彩转换
- ❌ 忽略硬件对齐步长，直接将 `width/height` 当作 stride 传给硬件加速器
- ❌ 将包含密码的原始 RTSP URL 传给外部库或打印到日志（必须逆向锚点清洗脱敏）
- ❌ 在 Tokio worker 中执行硬件解码 FFI 或阻塞等待
- ❌ 停机时无超时死等 `thread.join()`（必须 500ms 超时隔离）
- ❌ 跳过门控级联直接将 25fps 全帧送入 NPU
- ❌ 温度传感器读取失败时盲目假设 Normal（必须进入 Conservative 保守降级模式）
