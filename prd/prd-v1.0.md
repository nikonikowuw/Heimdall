# Argus / Heimdall 边缘端一体化 AI 视频分析系统 产品需求文档 (PRD)

| 文档版本 | 修订时间 | 架构负责人 | 评审状态 | 目标形态 |
|---|---|---|---|---|
| **V1.1 (正式定稿版)** | 2026-09-04 | Antigravity 架构组 | 评审通过 (Grilling 决策收敛) | 纯 Rust 单一可执行文件 (All-in-One Binary) + React 现代化内嵌控制台 |

---

## 1. 文档概述与修订历史

### 1.1 修订记录

| 版本号 | 修订日期 | 修订人 | 修订说明 |
|---|---|---|---|
| V1.0 | 2026-09-04 | 架构组 | 初版草稿：提出由 Go+C++ 重构为 Rust 单二进制系统；保留操作审计日志。 |
| **V1.1** | 2026-09-04 | 架构组 & 评审组 | **重构架构全量收敛定稿**：<br>1. **流媒体协议锁定**：全面废除 WebRTC/WHEP，正式确立轻量自研 Enhanced FLV (HTTP-FLV / WS-FLV) + 前端 MSE 硬件解码播放；<br>2. **算法包生态与 C ABI**：推翻硬编码单一 YOLO 设想，100% 继承并升级原有 C ABI 规范（兼容 `sdk/include/argus/algo.h`），保留 `algo-packages/{platform_id}/{algo_id}` 动态热插拔与七步沙箱校验；<br>3. **两级分工体系**：确立“算法包只做纯感知（Perception）推理，Engine 宿主管线统管硬件级预裁剪（Pre-crop ROI）、坐标映射、ByteTrack 航迹管理、空间几何规则（ROI/Line/Mask）与 5 秒防重复冷却”；<br>4. **边缘高能效解码与双流高清抓拍**：默认拉取子码流（640×360）进行硬解与推理；主码流以 NALU 数据包仅进内存环形队列（Ring Buffer，零解码开销），告警瞬间按时标 $T$ 快速解码主码流帧，产出 1080P/4K 高清证据原图与目标特写图；<br>5. **业务证据中心三支柱**：重构单一告警表，建立“📸 抓拍（行迹回溯）”、“🚨 告警（违规事件）”、“👤 识别对账（人脸/车牌底库 1:N 对比）”及底库管理体系；<br>6. **存储保护与原子淘汰**：统一存储池，拒绝死板物理分区与高开销 `du` 磁盘扫描；依托系统调用 `statvfs` 水位兜底，优先淘汰普通抓拍以保全告警大图，严格执行“图在案在，图销案销”原子级清理；<br>7. **实时流动态布防工作台**：废除静态截图标注，布防设计器直接在子码流动态实时流画面上叠加透明交互层绘制几何规则；前端无违规时不推高频噪点框，仅在告警时弹出卡片与大图。 |

### 1.2 名词解释与关键术语

| 术语 / 缩写 | 英文全称 | 说明 |
|---|---|---|
| **All-in-One Binary** | 单一可执行文件 | 系统构建产物为一个独立的单二进制程序（`argus`），无需外挂动态运行时、无 Python/Node/Go 依赖，前端构建产物通过 `rust-embed` 直接编译内嵌。 |
| **Enhanced FLV** | Enhanced RTMP/FLV (v1.0.1) | 扩展版 FLV 容器标准，支持 FourCC `hvc1`（H.265/HEVC）与 `avc1`（H.264）的原生封装，通过 HTTP-FLV / WS-FLV 分块传输，延迟在 200~400ms 级别。 |
| **MSE** | Media Source Extensions | 浏览器 W3C 标准多媒体扩展接口，前端借助 `mpegts.js` 将接收到的 FLV 分块流直接解封装并交由浏览器/显卡底层硬件解码播放。 |
| **C ABI Algorithm Package** | C ABI 算法包 | 遵循标准 C ABI 虚拟函数表（`av_algo_abi`）封装的动态库（`.dylib` / `.so`），由独立目录资产管理（`algo-packages/{platform_id}/{algo_id}`），支持热插拔与沙箱自测。 |
| **Pre-crop ROI** | 硬件级预裁剪推理 | 用户指定特写区域（如道闸口车牌、收银台），由 Engine 利用底层 2D 硬件单元裁剪后再送入模型推理，坐标自动无损还原至全画面。 |
| **Evidence Triad** | 证据中心三支柱 | 系统承载的 3 种独立生命周期业务数据：**抓拍记录**（行迹回溯）、**告警记录**（安全防范违规）、**识别记录**（人脸/车牌底库 1:N 对比）。 |
| **Atomic Eviction** | 原子联动淘汰 | 磁盘配额告急时，系统将最老批次的证据图片与 SQLite 数据库行在同一事务中同步销毁，恪守“图在案在，图销案销”，绝不产生无图死记录。 |

---

## 2. 产品背景与重构愿景

### 2.1 现状与痛点剖析（原 Go + C++ 异构架构）

原系统 `/Users/zhang/dev/go/argus` 验证了算法包生态与媒体底座的可行性，但也暴露了多进程拆分架构的严重瓶颈：
1. **跨进程 IPC 与内存二次拷贝损耗**：
   业务调度运行于 Go，流媒体与算法推理运行于 C++ 引擎，二者通过 UNIX Domain Socket (UDS) / Protobuf 频繁序列化通信。告警图片和元数据在不同进程间打转落盘，增加了不必要的内存拷贝与 CPU 缓存抖动。
2. **进程碎片与部署极其沉重**：
   依赖 Nginx 跨域反代、依赖外部动态运行时，多进程守护与崩溃恢复逻辑脆弱。在边缘嵌入式硬件（RK3588、Atlas 200I 等）上交付时容器镜像体积大、排障链路长。
3. **架构负债与冗余包袱**：
   原系统包含了大量企业管理软件的多级 RBAC 逻辑（多用户树、角色权限、部门架构、动态菜单路由），对于专注边缘端自治的“即插即用”智能视频分析盒子而言极其臃肿。

### 2.2 重构核心价值与设计原则

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│                    Argus / Heimdall (Rust All-in-One Binary)                     │
│                                                                                  │
│  ┌────────────────────────┐  ┌────────────────────────────────────────────────┐  │
│  │  Embedded Web Console  │  │          High-Performance Rust Core            │  │
│  │  React 19 + TypeScript │  │  - Axum HTTP & Enhanced FLV (HTTP/WS) Stream   │  │
│  │  Tailwind CSS + Vite   │  │  - StreamHub Multiplexing (GOP Ring Buffer)    │  │
│  │  Live Stream Rules SVG │  │  - Pipeline: ByteTrack + Geometry Rules Engine │  │
│  │  rust-embed (Single)   │  │  - C ABI Dynamic Algo-Packages Sandbox Host    │  │
│  │                        │  │  - SQLite (WAL Mode) + Statvfs Atomic Eviction │  │
│  └────────────────────────┘  └────────────────────────────────────────────────┘  │
│                                                                                  │
│  ┌────────────────────────────────────────────────────────────────────────────┐  │
│  │  Hardware Acceleration Layer (Zero-Copy FrameRef: CVPixelBuffer / DMA-BUF) │  │
│  │  - macOS Apple Silicon: VideoToolbox + Core ML / ANE                       │  │
│  │  - Rockchip Linux: MPP Decode + RGA 2D Crop + RKNN NPU                     │  │
│  │  - Huawei Ascend Linux: DVPP VDEC + VPC 2D Crop + CANN ACL                 │  │
│  └────────────────────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────────────────┘
```

1. **单进程极简自包含（Zero-Dependency Single Binary）**：
   - 彻底废除跨进程 IPC，全链路统一收敛至 Rust。前端静态产物编译进单一可执行文件，无任何外部运行时或动态库依赖，单端口（默认 8000）自托管服务。
2. **边缘高能效与双流零拷贝（Energy-Efficient Edge Pipeline）**：
   - 默认采用子码流（640×360）硬解并喂入模型推理，消减边缘盒子 80% 的算力与显存发热；
   - 主码流（1080P/4K）纯吃包进内存环形队列（Ring Buffer），零解码开销；触发告警瞬间按时间戳 $T$ 快速解码单帧主码流，产出全高清证据大图。
3. **算法包生态热插拔（Algorithm Package Ecosystem）**：
   - 保持 C ABI 标准契约（兼容 `sdk/include/argus/algo.h`），算法包按平台物理隔离（`algo-packages/{platform_id}/{algo_id}`）；
   - 支持算法热加载与七步沙箱自检（Self-Test），用户可按规范自主开发并上传目标检测、人脸识别、车牌识别等各类定制算法。
4. **两级分工与免重复开发**：
   - 算法包只做“前处理 $\to$ NPU 推理 $\to$ NMS 输出”的纯感知；
   - Engine 统一负责硬件级预裁剪（Pre-crop ROI）、坐标映射、ByteTrack 航迹管理、空间几何判定（ROI/Mask/Line）与 5 秒防重复冷却，算法开发者无需重复编写业务逻辑。
5. **证据三支柱与“图在案在”原子淘汰**：
   - 全面支持抓拍、告警、识别三类业务记录；
   - 统一存储池管理，基于系统调用 `statvfs` 水位保底，空间不足时优先淘汰普通抓拍保护告警大图，删除图片与删除数据库行原子联动，绝不留无图幽灵记录。

---

## 3. 系统核心架构与功能需求

### 3.1 身份认证与安全审计中心 (Auth & Audit)

#### 3.1.1 开箱向导与轻量认证 (Setup & Authentication)
- **开箱向导（Out-of-the-Box Wizard）**：
  首次启动且未初始化时，前端访问自动强制重定向至 `/setup` 开箱向导页面，引导设置管理员账号与强密码，杜绝默认密码（如 `admin123`）被扫描利用的安全隐患。
- **环境静默配置（Headless Deployment）**：
  在自动化运维场景下，支持通过环境变量 `ARGUS_ADMIN_USERNAME` 与 `ARGUS_ADMIN_PASSWORD` 实现无人工干预的静默开箱初始化。
- **灾备密码重置（CLI Disaster Recovery）**：
  若现场运维遗忘管理员密码，严禁通过删除数据库的方式处理。系统提供命令行工具：
  ```bash
  ./argus reset-admin --password <new_password>
  ```
  在本地直接更新管理员哈希凭据，保障数据资产绝对安全。
- **JWT 状态管理**：基于 HS256 JWT 签发 24 小时访问令牌，并在 SQLite `system_configs` 维护撤销时间戳，支持管理员一键令所有活跃令牌失效。

#### 3.1.2 操作审计日志 (Operation Logs)
- 全局中间件拦截所有状态写操作（`POST`、`PUT`、`DELETE`、`PATCH`）；
- 记录操作人、IP、路径、耗时、状态码、以及脱敏后的请求报文体（密码等敏感字段掩码为 `******`）；
- 控制台提供专属审计日志列表页，支持时间跨度与模块筛选。

---

### 3.2 摄像头接入与流媒体中心 (Media Center)

#### 3.2.1 RTSP 接入与流复用 (StreamHub)
- **协议支持**：标准 RTSP 1.0，支持 TCP Interleaved RTP 传输，支持 Basic 与 Digest 鉴权；
- **流多路复用（Multiplexing）**：
  基于规范化 URL（`canonicalize_rtsp_url`）实现物理连接复用。多个客户端观看或多个分析任务订阅同一路摄像头时，底层仅维持单条 RTSP 上行会话；
- **按需拉流与平滑冷却**：
  当某路流既无 Web 客户端观看，又无启用的 AI 分析任务时，启动 5 秒平滑防抖倒计时，倒计时结束后自动断开 RTSP 连接释放网络与系统资源；
- **子码流智能推导**：
  内置海康（Hikvision/ISAPI）、大华（Dahua）、天地伟业（Tiandy）、宇视（Uniview）、TP-Link 等主流品牌的子码流规则推导引擎（`deduce_sub_stream`），添加主码流时自动生成候选子码流地址。

#### 3.2.2 超低延迟 Enhanced FLV 流媒体服务
- **容器与协议标准**：
  自研纯 Rust `FlvMuxer`，完全符合 Enhanced FLV 标准，支持 H.264（`avc1`）与 H.265/HEVC（FourCC `hvc1`）；
- **推流端点**：
  - HTTP-FLV 分块传输：`GET /api/v1/live/{camera_id}/flv?stream=main|sub`
  - WebSocket-FLV 传输：`GET /api/v1/live/{camera_id}/ws?stream=main|sub`
- **首包秒开与 GOP 缓存**：
  `StreamHub` 维护 `KeyframeCache`，保存最近的关键帧、SPS、PPS、VPS；新客户端接入时立即下发完整序列头与关键帧，播放器瞬间点亮画面；
- **低延迟体验**：
  前端集成 `mpegts.js`（MSE 硬件解码），局域网内端到端播放延迟稳定在 **200~400ms**，彻底规避 WebRTC 在内网下的 UDP 端口穿透与黑屏难题。

---

### 3.3 边缘高能效分析管线与算法包生态 (Pipeline & Algo Packages)

#### 3.3.1 平台感知型算法包拓扑与七步沙箱 (Algo Package Sandbox)
- **目录拓扑按平台隔离**：
  ```text
  algo-packages/
  ├── macos-arm64/
  │   ├── general_detection/ (yolo26n CoreML .dylib)
  │   └── face_recognition/
  ├── linux-arm64-rknn/
  │   ├── general_detection/ (RK3588/RK3576 .so)
  │   └── license_plate_recognition/
  └── linux-arm64-ascend/
      └── general_detection/ (Ascend OM .so)
  ```
- **七步安全沙箱自检（7-Step Sandbox Validation）**：
  在安装或热加载算法包时执行：
  1. 路径穿越与文件名安全性校验；
  2. 算法包 SHA256 哈希校验；
  3. `manifest.json` 格式、版本与平台标签（`platform_id`）严格比对；
  4. `config.schema.json` 校验；
  5. 动态库 `dlopen` 符号寻址与 C ABI 虚表结构体尺寸（`sizeof`）断言；
  6. **真实测试图自测（Self-Test）**：使用包内内置的 `testimage.jpg` 执行单次真实前向推理，验证其不崩溃、不泄漏；
  7. 原子迁移至可用算法库并热注册。

#### 3.3.2 边缘双流与按需解码机制
- **解码器按需启动**：
  硬件解码器（macOS VideoToolbox / Linux MPP / Ascend DVPP）仅在对应摄像头的 AI 分析任务启用时（`desired_enabled == true`）由 Pipeline 内部专有工作线程按需拉起，关闭任务时即刻销毁；
- **推理流优先子码流（640×360）**：
  分析任务默认绑定子码流，极大降低 VPU 与 DDR 总线带宽。若摄像头未配置子码流，系统前端提示并自动回退为主码流；
- **主码流内存环形缓存（Ring Buffer）**：
  主码流网络线程仅接收原始 NALU 包，以引用计数切片（`bytes::Bytes`）形式暂存进最近 2~3 秒的内存环形队列，**完全不调用硬件解码器（零 VPU 开销，内存仅占用约 10MB）**。

#### 3.3.3 两级解耦：感知与业务规则引擎
- **算法包职责（Perception）**：
  仅负责前处理、NPU 推理与 NMS。每帧输入 `FrameRef`，输出结构化目标数组 `Vec<Detection>`，完全无需关心跟踪、几何求交与冷却状态机；
- **硬件级预裁剪（Pre-crop ROI）**：
  若用户在布防中配置了局部裁剪推理（如道闸车牌特写）：
  1. Engine 调用底层 2D 硬件单元（CoreVideo / RGA / VPC）在显存内完成极速裁剪；
  2. 裁剪帧透明送入算法包推理；
  3. Engine 将返回的目标坐标通过线性仿射变换无损还原至全图归一化坐标系；
- **Engine 统一航迹跟踪（ByteTrack）**：
  纯 Rust 实现，根据检测框高低分两阶段关联，分配并平滑维护连续的 `track_id` 与移动轨迹向量；
- **Engine 统一空间几何规则引擎**：
  - **ROI 多边形入侵**：目标底部几何中心进入多边形触发；
  - **Mask 遮罩屏蔽**：位于多边形内的目标直接静默过滤；
  - **Line 绊线越界**：基于目标运动轨迹线段与绊线进行跨立求交，支持单向（`A->B`、`B->A`）与双向（`Both`）；
- **Engine 统一告警冷却**：
  同一 `track_id` 触发告警后，自动进入 5 秒防重复防抖冷却，避免连续帧剧烈刷屏。

#### 3.3.4 全高清证据双流抓拍
当子码流在时标 $T$ 判定告警时：
1. 向主码流管道发送精准时标抽帧指令；
2. 主码流工作线程从内存 Ring Buffer 调出包含 $T$ 的前置 I 帧与后续 P 帧，硬件瞬时快进解码出该时间戳的高清主码流帧（1080P/4K）；
3. 异步投递至专有 I/O 线程池，调用硬件/SIMD 图像库（macOS ImageIO / Linux TurboJPEG）快速编码为全高清 JPEG 证据原图，并按目标 BBox 裁剪出一张高清特写图；
4. 若主码流断线或未接入，自动降级提取触发当帧的子码流图像，保证 100% 不漏图。

---

### 3.4 业务证据中心三支柱与存储保护 (Evidence & Storage)

#### 3.4.1 证据中心三支柱模型 (Evidence Triad)

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                           Argus 业务证据中心 (Evidence Center)                   │
├─────────────────────────┬─────────────────────────────┬─────────────────────────┤
│  📸 抓拍记录 (Captures)  │   🚨 违规告警 (Alarms)       │  👤 识别对账 (Recognitions)│
├─────────────────────────┼─────────────────────────────┼─────────────────────────┤
│ - 行人 / 机动车全景通行  │ - ROI 区域入侵告警          │ - 1:N 人脸识别比对      │
│ - 目标特写抠图          │ - Line 绊线越界告警         │ - 车牌黑白名单识别通行  │
│ - 最佳质量分 (Best-Shot)│ - 未戴安全帽 / 抽烟违规     │ - 现场抓拍 vs 底库登记照│
│ - TrackID 行迹轨迹回溯  │ - 1080P/4K 全景高清原图     │ - 相似度得分 (94.2%)    │
└─────────────────────────┴─────────────────────────────┴─────────────────────────┘
```

1. **📸 抓拍记录（Captures）**：
   记录画面中正常通行的所有目标。保存全景大图、特写抠图、目标类型（人/车/非机动车）、质量分与连续 Track ID，供用户在控制台按时间轴回溯行迹。
2. **🚨 告警记录（Alarms）**：
   记录触发布防规则的安全防范违规事件。保存规则类型、事件等级、违规目标属性、1080P/4K 全景大图与特写图。
3. **👤 识别记录（Recognitions）**：
   记录人脸比对与车牌识别事件。保存现场抓拍照、匹配命中的底库人员/车辆档案（姓名、工号、车牌号、部门）、**注册底库登记照（Gallery Photo）**、以及**相似度得分（Similarity）**。
4. **底库管理（Galleries）**：
   系统提供人脸与车辆白名单底库管理。人脸特征向量（512 维 FP32 Blob）在算法初始化时直接载入内存微秒级比对，无需外挂重度向量数据库。

#### 3.4.2 存储保护与“图在案在”原子联动淘汰
- **统一存储池，拒绝死板分区**：
  所有证据图片保存在统一目录（`var/data/evidence/`），不要求系统预先划分磁盘分区；
- **物理水位系统调用监控（`statvfs`）**：
  后台守护线程每 5 分钟调用一次轻量系统调用 `statvfs`，读取磁盘剩余空间百分比。当物理磁盘剩余空间 `< 15%` 或存储目录总容量超过设定的配额上限（如 10GB）时，自动触发淘汰；
- **加权优先级淘汰机制**：
  淘汰扫描时，**系统绝对优先批量删除“普通抓拍记录”（Captures）**；只有当抓拍已被清空仍不满足水位时，才滚动淘汰最老的告警与识别记录；
- **“图在案在，图销案销”原子清理**：
  每次淘汰一批记录时（如 500 条），在同一个操作步骤中完成：
  1. 从磁盘中物理 `unlink` 删除这 500 个 JPEG 图片文件；
  2. 从 SQLite 对应业务表中物理 `DELETE` 这 500 条数据库行；
  控制台看到的任何一条记录，**点开 100% 有图可查，绝无无图死记录**。

---

### 3.5 现代化 Web 控制台 (React 19 SPA)

#### 3.5.1 单二进制静态资源内嵌
- 前端采用 **Vite + React 19 + TypeScript + Tailwind CSS** 构建；
- 生产产物输出至 `web/dist`，由 Rust 宿主通过 `rust-embed` 编译进单一二进制，根路由自托管响应，支持 SPA 客户端路由回退（Fallback to `index.html`）。

#### 3.5.2 核心业务视窗与交互设计
1. **实时大屏（Live Viewport）**：
   - 1 / 4 / 9 多分屏自由切换，嵌入 `<LivePlayer />`（Enhanced FLV + MSE 播放）；
   - **低噪体验**：无违规时，画面纯净播放视频，不推高频噪点框；一旦发生越界或入侵告警，控制台伴随声音即时弹出醒目的告警卡片并定位快照。
2. **动态实时流布防工作台（Live Stream Rules Designer）**：
   - **彻底废除静态截图标注**；
   - 工作台直接运行子码流实时动态视频（延时仅 200ms）；
   - 视频上方覆盖一层透明交互矢量层（SVG / Pointer Events），值班员看着现场行人走动，直接在动态画面上点击绘制多边形（ROI/Mask）与折线绊线（Line，实时显示流动箭头与跨越方向）；
   - 坐标实时等比归一化至 `[0.0, 1.0]`，不受窗口缩放影响。
3. **证据中心三重視图（Evidence Hub）**：
   - **违规告警页**：卡片与表格视图，支持查看大图、目标框高亮、事件处理标记；
   - **抓拍回溯页**：按目标类别（人/车）与时间轴回溯历史经过记录；
   - **识别对账页**：**「现场抓拍照 | 相似度 94.2% | 底库登记照」** 左右直观对比核验。
4. **算法包管理（Algorithm Hub）**：
   - 呈现当前硬件平台可用的算法包列表（名称、版本、类别、目标平台标签）；
   - 支持上传 `.zip` 算法包并展示七步沙箱自检结果（自检通过后即刻热可用）。
5. **设备与任务管理（Cameras & Tasks）**：
   - 摄像头添加、一键测活、状态指示灯；
   - 任务配置向导：选择摄像头 $\to$ 选择算法包 $\to$ 选择推理流（默认推荐子码流） $\to$ 实时流上拉选规则 $\to$ 一键启动。
6. **安全操作审计（Audit Logs）**：
   - 清晰展示写操作流水，支持查看脱敏报文体抽屉组件。

---

## 4. 非功能性需求与性能指标 (Non-Functional Requirements)

| 指标项 | 目标阈值 | 验证条件 |
|---|---|---|
| **实时流播放端到端延迟** | **≤ 400 ms** (局域网实测典型值 200~300 ms) | RTSP 摄像头推流至浏览器 Enhanced FLV + MSE 画面呈现 |
| **单帧推理耗时** | **≤ 15 ms** | Apple ANE / RK3588 NPU (640×360 输入) |
| **告警触发到 Web 弹窗耗时** | **≤ 100 ms** | 目标跨线时刻至浏览器收到 WebSocket 告警事件 |
| **内存底噪开销 (Idle)** | **≤ 45 MB** | 单二进制进程启动完成，无活动流接入 |
| **4 路 1080P/4K 分析整机开销** | **内存 ≤ 200 MB，CPU 占用 ≤ 12%** | 4 路摄像头接入（子码流推理 + 主码流 RingBuffer 抓拍 + 1路预览） |
| **存储安全红线** | **磁盘剩余空间永远保持 ≥ 15%** | 高频告警压力测试下，`statvfs` 触发加权原子淘汰 |

---

## 5. 接口契约与数据库模型规范

### 5.1 RESTful HTTP API 统一信封

```json
{
  "code": 0,
  "message": "success",
  "data": {},
  "timestamp": 1747584000000
}
```
- `code = 0` 表示成功；非 0 表示业务错误，此时 `data` 为 `null`；
- 所有时间戳统一为 13 位 UTC Unix 毫秒整数；
- 字段命名严格遵循 `camelCase`。

### 5.2 核心 REST & WebSocket API 清单

| 端点路径 | 方法 | 说明 | 鉴权要求 |
|---|---|---|---|
| `/api/v1/auth/init-status` | GET | 查询系统是否已完成首次初始化 | 公开 |
| `/api/v1/auth/initialize` | POST | 开箱向导设置管理员用户名与密码 | 公开 (限未初始化时) |
| `/api/v1/auth/login` | POST | 管理员登录换取 JWT 访问令牌 | 公开 |
| `/api/v1/auth/password` | PUT | 修改管理员登录密码 | 需登录 (记日志) |
| `/api/v1/cameras` | GET | 获取摄像头列表及当前状态 | 需登录 |
| `/api/v1/cameras` | POST | 添加摄像头视频源（触发子码流推导与探活） | 需登录 (记日志) |
| `/api/v1/cameras/:id` | PUT | 更新摄像头信息及 RTSP 地址 | 需登录 (记日志) |
| `/api/v1/cameras/:id` | DELETE | 删除摄像头 | 需登录 (记日志) |
| `/api/v1/cameras/:id/probe` | POST | 手动发起单次连接探活与参数提取 | 需登录 |
| `/api/v1/live/:id/flv` | GET | HTTP-FLV 实时流拉取（支持 `stream=main\|sub`） | 需登录/Token |
| `/api/v1/live/:id/ws` | GET | WebSocket-FLV 实时流拉取（支持 `stream=main\|sub`） | 需登录/Token |
| `/api/v1/algorithms` | GET | 获取当前平台已加载的算法包列表 | 需登录 |
| `/api/v1/algorithms/upload` | POST | 上传算法包 `.zip` 并执行七步沙箱校验热安装 | 需登录 (记日志) |
| `/api/v1/tasks` | GET | 获取所有分析任务及运行状态 | 需登录 |
| `/api/v1/tasks` | POST | 创建并启动摄像头的 AI 分析任务 | 需登录 (记日志) |
| `/api/v1/tasks/:id` | PUT | 更新任务布防规则与配置 | 需登录 (记日志) |
| `/api/v1/tasks/:id/enable` | POST | 启用/停止指定摄像头的分析任务 | 需登录 (记日志) |
| `/api/v1/alarms` | GET | 分页检索安全违规告警记录 | 需登录 |
| `/api/v1/captures` | GET | 分页检索行迹回溯抓拍记录 | 需登录 |
| `/api/v1/recognitions` | GET | 分页检索人脸/车牌识别对账记录 | 需登录 |
| `/api/v1/galleries` | GET | 分页获取底库人员/车辆名单 | 需登录 |
| `/api/v1/galleries` | POST | 注册底库人员/车辆并提取特征 | 需登录 (记日志) |
| `/api/v1/evidence/:imageId` | GET | 获取全景大图或特写抠图图片流 (JPEG) | 需登录/Token |
| `/api/v1/logs/operations` | GET | 分页查询系统写操作审计日志 | 需登录 |
| `/api/v1/ws/events` | GET | WebSocket 统一实时业务事件通知（稀疏推送违规告警） | 需登录/Token |

---

### 5.3 数据库模型 Schema (SQLite WAL)

```sql
-- 1. 摄像头视频源表
CREATE TABLE cameras (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    camera_id VARCHAR(36) NOT NULL UNIQUE,
    name VARCHAR(128) NOT NULL,
    rtsp_url TEXT NOT NULL,
    sub_rtsp_url TEXT NOT NULL DEFAULT '',
    remark VARCHAR(255) NOT NULL DEFAULT '',
    last_probe_status VARCHAR(16) NOT NULL DEFAULT 'never',
    last_codec VARCHAR(16) NOT NULL DEFAULT '',
    last_width INTEGER NOT NULL DEFAULT 0,
    last_height INTEGER NOT NULL DEFAULT 0,
    last_fps REAL NOT NULL DEFAULT 0.0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 2. 分析任务配置表
CREATE TABLE analysis_tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id VARCHAR(36) NOT NULL UNIQUE,
    camera_id VARCHAR(36) NOT NULL,
    algorithm_id VARCHAR(64) NOT NULL,
    algorithm_version VARCHAR(32) NOT NULL DEFAULT '1.0.0',
    infer_stream VARCHAR(16) NOT NULL DEFAULT 'sub',      -- 'sub' (推荐) 或 'main'
    desired_enabled INTEGER NOT NULL DEFAULT 0,
    rules_json TEXT NOT NULL DEFAULT '[]',                 -- ROI, Mask, Line 规则定义
    config_json TEXT NOT NULL DEFAULT '{}',                -- 算法私有配置
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(camera_id) REFERENCES cameras(camera_id) ON DELETE CASCADE
);

-- 3. 🚨 违规告警记录表 (Alarms)
CREATE TABLE alarm_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id VARCHAR(64) NOT NULL UNIQUE,
    camera_id VARCHAR(36) NOT NULL,
    task_id VARCHAR(36) NOT NULL,
    alarm_type_id VARCHAR(64) NOT NULL,                    -- 如 'line_crossing', 'region_intrusion'
    occurred_at DATETIME NOT NULL,
    target_label VARCHAR(64) NOT NULL,                    -- 如 'person', 'car'
    confidence REAL NOT NULL DEFAULT 0.0,
    track_id INTEGER NOT NULL DEFAULT 0,
    bbox_json TEXT NOT NULL DEFAULT '[]',                  -- [x1, y1, x2, y2]
    image_id VARCHAR(64) NOT NULL,                         -- 1080P/4K 高清全景原图
    crop_image_id VARCHAR(64) NOT NULL DEFAULT '',         -- 目标特写抠图
    image_rel_path VARCHAR(255) NOT NULL,
    file_size_bytes INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_alarm_records_camera_time ON alarm_records(camera_id, occurred_at DESC);

-- 4. 📸 行迹抓拍记录表 (Captures)
CREATE TABLE capture_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    capture_id VARCHAR(64) NOT NULL UNIQUE,
    camera_id VARCHAR(36) NOT NULL,
    target_type VARCHAR(32) NOT NULL,                     -- 'person', 'vehicle'
    quality_score REAL NOT NULL DEFAULT 0.0,
    track_id INTEGER NOT NULL DEFAULT 0,
    bbox_json TEXT NOT NULL DEFAULT '[]',
    image_id VARCHAR(64) NOT NULL,
    crop_image_id VARCHAR(64) NOT NULL,
    image_rel_path VARCHAR(255) NOT NULL,
    file_size_bytes INTEGER NOT NULL DEFAULT 0,
    captured_at DATETIME NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_capture_records_time ON capture_records(captured_at DESC);

-- 5. 👤 识别对账记录表 (Recognitions)
CREATE TABLE recognition_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recognition_id VARCHAR(64) NOT NULL UNIQUE,
    camera_id VARCHAR(36) NOT NULL,
    target_type VARCHAR(32) NOT NULL,                     -- 'face', 'plate'
    recognized_id VARCHAR(64) NOT NULL DEFAULT '',         -- 关联底库 person_id 或 plate_number
    recognized_name VARCHAR(128) NOT NULL DEFAULT '',
    similarity REAL NOT NULL DEFAULT 0.0,
    gallery_image_path VARCHAR(255) NOT NULL DEFAULT '',   -- 底库登记照路径
    crop_image_id VARCHAR(64) NOT NULL,                    -- 现场抓拍特写图
    image_rel_path VARCHAR(255) NOT NULL,
    file_size_bytes INTEGER NOT NULL DEFAULT 0,
    observed_at DATETIME NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_recognition_records_time ON recognition_records(observed_at DESC);

-- 6. 底库人员与车辆名单表 (Galleries)
CREATE TABLE galleries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    member_id VARCHAR(64) NOT NULL UNIQUE,
    member_type VARCHAR(16) NOT NULL,                     -- 'person', 'vehicle'
    name VARCHAR(128) NOT NULL,
    group_name VARCHAR(64) NOT NULL DEFAULT 'default',
    identity_card VARCHAR(64) NOT NULL DEFAULT '',
    plate_number VARCHAR(32) NOT NULL DEFAULT '',
    photo_rel_path VARCHAR(255) NOT NULL DEFAULT '',
    feature_vector BLOB,                                   -- 512维 FP32 特征向量
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 7. 操作审计日志表 (Oplog)
CREATE TABLE operation_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username VARCHAR(64) NOT NULL,
    module VARCHAR(64) NOT NULL,
    action VARCHAR(64) NOT NULL,
    method VARCHAR(16) NOT NULL,
    path VARCHAR(255) NOT NULL,
    query TEXT,
    body TEXT,                                             -- 已脱敏 JSON
    status_code INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL,
    ip VARCHAR(64) NOT NULL,
    user_agent VARCHAR(255) NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_oplog_module_time ON operation_logs(module, created_at DESC);
```

---

## 6. 实施路线与阶段划分 (Execution Roadmap)

为了高质量交付这套工业级闭环系统，工程落地划分为 4 个高度聚焦的阶段：

* **阶段 1：C ABI 算法包宿主与沙箱加载体系（C ABI Host & Algo Sandbox）**
  - 在 Rust 侧 1:1 映射 `sdk/include/argus/algo.h` 的 C ABI 结构体与虚表；
  - 实现基于 `libloading` 的动态库加载、平台标签匹配与七步沙箱校验；
  - 成功热载入已有的 `yolo26n` 等算法包，通过内建自测图完成推理自检。
* **阶段 2：硬件解码、按需调度与双流高清抓拍（Dual-Stream Ingest & Snapshot）**
  - Pipeline 内部按需拉起 VideoToolbox / MPP 硬件解码器，绑定子码流；
  - 完善主码流 NALU 内存环形队列（Ring Buffer），实现告警瞬间按 PTS 精准回溯解码 1 帧主码流全高清原图与特写抠图；
  - 硬件/SIMD 异步 JPEG 编码与落盘。
* **阶段 3：感知解耦、统一规则引擎与证据三支柱落库（Rules Engine & Evidence Triad）**
  - Engine 统一承接硬件级预裁剪（Pre-crop ROI）与坐标还原；
  - Engine 统一运行 ByteTrack 航迹管理、空间几何判定（ROI/Mask/Line）与 5 秒防重复冷却；
  - 升级数据持久层，落地 `alarm_records`、`capture_records`、`recognition_records`、`galleries`；
  - 落地 `statvfs` 水位保底与“图在案在，图销案销”加权原子淘汰守护线程。
* **阶段 4：动态实时流布防工作台与前端控制台全量闭环（Live Rules Designer & Console）**
  - 升级布防设计器：在子码流实时动态视频流上叠加 SVG/Canvas 交互层，实时绘制并生成归一化规则；
  - 重构证据中心前端视图：违规告警列表、抓拍回溯时间轴、识别左右对账视图；
  - 算法包管理与上传页面；
  - `rust-embed` 单一二进制构建与端到端闭环验证。
