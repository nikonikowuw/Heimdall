# 实现计划：接入 Retina 打造生产级 RTSP 接入内核

## 步骤拆解

1. [x] **依赖与配置接入**
   - 验证 `crates/media/Cargo.toml` 中 `retina = "0.4.20"` 依赖；
   - 验证 `cargo check -p media` 通过。

2. [x] **实现 `crates/media/src/retina_ingest.rs`**
   - 实现 `sanitize_rtsp_url_and_credentials` URL 与鉴权分离；
   - 实现 `RetinaIngestor` 结构体；
   - 实现 `RetinaIngestor::run_loop`（带指数退避与 `tokio::select!` 毫秒级取消）；
   - 实现 `RetinaIngestor::stream_session`（连接、describe、setup、play、demuxed 读取与 `EncodedPacket` 转换）；
   - 编写单元测试（URL 提取、凭据脱敏、格式转换逻辑）。

3. [x] **挂载至 `StreamHub`**
   - 更新 `crates/media/src/stream_hub.rs` 中的拉流执行逻辑，采用 `RetinaIngestor` 驱动实时码流摄取；
   - 导出 `RetinaIngestor` 到 `crates/media/src/lib.rs`。

4. [x] **统一 `StreamProber` 与 Retina**
   - 重构 `crates/media/src/probe.rs`，消除手写 TCP / HTTP-like RTSP 握手，直接接入 `retina::client::Session::describe`；
   - 优先通过 `ParametersRef::Video` 获取视频流显示尺寸与真实帧率，保留 `parse_sdp` sprop 兜底解析；
   - 确保测活与拉流行为绝对一致。

5. [x] **验证门禁与质量检查**
   - `cargo fmt --all`；
   - `cargo clippy --all-targets -- -D warnings`；
   - `cargo test --workspace` 确保所有 media 与 pipeline 测试全绿；
   - 记录 Session进展到 `journal-1.md`。
