# 媒体管线规范

> 从 RTSP 拉流到推理输入之间的全部处理。
> **核心边界：解码输出到推理输入的设备侧零拷贝，输入压缩码流存在一次 Host→Device DMA 复制。**

---

## 为什么零拷贝是硬要求与边界精确性

一帧 1080p NV12 约 3 MB。8 路 × 15fps：
- 每多一次 CPU 拷贝 = **360 MB/s 内存带宽**
- 嵌入式与边缘平台的内存带宽本来就紧张，几次未压缩图像拷贝就会压垮总线
- 拷贝还伴随 cache 严重污染与 CPU 满载

但必须**在工程与学术上严格区分压缩码流输入与未压缩图像数据**，杜绝笼统的“全链路零拷贝”宣称：
1. **输入码流（Host -> Device DMA 复制）**：RTSP 网络包在 CPU/Host 内存解包，送入 VPU/DVPP 硬件解码器时存在一次不可避免的 Host→Device DMA 复制。因码流是高压缩比数据（1080p 30fps 仅 2~8 Mbps），其开销极小。
2. **解码输出至推理输入（设备侧零拷贝）**：未压缩的原始图像（1080p NV12 约 93.3 MB/s）必须全程驻留于物理连续设备显存/共享总线，0 次 Host 内存回读，0 次 CPU 色彩转换。

---

## 系统三大核心数据路径

Argus 严格在架构与代码中切分三条独立路径，杜绝性能评估与排查误导：

```text
1. 常驻推理主路径 (infer_fast_path)：
   VPU / DVPP (硬解输出)
       │ (设备显存 / DMA-BUF 零拷贝流转)
       ▼
   RGA / VPC / AIPP (硬件缩放 / 色度转换 / 归一化)
       │ (设备侧零拷贝直通)
       ▼
   RKNN / ACL / ANE (异构 NPU/ANE 推理)

2. 低频证据生成路径 (snapshot_readback_path)：
   VPU / DVPP (告警按需解码单帧)
       │
       ▼ (Device-to-Host readback: aclrtMemcpy D2H / dma_buf mmap)
   Host Vec<u8> (CPU ITU-R BT.601 转换)
       │
       ▼
   JPEG Encode -> Disk (证据图片落盘)

3. 开发调试回退路径 (debug_cpu_fallback_path)：
   Host Memory -> CPU Software NV12 to RGB -> CPU Mock/Ort Inference (开发测试专用，带显式告警)
```

---

## 平台设备侧零拷贝通路 (infer_fast_path)

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

## 工业级 RTSP 接入内核 (Retina) 与 Annex B 直通规范

系统全面采用经过工业级考验的纯 Rust 异步库 `retina` (v0.4.20) 作为 RTSP/RTP 接入内核，彻底取代脆弱的手写状态机，实现开箱即用的真实安防摄像头容错与单二进制零 C 依赖交付：

### 1. 接入内核与 Annex B 零拷贝输出 (`RetinaIngestor`)
- **零凭证 URL 约束**：`retina::client::Session` 严格禁止在 `Url` 内部包含用户名和密码。接入前必须通过 `sanitize_rtsp_url_and_credentials` 将凭证剥离为 `retina::client::Credentials` 显式注入 `SessionOptions`。
- **Annex B 零拷贝输出**：
  - 在 `SetupOptions` 中配置 `frame_format(FrameFormat::SIMPLE)`；
  - 由 Retina 原生在解复用时注入 Annex B (`00 00 00 01`) 起始码和参数集（SPS/PPS/VPS）；
  - 将 `VideoFrame::into_data()` 直接包装为 `Bytes` 存入 `types::EncodedPacket`，全链路无二次内存重拷贝。
- **传输策略双轨映射**：
  - `TransportPolicy::Tcp` / `Auto` ➔ `retina::client::Transport::Tcp(TcpTransportOptions::default())`；
  - `TransportPolicy::Udp` ➔ `retina::client::Transport::Udp(UdpTransportOptions::default())`。
- **单调时钟与异步取消安全**：
  - 基于基准系统时钟结合 `frame.timestamp().elapsed_secs()` 换算 13 位 UTC Unix 毫秒时戳（`pts_ms`）；
  - 内置单调递增看门狗过滤，杜绝安防摄像头时间戳回跳导致 FLV 播放器卡死；
  - 结合 `tokio::select!` 与 `tokio::sync::watch::Receiver<bool>`，实现确定性的毫秒级停止响应。

### 2. 秒开关键帧与参数集缓存 (`KeyframeCache`)

- `StreamHub` 针对每路活跃流维护轻量内存 `KeyframeCache`：
  - **H.264**：缓存最近的 `SPS` (7)、`PPS` (8) 与 `IDR` 帧 (5)；
  - **H.265**：缓存最近的 `VPS` (32)、`SPS` (33)、`PPS` (34) 与 `IRAP` 帧 (16..21)；
- 无论客户端何时接入，在握手建连第一时刻直接注入缓存参数集与最新关键帧，实现 **<100ms 极速秒开首帧**。

---

## Scenario: RTSP 脏 URL 逆向锚点清洗与凭证隔离规范

### 1. Scope / Trigger
- Trigger: 安防监控现场强密码普遍包含 `@`, `:`, `#`, `?`, `!`, `&` 等保留字符（如 `rtsp://admin:p@ss:word#123@192.168.1.10:554/live`）。标准 `url::Url::parse` 会将第一个 `@` 误认为 userinfo 终止符，导致 `InvalidPort` 崩溃、路径腰斩或在日志中泄露部分明文密码。需要建立工业级逆向锚点解析、凭据剥离与脱敏规范。

### 2. Signatures
- `crates/media/src/rtsp.rs`:
  ```rust
  pub struct ParsedRtspUrl {
      pub scheme: String,
      pub username: Option<String>,
      pub password: Option<String>,
      pub host: String,
      pub port: Option<u16>,
      pub path_and_query: String,
  }

  impl ParsedRtspUrl {
      pub fn to_clean_url(&self) -> Result<url::Url, MediaError>;
      pub fn to_masked_string(&self) -> String;
      pub fn to_canonical_key(&self) -> String;
  }

  pub fn parse_and_clean_rtsp_url(raw_url: &str) -> Result<ParsedRtspUrl, MediaError>;
  pub fn mask_rtsp_url(raw_url: &str) -> String;
  ```
- `crates/media/src/retina_ingest.rs`:
  ```rust
  pub fn sanitize_rtsp_url_and_credentials(
      raw_url: &str,
  ) -> Result<(url::Url, Option<retina::client::Credentials>), MediaError>;
  ```
- `crates/media/src/stream_hub.rs`:
  ```rust
  pub fn canonicalize_rtsp_url(raw_url: &str) -> String;
  ```

### 3. Contracts
- `ParsedRtspUrl` 字段与约束：
  - `scheme: String`：协议前缀，统一规范为小写 `"rtsp"` 或 `"rtsps"`；
  - `username: Option<String>`：提取的原始用户名；若缺省则为 `None`；
  - `password: Option<String>`：提取的原始密码明文（不含任何转义变形，完整保留 `@`, `:`, `#`, `?`）；
  - `host: String`：主机名、IPv4 字符串或标准 IPv6 闭合形式（例如 `"[fe80::1]"`）；
  - `port: Option<u16>`：端口号（缺省时为 `None`，规范化键中自动缺省补齐 554）；
  - `path_and_query: String`：从第一个 `/` 开始的完整路径与参数串（若缺省则默认为空或 `"/"`）。
- `to_clean_url(&self) -> Result<url::Url, MediaError>`：
  - 产出的 `url::Url` **严格不得包含 username 与 password**，满足 Retina 原生约束并杜绝在 HTTP/RTSP 握手请求行中明文暴露凭证。
- `to_masked_string(&self) -> String`：
  - 产出安全脱敏字符串，密码替换为 `***`，用于全系统的错误追踪与日志打印。
- `to_canonical_key(&self) -> String`：
  - 产出标准复用 Key：补齐 554 端口、剥离末尾无意义斜杠，供 `StreamHub` 物理连接去重。

### 4. Validation & Error Matrix
| 触发条件 | 返回结果 / 错误类型 | 行为说明 |
| :--- | :--- | :--- |
| `raw_url.trim().is_empty()` | `MediaError::RtspConnect { reason: "URL 不能为空" }` | 拒绝空字符串 |
| 缺少 `://` 或协议非 `rtsp`/`rtsps` | `MediaError::RtspConnect { reason: "缺少协议头" 或 "不支持的协议类型" }` | 协议白名单限制 |
| 缺少 Host（例如 `rtsp:///path` 或 `@` 后无内容） | `MediaError::RtspConnect { reason: "URL 缺少主机地址" }` | 严防无效网络地址 |
| IPv6 格式缺少闭合括号 `]` | `MediaError::RtspConnect { reason: "IPv6 地址格式缺失闭合括号 ']'" }` | 严格校验 IPv6 语法 |
| 端口号非数字或数值超出 65535 | `MediaError::RtspConnect { reason: "端口解析失败" }` | 强校验合法端口范围 |
| 密码包含 `@`, `:`, `#`, `?`, `!` 等保留字符 | **成功解析** (`Ok(ParsedRtspUrl)`) | 逆向锚点定位，提取出纯净密码与纯净 URL |

### 5. Good/Base/Bad Cases
- **Good (复杂强密码)**:
  - 输入：`rtsp://admin:p@ss:word#123@192.168.1.10:554/live/ch0`
  - 提取：`clean_url = "rtsp://192.168.1.10:554/live/ch0"`, `user = "admin"`, `pass = "p@ss:word#123"`
  - 脱敏：`rtsp://admin:***@192.168.1.10:554/live/ch0`
- **Base (标准 URL 与 IPv6)**:
  - 输入：`rtsp://admin:123456@[fe80::1]:554/live`
  - 提取：`clean_url = "rtsp://[fe80::1]:554/live"`, `host = "[fe80::1]"`, `port = 554`
- **Bad (非法格式)**:
  - 输入：`http://192.168.1.100/live` -> 拒绝非 RTSP 协议
  - 输入：`rtsp://admin:pass@:554/live` -> 拒绝空主机名

### 6. Tests Required
- `test_parse_and_clean_rtsp_url_special_chars`：断言包含 `@`, `:`, `#`, `?` 及 IPv6、路径带 `@` 的 URL 正确提取组件；
- `test_mask_rtsp_url`：断言复杂密码脱敏为 `***`，杜绝截断切片泄漏；
- `test_sanitize_rtsp_url_with_reserved_characters`：断言纯净 URL 无凭证且 Credentials 还原真实密码；
- `test_canonicalize_rtsp_url`：断言带特殊字符密码的 URL 在端口缺失、尾随斜杠下的复用一致性。

### 7. Wrong vs Correct
#### Wrong (直接依赖 Url::parse)
```rust
// ❌ 错误：直接依赖标准 Url::parse 解析包含特殊字符密码的原始 URL
let mut parsed = url::Url::parse(raw_url)?; // 当密码含 '@' 时，第一个 '@' 被误判为 userinfo 终止符，导致 host/port 解析崩溃！
let username = parsed.username().to_string();
let password = parsed.password().unwrap_or("").to_string();
```
#### Correct (逆向锚点切分与凭据隔离)
```rust
// ✅ 正确：基于 Host 绝不可能包含 '@' 的数学不变性，逆向锁定分割边界
let parsed = parse_and_clean_rtsp_url(raw_url)?;
let clean_url = parsed.to_clean_url()?; // 彻底剥离凭证的纯净 Url
let creds = match (parsed.username, parsed.password) {
    (Some(username), Some(password)) => Some(Credentials { username, password }),
    _ => None,
};
```

---

## 统一探活引擎与元数据自愈 (StreamProber)

探活引擎与拉流引擎必须保持 100% 协议一致性：

- **全面收敛至 Retina DESCRIBE**：废弃脆弱的手写 TCP 握手与 Digest 鉴权拼接，统一调用 `retina::client::Session::describe(clean_url, session_options)`；
- **多级分辨率与帧率探测**：
  1. **优先提取**：从 `stream.parameters() -> ParametersRef::Video` 获取标准 `pixel_dimensions()` 与 `frame_rate()`；
  2. **降级兜底**：若参数集未就绪，通过 `session.sdp()` 原始文本调用 `parse_sdp` 提取 `sprop-parameter-sets` (H.264) 或 `sprop-sps` (H.265)；
  3. **尺寸待定状态**：若 SDP 未显式提供 SPS，返回 `(0, 0, fps)` 待定状态，严禁硬编码伪造 1080P。
- **离线与故障严谨门禁**：严格校验 `m=video` 视频轨与 200 OK 响应，若返回 401/404/500 或仅有音频轨，一律返回 `MediaError::RtspConnect`。

---

## 实时流媒体分发引擎：HTTP-FLV / Enhanced FLV

系统全面收敛于 **HTTP-FLV / Enhanced FLV (MSE 硬件解码流)** 架构，兼顾极致低延迟（~150ms）与 100% 跨浏览器 H.264 / H.265 硬件解码支持：

- **端点**：`GET /api/v1/live/{cameraId}.flv?stream=main|sub&token={jwt}`
- **协议格式**：
  - 标准 FLV Header（9 字节 + 4 字节 PreviousTagSize0）；
  - **H.264**：`AVCDecoderConfigurationRecord` (SPS/PPS) + FLV Video Tag (`0x17` / `0x27`)；
  - **H.265**：**Enhanced FLV (FourCC `hvc1`)**，输出 `HEVCDecoderConfigurationRecord` (VPS/SPS/PPS) + ExVideoTag (`0x90` / `0x91` / `0xa1`)；
- **前端消费**：基于 `mpegts.js`（MSE 架构），浏览器通过 GPU 硬件解码原生播放 **4K/1080P H.265 与 H.264**，杜绝浏览器 WebRTC 软解黑屏与复杂的 ICE/STUN/DTLS 端口穿透负担。

---

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

## 工业级边缘温控与热安全闭环规范 (Thermal Safety Engine)

嵌入式边缘异构平台（如 Rockchip RK3588、华为昇腾 Ascend 310B / Atlas 200I/500 等无风扇被动散热设备）在长时间高并发推理场景下，极易发生结温迅速攀升。若仅靠内核物理节流（CPU Throttling）或无闭环的跳帧，无法阻断热失控。

系统必须遵循以下热安全闭环准则：

1. **多 Thermal Zone 热点感知**：
   - 自动探测并监控系统所有 thermal zones（CPU、GPU、NPU、SoC、DDR），温控决策基于**最高热点结温 (Peak / Hotspot Temperature)** 驱动。
2. **硬件平台散热档案配置化**：
   - 严禁硬编码统一温度阈值；支持针对 RK3588、Ascend 310B、Atlas 500 提供差异化策略配置文件（如 RK3588 无风扇通常 80℃ 预警 / 90℃ 临界，Ascend 310B 通常 75℃ 预警 / 83℃ 临界）。
3. **传感器故障保守安全模式 (Conservative Mode)**：
   - **严禁在温度采样失败（sysfs 不可读或探头故障）时盲目假设 Normal**；
   - 采样连续失败时自动进入 `ThermalLevel::Conservative` 模式，强制执行 50% 保护性降载、暂停新任务准入，并上报设备维护告警。
4. **防抖 (Debouncing) 与迟滞 (Hysteresis) 机制**：
   - **升温防抖**：必须连续 $N$ 次采样超温方可触发升级，过滤单点偶发温度尖峰毛刺；
   - **迟滞回退**：降温恢复必须低于 `Threshold - Hysteresis_Delta`（例如 4.0℃~5.0℃），并满足连续平稳降温冷却观察期，杜绝阈值线附近剧烈冷热循环。
5. **全闭环控制矩阵 (Action Plan)**：
   - `Normal`：全速处理，允许新任务；
   - `Warning`：50% 阶梯跳帧，产生设备预警；
   - `Critical`：75% 应急削峰，**阻断新任务准入**；
   - `Emergency`：**切断非关键辅码流，停止高负载常规推理**，全力防止硬件热关机；
   - `Conservative`：50% 保守限流，阻断新任务，上报传感器维护告警。

---

## 异构硬件解码器工作线程优雅停机与超时隔离规范 (Graceful Shutdown & Join Timeout)

Linux 系统工程与嵌入式多媒体管线中，单纯依赖 `drop(sender)` + `blocking_recv() -> None` + `thread.join()` 是**致命的无界阻塞缺陷**。若工作线程恰好进入底层驱动 FFI（如 `mpp_decode_put_packet`、`mpp_decode_get_frame`、`aclvdecSendFrame`、`aclrtProcessReport`）而硬件因内核态挂死未返回，主析构线程将永久死等，导致整个守护进程在摄像头注销、重构或停机时挂死（Hang）。

必须严格遵循以下六步停机生命周期与隔离策略：

1. **协同停止信号 (Stop Signal)**：
   - 维护 `shutdown_flag: Arc<AtomicBool>`，停机时首先置为 `true`，向解码命令队列显式发送 `DecodeCommand::Stop` 并释放 `tx`；
   - 内部每一步耗时硬件操作（如 `decode`、`flush`、`drain_in_flight_frames`）在循环与入口处检查 `shutdown_flag`，若为 true 立即提前 abort 退出。
2. **显式唤醒与队列停止**：
   - 显式调用显存池的 `pool.close()`，唤醒所有因显存池耗尽挂起的解码线程；
   - 显式通知 Report 驱动线程退出循环（`report_running.store(false)`）。
3. **带超时的退出等待 (Join with Timeout)**：
   - 工作线程在退出前最后一刻通过标准 channel 发送 `exit_tx.send(())`；
   - 外壳在 `stop(timeout: Duration)` 中通过 `exit_rx.recv_timeout(timeout)`（默认 500ms）等待线程退出；
   - **成功退出**：调用 `thread.join()` 保证 0 毫秒立即返回并彻底回收 OS 线程资源；
   - **超时挂死 (Driver Hang)**：记录 `error!` 级别日志（包含 `camera_id`、`timeout_ms` 及驱动挂起警告），**绝对不再调用阻塞的 `thread.join()`**，执行线程隔离放弃，确保守护进程能够继续安全清理其他摄像头并正常执行关机/重启流程。
4. **统一析构顺序**：
   - `Signal Stop` -> `Close Channel/Pool` -> `Wait Worker with Timeout` -> `Destroy Hardware Desc/Context` -> `Release Native Buffers`。

---

## 异构硬件解码器动态分辨率安全重配与防抖规范 (DVPP / MPP)

在网络视频流出现分辨率动态切换（如监控摄像头自适应码率或恶意流构造 SPS 扰动）时，硬件解码器必须遵循严格的物理硬件生命周期与安全防护约束：

1. **白名单与安全边界校验**：
   - 仅允许合法尺寸（$128 \le \text{width} \le 3840$, $128 \le \text{height} \le 2160$）；
   - 偶数像素校验（$\text{width} \pmod 2 == 0 \land \text{height} \pmod 2 == 0$），总像素数严禁超过 4K 边界；拒绝畸形/极端尺寸导致的显存溢出。
2. **变更冷却时间 (Cooldown Window)**：
   - 设置最小变更间隔（5 秒）：在冷却期内忽略任何后续 SPS 变更，抑制瞬时抖动。
3. **高频抖动熔断与降级模式 (Flapping Circuit Breaker & Degraded Mode)**：
   - 在 60 秒滑动窗口内，若变更次数达到 3 次，判定为恶意码流 DoS 攻击或物理链路震荡；
   - 触发熔断保护，将解码器置为 `is_degraded = true`，锁定当前分辨率，拒绝一切后续重配，杜绝连续物理显存碎片化与耗尽。
4. **严格的硬件 Drain-Before-Switch 语义**：
   - 切换前必须先调用 `drain_in_flight_frames` 排空旧通道在途所有未完成硬件任务；
   - 确保 `in_flight_count == 0`，所有旧输入描述符与输出描述符在回调中安全终结；收割的旧帧单调暂存交付，绝不丢帧。
5. **显存预算与通道销毁重建顺序 (Buffer Budget & Destruction Ordering)**：
   - 单通道设定显存预算硬上限（200 MB），依据单帧步长容量动态计算安全块数（8~20 块）；
   - 严格遵循硬件销毁顺序：先销毁旧 channel，再销毁旧 channel desc；
   - 尝试分配新池，若分配失败优雅回退至旧配置并降级；
   - 成功后替换新显存池，并以新参数与步长重建新通道。

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
- **真·活跃数据包感知 (Watchdog Integrity)**：
  - 严禁以“存在后台拉流协程任务”作为流在线判断依据；
  - 看门狗必须通过 `StreamHub::is_healthy_streaming(camera_id, max_age_ms)` 严格校验**最近 4000ms 内是否确实有 NALU 数据包流入**；若超时无数据包流入，必须回退至主动探活并判定故障。
- **主动探活门禁严谨性 (Prober Gate)**：
  - RTSP TCP 探活在发送 DESCRIBE 后，必须严格校验返回状态码为 `200 OK` 且 SDP 中包含 `m=video` 视频轨；
  - 严禁在 404 / 401 / 500 等错误响应下回退返回默认 1080P/H.264 虚拟信息，杜绝离线设备被误判为健康。
- **三态防抖健康模型（Anti-Flapping 3-State Model）**：
  - 避免网络单包丢失导致状态在红绿灯之间频繁横跳：
    - `Healthy 🟢`：码流接收正常，探活连续成功；
    - `Degraded 🟡`：发生首次瞬时网络抖动或正在重连，容错缓冲窗口（连续重试 3 次或持续 10 秒内不向用户报死亡）；
    - `Failed 🔴`：连续 3 次探活失败或重连超过容错上限无数据，才正式判定为离线并记录错误日志。
- **双轨感知机制**：
  - **活跃拉流流**：由 RTSP 接收 Actor 实时感知断线并触发重连；
  - **静默待机流**：由后台定时巡检任务（30s 周期，启动时立即执行首次判定）轮询执行轻量探活并通过 WebSocket 广播同步状态。
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
