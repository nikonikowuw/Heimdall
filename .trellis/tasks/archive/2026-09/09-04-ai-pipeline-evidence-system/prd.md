# 边缘端一体化 AI 视频分析与多维证据闭环系统 (Master Task)

## Goal

基于已收敛的技术规范（`@prd/prd-v1.0.md`），在单一 Rust 可执行文件（All-in-One Binary）内完成全链路功能闭环：
打通「C ABI 算法包动态热插拔与沙箱自测」 $\to$ 「子码流硬件解码与主码流双流内存环形抓拍」 $\to$ 「Engine 硬件级预裁剪、ByteTrack 航迹管理与空间几何规则引擎」 $\to$ 「证据中心三支柱（抓拍/告警/识别）持久化与加权原子淘汰」 $\to$ 「前端动态流布防工作台与全量控制台」。

## 子任务拆解与交付依赖

```
[09-04-c-abi-algo-sandbox] (阶段 1: C ABI 算法包宿主与沙箱)
           │
           ▼
[09-04-dual-stream-snapshot-pipeline] (阶段 2: 硬件按需解码与双流全高清抓拍)
           │
           ▼
[09-04-rules-engine-and-evidence-triad] (阶段 3: 业务规则引擎与证据三支柱落库)
           │
           ▼
[09-04-live-rules-designer-console] (阶段 4: 动态流布防设计器与证据中心控制台)
```

1. **Subtask 1: `09-04-c-abi-algo-sandbox`**
   - 映射 C ABI 虚表与结构体，基于 `libloading` 搭建平台拓扑感知（`algo-packages/{platform_id}/{algo_id}`）与七步沙箱校验；
   - 支持动态载入 `yolo26n`，通过内建自测图完成前向推理验证。
2. **Subtask 2: `09-04-dual-stream-snapshot-pipeline`**
   - Pipeline 按需驱动 VideoToolbox / MPP 硬件解码器（优先子码流 640×360）；
   - 主码流 NALU 仅进内存 Ring Buffer（零解码开销，2~3秒 GOP）；
   - 告警时按 PTS 精准定位解码单帧主码流，产出 1080P/4K 高清原图与特写抠图，异步硬件/SIMD JPEG 编码。
3. **Subtask 3: `09-04-rules-engine-and-evidence-triad`**
   - 感知与业务解耦：Engine 统一负责硬件级 Pre-crop ROI 与坐标还原；
   - Engine 统一运行 ByteTrack 航迹跟踪、ROI/Line/Mask 空间几何规则与 5 秒冷却；
   - SQLite 迁移落地抓拍表、告警表、识别对账表与底库表；
   - 后台 `statvfs` 水位保底与“图在案在，图销案销”加权原子级清理守护任务。
4. **Subtask 4: `09-04-live-rules-designer-console`**
   - 布防设计器升级为直接在子码流实时流画面上叠加透明交互层绘制几何规则；
   - 证据中心三重视图落地（违规告警、行迹抓拍、人脸/车牌识别左右对账）；
   - 算法包管理列表与上传自测页面；
   - `rust-embed` 单二进制构建与端到端闭环验收。

## Acceptance Criteria

- [ ] 所有 4 个子任务全部高质量交付并通过门禁。
- [ ] 接入标准 RTSP 摄像头，控制台能直接在动态实时流上绘制 ROI 和 Line 规则。
- [ ] 目标跨线或入侵瞬间，100ms 内触发告警，抓拍到对应主码流 1080P/4K 全高清原图并写入 SQLite。
- [ ] 证据中心能清晰检索违规告警、行迹抓拍与人脸/车牌识别对账，每一条记录点开 100% 有高清证据图片。
- [ ] 磁盘模拟写满时，`statvfs` 机制能稳定优先淘汰普通抓拍，保证告警大图不丢失，且彻底消除无图幽灵记录。
- [ ] 单一二进制执行文件体积小巧，空闲 CPU 占用低于 3%，内存底噪低于 45MB。
