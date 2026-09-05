# 边缘按需硬件解码与双流高清抓拍管线 执行落地计划 (Subtask 2)

## 执行步骤与阶段划分

```
Step 1: 主码流 NALU 内存环形队列 (MainStreamRingBuffer)
                       │
                       ▼
Step 2: macOS VideoToolbox 硬件解码器实现 (VideoToolboxDecoder)
                       │
                       ▼
Step 3: 靶向快进抽帧与异步 JPEG 编码引擎 (SnapshotEngine)
                       │
                       ▼
Step 4: Pipeline 按需调度与双流生命周期闭环
                       │
                       ▼
Step 5: 单元测试、集成验证与全量门禁检查
```

---

### Step 1: 主码流内存环形队列实现 (`MainStreamRingBuffer`)
- [ ] 在 `crates/media/src/ring_buffer.rs` 实现 `MainStreamRingBuffer`；
- [ ] 支持按容量（帧数与毫秒跨度）淘汰超期包，确保维护最近 2~3 秒完整 GOP；
- [ ] 实现 `get_gop_for_timestamp(target_pts)` 算法，精准返回用于快进解码的切片包列表；
- [ ] 编写单元测试验证 RingBuffer 在多并发推入和检索时的时序单调性与内存开销。

### Step 2: macOS VideoToolbox 硬件加速解码器 (`VideoToolboxDecoder`)
- [ ] 在 `crates/media/src/decoders/videotoolbox.rs` 实现 `VideoToolboxDecoder`；
- [ ] 支持 H.264 与 H.265 (HEVC) 参数集（SPS/PPS/VPS）解析与 `CMFormatDescription` 构建；
- [ ] 同步/异步解码 NALU 并输出封装了 `CVPixelBuffer` 的 `FrameRef`；
- [ ] 实现 `VideoDecoder` trait 与 RAII 显存自动释放。

### Step 3: 靶向精准抽帧与异步 JPEG 编码 (`SnapshotEngine`)
- [ ] 在 `crates/pipeline/src/snapshot.rs` 实现 `SnapshotEngine`；
- [ ] 接收抽帧请求：时标 $T$ 与目标 `BoundingBox`；
- [ ] 调用解码器快进重放 GOP 直至 $T$ 单帧高清原图；
- [ ] 异步线程池实现 JPEG 编码与按 BBox 高清特写抠图（Crop Image）；
- [ ] 降级机制：主流断线或无可用 GOP 时，无缝回退当前子流帧保底。

### Step 4: Pipeline 按需解码与双流闭环集成
- [ ] 升级 `crates/pipeline/src/manager.rs`：
  - 当 `task.desired_enabled == true` 时拉起解码工作线程；
  - 任务停止时即刻释放解码会话；
  - 接入 `StreamHub` 子码流推流广播；
  - 接入主码流 RingBuffer 存储与快照抽取。

### Step 5: 全量门禁与性能验收
- [ ] 运行单元测试与集成测试；
- [ ] 运行代码规范门禁：
  ```bash
  cargo fmt --all -- --check
  cargo clippy --all-targets -- -D warnings
  cargo test --workspace
  ```

---

## 验收核对表 (Checklist)

- [ ] `MainStreamRingBuffer` 内存占用稳定在 20MB 以内，CPU 占用接近 0。
- [ ] 仅当 AI 任务开启时才创建解码器，关闭任务后显存和解码实例立即销毁。
- [ ] 触发告警时能从 Ring Buffer 快进解码出高清原图并产出全景 JPEG 与特写 JPEG。
- [ ] 主码流断线时自动降级子流帧，保证 100% 不漏图。
- [ ] Clippy 与 workspace 单元测试 100% 通过。
