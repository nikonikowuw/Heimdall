# Design: RK3568 级联吸烟行为检测引擎 (Cascade Smoking Detection Engine for RK3568)

> **状态**: Draft（设计规范）  
> **关联规范**: [Nuwa 全局约定](../guides/conventions.md)、[算法 SDK 规范](../backend/algo-sdk-guidelines.md)、[检测告警契约](../backend/detection-alarm-contract.md)、[媒体管线](../backend/media-pipeline.md)、[运动门控引擎](./motion-detection-gating-engine.md)  
> **目标平台**: Rockchip RK3568 (Linux Kernel 5.10/6.1 BSP, RKNPU2 v2.x, RGA2, MPP)  
> **插件归属**: `algo-packages/rknn/rk3568/smoking_detection/`

---

## 1. 设计背景与核心挑战

吸烟行为检测是工业园区、加油站、仓储与高危防火区域的刚需布防场景。然而在边缘计算端侧落地时，面临严苛的物理矛盾：

1. **小目标与全图大算力的矛盾**：在 1080P/2K 监控画面中，香烟像素仅占 $10 \times 10 \sim 30 \times 30$ 像素。全图直接进行细粒度检测会导致大量反光、水杯、手指等误报；
2. **RK3568 单核微算力约束**：RK3568 仅配备 **单核 1.0 TOPS @ INT8 NPU**（`RKNN_NPU_CORE_0`）与 4 核 Cortex-A55（弱 CPU 算力）。相比 RK3588 的 6.0 TOPS 3-Core 配置，RK3568 的算力预算处于极度紧绷状态；
3. **多模型级联的流水线堵塞风险**：若每一帧对全图执行 YOLOv8n-pose，再无脑切图执行 YOLOv8n-cigarette，串行推理会形成明显的 NPU 排队压力。Pose 采用 `640×384` 输入后，模型输入像素相较 `640×640` 减少约 40%，但实际时延仍须在 RK3568 真机上测量；当出现多人并发动作时仍需通过抽帧、运动门控和单帧细检配额限制队列长度；
4. **内存带宽瓶颈**：RK3568 采用 32-bit LPDDR4/DDR4 统一内存架构，带宽仅约 $10 \sim 13\text{ GB/s}$。若在 CPU 侧执行图像切片、色彩转换和双线性插值，将严重抢占总线，使系统帧率劣化。

为此，本设计针对 RK3568 芯片微架构量身定制了**“四级漏斗门控 + 纯硬件端到端零拷贝 + 无头纯卷积 + 时序平滑状态机”**的级联架构，将设计目标控制在平均 NPU 算力消耗 35% 以内，保障 30fps 实时流的平稳吞吐与精准告警。该目标必须以 RK3568 真机测量结果为准，不能由模型输入尺寸线性推导替代。

---

## 2. 核心架构准则 (Core Architectural Principles)

1. **四级漏斗门控（Multi-Stage Funnel Gating）**：
   - **Level 1 (宿主运动门控)**：利用现有 `motion_gate`，画面无有效运动时 0 次 NPU 推理（拦截率 $\ge 60\%$）；
   - **Level 2 (时空分级抽帧)**：人体姿态变化具有物理惯性，Pose 模型采用 **5~10 fps 动态步长**（每 3~5 帧推理 1 帧），空窗帧由 ByteTrack 卡尔曼滤波插值补齐；
   - **Level 3 (空间几何先验初筛)**：严格根据手肘夹角（$< 55^\circ$）与手腕-口鼻相对距离（$< 0.8$）进行双臂初筛，95% 以上的常规活动帧在几何阶段阻断，不触发香烟推理；
   - **Level 4 (香烟细检占空比防抖)**：同一跟踪目标进入疑似状态后，设置 **250ms 冷却占空比**（单秒细检最多 4 次），配合时序累积分数状态机平滑判定。
2. **设备侧纯硬件零拷贝直通 (`infer_fast_path`)**：
   - MPP 解码输出 NV12 DMA-BUF 文件描述符（`AV_OPAQUE_DMABUF`）；
   - 全图预处理通过 RGA2 硬件执行 NV12 $\to$ RGB888 转换、保持 16:9 内容的 `640×384` Letterbox（标准 16:9 输入有效区域为 `640×360`，上下各补 12px）与 16 字节跨步对齐，直接注入预分配的 Pose 输入 DMA-BUF；
   - 局部 ROI 裁切通过 RGA2 硬件直接从源帧物理 NV12 显存中按动态坐标扣取并缩放至 $416 \times 416$，直接注入 Cigarette 输入 DMA-BUF；
   - 全流程 **0 次 CPU 像素拷贝、0 次 CPU 缩放插值、0 次 CPU 色彩空间转换**。
3. **无头纯卷积（Headless Conv）100% NPU 算子卸载**：
   - 彻底剥离 YOLOv8 尾部 DFL、Softmax、Sigmoid、Concat 等胶水算子；
   - Pose 输出 9 分支纯卷积张量，Cigarette 输出 6 分支纯卷积张量；
   - 算子 100% 映射至 RKNPU 硬件引擎，0 CPU Fallback，INT8 坐标量化零精度漂移。
4. **防张冠李戴（Anti-Stale Index Bug）的安全跟踪架构**：
   - 彻底废除原版 Python 原型中跨帧使用 `dets` 数组下标回查 `kpts[idx]` 的隐患机制；
   - 将 17 关键点骨骼坐标与更新时间戳深度内聚至 `STrack` 实体；目标失配或丢失时显式失效姿态数据，严禁引用过期或他人关键点。
5. **符合 Nuwa 算法 SDK 规范与单二进制交付**：
   - 算法插件以标准 `cdylib` 形式构建（`algo-packages/rknn/rk3568/smoking_detection/`），实现 `algo_sdk::plugin::AlgoPlugin`；
   - 严格遵循 `detection-alarm-contract.md` 契约，输出对角两点式归一化坐标 `[x1, y1, x2, y2]` 与结构化告警。

---

## 3. 硬件算力、内存与性能账本 (Budget & Sizing)

### 3.1 RK3568 硬件参数基准

| 硬件单元 | 物理规格 | 架构限制与设计硬约束 |
| :--- | :--- | :--- |
| **NPU** | 1.0 TOPS @ INT8，单核心 (`CORE_0`) | 无多核调度，模型间属于时分复用串行执行，必须杜绝算力堆叠 |
| **CPU** | 4× Cortex-A55 @ 2.0 GHz | 弱算力弱浮点，后处理必须采用 ARMv8-A NEON 向量化 |
| **2D 硬件** | RGA2 (2D Hardware Engine) | 16 字节 Stride 对齐，寻址受限于 DMA32 空间，最小尺寸 2px |
| **内存总线** | 32-bit LPDDR4/DDR4，带宽 $\approx 12\text{ GB/s}$ | 严格杜绝大块内存 memcpy，所有缓冲区在 `init()` 阶段预分配 |

### 3.2 Pose 输入与模型产物契约

Pose 模型固定使用 **`640×384`**：标准 16:9 内容保持为 `640×360`，在模型画布顶部和底部各补 `12px`，不对原始画面做纵向拉伸。

- 图模型输入形状为 `[1, 3, 384, 640]`（NCHW 图布局）；RKNN 外部输入的实际 `fmt/type/w_stride/h_stride` 必须以 `rknn_query()` 返回值为准，不能仅凭 ONNX 形状猜测；
- 对标准 16:9 源帧，保持比例缩放到有效区域 `640×360`，在目标画布顶部和底部各填充 `12px`。其他源比例按同一 `compute_letterbox_layout` 计算，运行时保存 `scale/pad_x/pad_y`，后处理使用同一布局逆变换；
- `384` 是 32 的整数倍，避免步长 32 特征图出现非整数尺寸；Pose 三个检测尺度的输出空间尺寸为 `80×48`、`40×24`、`20×12`（宽×高），总 anchor 数为 `80×48 + 40×24 + 20×12 = 5040`；
- 9 个无头输出张量按每个尺度的 `box/cls/kpt` 排列：`[1,64,H,W]`、`[1,1,H,W]`、`[1,51,H,W]`；后处理必须根据运行时查询到的实际 `H/W/stride` 校验，不得硬编码成未经验证的尺寸；
- 现有 `yolov8n-pose_cut.onnx` 和对应 RKNN 产物是 `640×640` 版本，不能直接复用为本设计的生产模型。应从原始 Pose 模型按 `(height=384,width=640)` 重新导出，再执行无头输出截取和 RKNN-Toolkit2 `rk3568` INT8 转换；不能只修改 ONNX 输入维度而忽略图内 shape 常量、Resize/Concat 形状和输出验证；
- 校准集必须经过与生产一致的 `640×384` letterbox。转换脚本、Toolkit2 版本、校准集身份、构建日志、RKNN SHA-256 和板端 `rknn_query()` 属性需要随模型一起归档。

> **模型证据状态：blocked。** 当前仓库已确认的 Pose ONNX/RKNN 证据对应 `640×640`；`640×384` 的 ONNX 身份、转换日志、实际 INT8/混合精度、RKNN 输出 stride 和 RK3568 真机时延尚未形成证据链。在这些证据补齐前，本文的尺寸与预算属于目标设计，不代表生产模型已经生成或验证。

### 3.3 单帧算力与时延预算 (Single-Frame Latency Breakdown)

在 RK3568（CPU 2.0GHz 锁定，NPU 1000MHz 锁定）实测推导基准。下表中 Pose 的 `640×384` 数值是设计预算，不是已经完成的真机实测；模型转换和板端基线完成后必须回填 P50/P95：

| 执行阶段 | 执行单元 | 输入/输出规格 | 物理耗时 | 说明 |
| :--- | :--- | :--- | :--- | :--- |
| **前置运动门控** | CPU (SIMD) | $160 \times 90$ Y 平面 | $\le 0.4\text{ ms}$ | 静态无运动直接跳帧放行（0 NPU 开销） |
| **RGA 全图 Letterbox** | RGA2 硬件 | 原图 NV12 $\to 640\times 384$ RGB888 | $\approx 1.2\text{ ms}$ | 16:9 内容缩放到 $640\times 360$，上下各补 12px；硬件色彩转换 + 16 字节对齐 |
| **YOLOv8n-Pose 推理** | RKNPU2 (INT8) | $640 \times 384 \times 3$ (9 纯卷积输出) | 设计预算 $\le 25\text{ ms}$，待真机测量 | 输入像素较 $640\times640$ 减少 40%；单核 NPU 执行，不能按线性比例承诺最终时延 |
| **Pose NEON 后处理** | CPU (NEON) | 9 输出 $\to$ 框 + 17 关键点（5040 anchors） | $\approx 1.8\text{ ms}$，待真机测量 | C/Rust 向量化 DFL + NMS；需以 5040 anchor 版本实测 |
| **ByteTrack 跟踪** | CPU | 坐标卡尔曼更新 | $\approx 0.3\text{ ms}$ | 纯整型/浮点矩阵状态更新 |
| **姿态几何先验初筛** | CPU | 关键点向量点乘与相对距离计算 | $\approx 0.05\text{ ms}$ | $< 55^\circ$ 与 $< 0.8$ 阈值判定 |
| **RGA 上半身 ROI 裁切** | RGA2 硬件 | 原图 NV12 $\to 416\times 416$ RGB888 | $\approx 0.7\text{ ms}$ | 硬件自适应抠图与缩放 |
| **Cigarette 推理** | RKNPU2 (INT8) | $416 \times 416 \times 3$ (6 纯卷积输出) | $\approx 14.8\text{ ms}$ | 3549 anchors 黄金分辨率 |
| **Cigarette NEON 后处理**| CPU (NEON) | 6 输出 $\to$ 香烟检出置信度 | $\approx 0.6\text{ ms}$ | 向量化 Sigmoid + 阈值过滤 |
| **时序状态机与发射** | CPU | 积分累加与 JSON 组装 | $\approx 0.05\text{ ms}$ | 状态转移与零分配 Emitter |

### 3.4 场景算力负荷与占空比测算

- **场景 A：无人或环境静止**
  - 运动门控拦截，仅保活心跳触发（每 2000ms 一次）：
  - 平均 NPU 占用：**$< 1.5\%$**。
- **场景 B：常态监控（1~3 人正常走动，无抬手）**
  - Pose 降频至 10fps（每 3 帧处理 1 帧），几何初筛 100% 阻断，0 次香烟推理：
  - 平均 NPU 占用设计上限：$25\text{ ms} \times 10\text{ fps} = 250\text{ ms/s} \approx$ **$25\%$**，最终以 RK3568 实测为准。
- **场景 C：疑似吸烟（1 人持续吸烟动作）**
  - Pose 10fps（设计预算 $250\text{ ms/s}$）+ 香烟细检触发（受 250ms 冷却限制，单秒最多 4 次，每次设计预算 $14.8\text{ ms}$，计 $59.2\text{ ms/s}$）：
  - 综合 NPU 占用设计上限：$250 + 59.2 = 309.2\text{ ms/s} \approx$ **$30.9\%$**；香烟模型时延同样需要真机基准确认。
- **结论**：按当前设计预算，系统在极限活动状态下的目标 NPU 占用低于 **35%**，为宿主媒体拉流、硬件 JPEG 抓拍读回预留时间裕量；最终结论必须由 RK3568 P50/P95 基准确认。

---

## 4. 端到端流水线拓扑 (Pipeline Topology)

```text
                               RTSP H.264/H.265 网络流
                                         │
                                         ▼
                         [ Heimdall MPP 硬件解码器 ]
                                         │
                        FrameRef (NV12 DMA-BUF fd, 1080P)
                                         │
                   ┌─────────────────────┴─────────────────────┐
                   ▼                                           ▼
       [ 宿主 MotionGate 门控 ]                     [ 保留原帧物理显存句柄 ]
       (轻量 Y 分量背景差分)                                    │
          │             │ (无运动且非保活)                      │
          │             └──────────────► [ 跳过本帧 (0 NPU) ]   │
          ▼ (有效运动)                                         │
       [ 帧速率节流步长 (1/3 抽帧) ]                            │
          │             │ (非采样帧)                            │
          │             └──────────────► [ ByteTrack 卡尔曼外推]│
          ▼ (采样帧)                                           │
 ┌──────────────────────────────────────────────────────────┐  │
 │ 阶段 1：RGA2 硬件全图 Letterbox                           │  │
 │  - 源: 原图 NV12 DMA-BUF                                 │  │
 │  - 操作: NV12 -> RGB888, 保持 16:9 内容等比缩放到 640x360，再上下各补 12px (16 字节对齐) │  │
 │  - 目标: Pose 输入 Scratchpad DMA-BUF (640x384x3)        │  │
 └────────────────────────────┬─────────────────────────────┘  │
                              ▼                                │
 ┌──────────────────────────────────────────────────────────┐  │
 │ 阶段 2：RKNN YOLOv8n-Pose INT8 单核推理                  │  │
 │  - 输入: 640x384；输出 9 分支纯卷积张量                  │  │
 └────────────────────────────┬─────────────────────────────┘  │
                              ▼                                │
 ┌──────────────────────────────────────────────────────────┐  │
 │ 阶段 3：ARM NEON 向量化后处理 + 安全 ByteTrack 跟踪      │  │
 │  - 解析人体框与 17 关键点                                │  │
 │  - 卡尔曼航迹关联，深拷贝关键点至 STrack 实体内部        │  │
 └────────────────────────────┬─────────────────────────────┘  │
                              ▼                                │
 ┌──────────────────────────────────────────────────────────┐  │
 │ 阶段 4：空间几何先验初筛 (Heuristic Filter)              │  │
 │  - 计算双肘内角 < 55° || 手腕口鼻相对距离 < 0.8         │  │
 └────────────────────────────┬─────────────────────────────┘  │
               ┌──────────────┴──────────────┐                 │
               ▼ (姿态可疑)                  ▼ (正常姿态)      │
 ┌───────────────────────────┐ ┌─────────────────────────────┐ │
 │ 阶段 5：自适应嘴部 ROI 计算 │ │ 目标积分衰减:             │ │
 │  - 以鼻关键点为中心扩展     │ │ `score = max(0, S - 1)`   │ │
 │  - 边界 clamp 与 16 字节对齐│ └───────────────────────────┘ │
 └─────────────┬─────────────┘                                 │
               │                                               │
               ▼ (ROI 矩形坐标 [rx1, ry1, rx2, ry2])           │
 ┌──────────────────────────────────────────────────────────┐  │
 │ 阶段 6：RGA2 硬件 ROI 直接裁切与缩放                     │◄─┘
 │  - 源: 原图 NV12 DMA-BUF (依据 ROI 坐标直接切取)         │
 │  - 操作: 裁切 + 缩放到 416x416 RGB888 (16 字节 Stride)   │
 │  - 目标: Cigarette 输入 Scratchpad DMA-BUF (416x416x3)   │
 └────────────────────────────┬─────────────────────────────┘
                              ▼
 ┌──────────────────────────────────────────────────────────┐
 │ 阶段 7：RKNN YOLOv8n-Cigarette INT8 单核细检             │
 │  - 耗时: ~14.8ms, 输出 6 分支纯卷积张量                  │
 └────────────────────────────┬─────────────────────────────┘
                              ▼
 ┌──────────────────────────────────────────────────────────┐
 │ 阶段 8：NEON 置信度解析 + 时序积分状态机                 │
 │  - 检出香烟: score = min(100, score + 10)                │
 │  - 未检出:   score = max(0, score - 1)                   │
 │  - Score >= 50: 触发 Smoking 行为违规告警                │
 └────────────────────────────┬─────────────────────────────┘
                              ▼
 ┌──────────────────────────────────────────────────────────┐
 │ 阶段 9：ResultEmitter 发射告警与证据抓拍                 │
 │  - 发射标准规范 JSON 目标框 (label: "smoking")           │
 │  - 触发 snapshot_readback_path 异步请求全景图与特写图    │
 └──────────────────────────────────────────────────────────┘
```

---

## 5. 算法数学模型与判定几何学 (Mathematical Formulation)

### 5.1 空间几何初筛数学模型

骨骼拓扑索引基于 COCO-17 标准：
- 鼻子：$P_0 = (x_0, y_0)$
- 躯干参考点：左肩 $P_5$、右肩 $P_6$、左髋 $P_{11}$、右髋 $P_{12}$
- 手臂三关节：
  - 左臂：肩 $P_5$、肘 $P_7$、腕 $P_9$
  - 右臂：肩 $P_6$、肘 $P_8$、腕 $P_{10}$

#### 1. 手肘屈曲角度（Elbow Flexion Angle）
定义肩到肘向量 $\vec{v}_{se} = P_{\text{shoulder}} - P_{\text{elbow}}$，腕到肘向量 $\vec{v}_{we} = P_{\text{wrist}} - P_{\text{elbow}}$。
利用向量点积计算夹角 $\theta$：
$$\cos\theta = \frac{\vec{v}_{se} \cdot \vec{v}_{we}}{\|\vec{v}_{se}\|_2 \|\vec{v}_{we}\|_2 + \epsilon}$$
$$\theta_{\text{elbow}} = \arccos\left(\text{clamp}(\cos\theta, -1.0, 1.0)\right) \times \frac{180^\circ}{\pi}$$
其中 $\epsilon = 10^{-5}$ 用于防止两点重合时除以零。**判定门限**：$\theta_{\text{elbow}} < 55^\circ$。

#### 2. 手部到口鼻相对归一化距离（Torso-Normalized Distance）
为消除监控摄像头视场角（FOV）与人体远近造成的绝对像素尺度影响，采用**躯干基准长度**进行归一化：
$$L_{\text{torso}} = \|P_{\text{shoulder}} - P_{\text{hip}}\|_2$$
$$D_{\text{rel}} = \frac{\|P_{\text{nose}} - P_{\text{wrist}}\|_2}{\max(L_{\text{torso}}, \epsilon)}$$
**判定门限**：$D_{\text{rel}} < 0.8$。

**综合疑似吸烟动作逻辑**：
$$\text{IsSuspicious} = (\theta_{L} < 55^\circ \lor D_{\text{rel}, L} < 0.8) \lor (\theta_{R} < 55^\circ \lor D_{\text{rel}, R} < 0.8)$$

### 5.2 上半身 ROI 自适应截取模型

为了完整囊括面部、嘴唇、下巴、手势握持动作及烟雾扩散区域，根据人体检测框外接矩形尺寸 $(W_{\text{box}}, H_{\text{box}})$ 自适应推导截取半径 $R$：
$$R = \text{clamp}\left(\max\left(0.55 \times H_{\text{box}}, \; 0.85 \times W_{\text{box}}, \; 80\right), \; 80, \; \frac{\min(W_{\text{frame}}, H_{\text{frame}})}{2}\right)$$

以鼻关键点 $P_0(x_0, y_0)$ 为锚点，施加向下垂直偏移以涵盖手部与胸腔：
$$C_x = x_0, \quad C_y = y_0 + 0.1 \times R$$
$$X_{\min} = \text{round\_down\_16}\left(\max(0, C_x - R)\right)$$
$$Y_{\min} = \max(0, C_y - R)$$
$$X_{\max} = \min(W_{\text{frame}}, X_{\min} + 2R)$$
$$Y_{\max} = \min(H_{\text{frame}}, Y_{\min} + 2R)$$
其中 `round_down_16(v)` 强制将横向起始坐标向下对齐至 16 的整数倍，以严格满足 RGA2 硬件搬运要求。

### 5.3 时序平滑状态机转移方程

为彻底消除由于打火机反光、进食、抓挠面部造成的单帧闪烁虚警，状态机基于目标 `track_id` 维护时序积分 $S_t \in [0, 100]$：

$$S_t = \begin{cases} 
\min(100, S_{t-1} + 10), & \text{if 检出香烟目标 (置信度 } \ge C_{\text{cgr\_thresh}}) \\
\max(0, S_{t-1} - 1), & \text{if 动作疑似但未检出香烟} \\
\max(0, S_{t-1} - 2), & \text{if 姿态正常 (加速衰减)}
\end{cases}$$

**状态转移判定**：
- **`Smoking` (确诊告警)**：当 $S_t \ge 50$ 时成立，发射业务告警；
- **`Suspicious` (疑似预警)**：当 $0 < S_t < 50$ 或处于疑似动作期间；
- **`Normal` (常态恢复)**：当 $S_t == 0$ 时复位。

---

## 6. 算法包工程实现架构 (`algo-packages`)

算法包物理路径：`algo-packages/rknn/rk3568/smoking_detection/`。

### 6.1 目录结构组织

```text
algo-packages/rknn/rk3568/smoking_detection/
├── Cargo.toml                  # 独立插件编译清单 (依赖 algo-sdk)
├── manifest.json               # 算法元信息与资源画像
├── config.schema.json          # 运行时动态参数 JSON Schema
├── Makefile                    # 交叉编译与打包指令
├── model/
│   ├── yolov8n-pose_640x384_rk3568_i8.rknn # 9 输出无头姿态模型 (640x384)
│   └── cigarette_yolov8n_416_rk3568_i8.rknn # 6 输出无头香烟模型 (416x416)
└── src/
    ├── lib.rs                  # 导出 export_algo! C ABI 符号
    ├── plugin.rs               # AlgoPlugin 生命周期实现
    ├── config.rs               # 实例级配置参数 (InstanceConfig)
    ├── rga_pipeline.rs         # RGA2 硬件 Letterbox 与动态 ROI 切图引擎
    ├── pose_decoder.rs         # NEON 向量化 YOLOv8-Pose 后处理
    ├── cigarette_decoder.rs    # NEON 向量化 Cigarette 后处理
    ├── tracker.rs              # 线程安全且关键点内聚的 ByteTrack
    └── state_machine.rs        # 时序平滑积分状态机
```

### 6.2 算法清单元数据 (`manifest.json`)

```json
{
  "manifest_version": 1,
  "algorithm_id": "smoking_detection",
  "version": "1.0.0",
  "name": "Smoking Behavior Detection (RK3568 RKNN)",
  "description": "Cascade smoking detection powered by YOLOv8n-pose, RGA2 hardware crop, and YOLOv8n-cigarette headless models on Rockchip RK3568.",
  "algorithm_type": "behavior_detection",
  "alarm_type_id": "smoking_violation",
  "platform_id": "linux-rknn",
  "min_adapter_version": "1.0.0",
  "runtime_constraints": {
    "target_soc": "rk3568",
    "required_drivers": {
      "rknpu": ">=0.9.2",
      "rga": ">=2.0.0"
    }
  },
  "resource_profile": {
    "min_free_memory_mb": 96,
    "npu_budget_permille": 400,
    "fps_tiers": [
      { "fps": 5,  "units": 80 },
      { "fps": 10, "units": 150 },
      { "fps": 15, "units": 220 }
    ]
  },
  "self_test": {
    "timeout_ms": 10000,
    "input_mode": "test_image"
  }
}
```

### 6.3 插件运行时配置 (`config.rs`)

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceConfig {
    /// 人体检测置信度阈值 (默认 0.40)
    #[serde(default = "default_person_conf")]
    pub person_conf: f32,

    /// 香烟精细检测置信度阈值 (默认 0.25)
    #[serde(default = "default_cig_conf")]
    pub cig_conf: f32,

    /// 状态机触发吸烟判定的累积分数门限 (默认 50)
    #[serde(default = "default_smoking_thresh")]
    pub smoking_thresh: u32,

    /// 手肘弯曲角度门限 (度, 默认 55.0)
    #[serde(default = "default_elbow_angle_thresh")]
    pub elbow_angle_thresh: f32,

    /// 手腕到口鼻归一化躯干距离门限 (默认 0.8)
    #[serde(default = "default_wrist_dist_thresh")]
    pub wrist_dist_thresh: f32,

    /// 姿态推理抽帧步长 (默认 3，即每 3 帧推理 1 帧姿态)
    #[serde(default = "default_pose_stride")]
    pub pose_stride: usize,

    /// 香烟细检最小冷却时间间隔 (毫秒, 默认 250ms)
    #[serde(default = "default_cig_cooldown_ms")]
    pub cig_cooldown_ms: u64,
}

fn default_person_conf() -> f32 { 0.40 }
fn default_cig_conf() -> f32 { 0.25 }
fn default_smoking_thresh() -> u32 { 50 }
fn default_elbow_angle_thresh() -> f32 { 55.0 }
fn default_wrist_dist_thresh() -> f32 { 0.80 }
fn default_pose_stride() -> usize { 3 }
fn default_cig_cooldown_ms() -> u64 { 250 }

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            person_conf: default_person_conf(),
            cig_conf: default_cig_conf(),
            smoking_thresh: default_smoking_thresh(),
            elbow_angle_thresh: default_elbow_angle_thresh(),
            wrist_dist_thresh: default_wrist_dist_thresh(),
            pose_stride: default_pose_stride(),
            cig_cooldown_ms: default_cig_cooldown_ms(),
        }
    }
}
```

### 6.4 解决“张冠李戴”：内聚关键点的安全航迹结构 (`tracker.rs`)

```rust
use algo_sdk::math::NormBox;

pub const NUM_KEYPOINTS: usize = 17;

#[derive(Debug, Clone, Copy)]
pub struct Keypoint {
    pub x: f32,
    pub y: f32,
    pub score: f32,
}

/// 针对吸烟行为优化的内聚性航迹结构体
#[derive(Debug, Clone)]
pub struct SmokingTracklet {
    pub track_id: u64,
    pub bbox: NormBox,
    pub score: f32,
    /// 必须内聚关键点与所属源帧号，杜绝数组索引跨帧错位
    pub keypoints: Option<[Keypoint; NUM_KEYPOINTS]>,
    pub last_kpt_frame_id: u64,
    pub last_cig_check_pts_ms: u64,
    pub smoking_score: u32,
}

impl SmokingTracklet {
    /// 验证当前关键点是否与当前分析帧同步
    pub fn is_kpt_fresh(&self, current_frame_id: u64, max_drift_frames: u64) -> bool {
        self.keypoints.is_some() && (current_frame_id.saturating_sub(self.last_kpt_frame_id) <= max_drift_frames)
    }

    /// 当目标失配或关键点过期时强制解绑姿态
    pub fn invalidate_kpt(&mut self) {
        self.keypoints = None;
    }
}
```

### 6.5 RGA2 硬件双路缓冲区池与动态裁切 (`rga_pipeline.rs`)

针对 RK3568 的 RGA2 硬件特性，分配专用的 DMA-BUF Scratchpad 缓冲区池，杜绝逐帧创建与内存泄漏：

```rust
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;

pub struct SmokingRgaPipeline {
    /// 全图 640x384x3 预分配 DMA-BUF (Pose 模型输入)
    pose_input_buf: algo_sdk::cv::buffer::CvBuffer,
    /// 局部 416x416x3 预分配 DMA-BUF (Cigarette 模型输入)
    cig_input_buf: algo_sdk::cv::buffer::CvBuffer,
}

impl SmokingRgaPipeline {
    pub fn new() -> Result<Self, AlgoError> {
        // 在初始化阶段由系统 DMA32 堆分配 2 块物理连续内存
        let pose_input_buf = algo_sdk::cv::buffer::CvBuffer::alloc_dmabuf(640, 384, 3)?;
        let cig_input_buf = algo_sdk::cv::buffer::CvBuffer::alloc_dmabuf(416, 416, 3)?;
        Ok(Self {
            pose_input_buf,
            cig_input_buf,
        })
    }

    /// 执行全图 Letterbox 缩放 (原图 NV12 -> 640x384 RGB888)
    pub fn process_pose_letterbox(
        &mut self,
        frame: &SafeFrame<'_>,
    ) -> Result<&algo_sdk::cv::buffer::CvBuffer, AlgoError> {
        // 调用 librga imcrop/imresize 硬件加速接口
        // 确保输入符合 16 字节 Stride 对齐
        // ... (RGA2 硬件级批处理，写入 self.pose_input_buf)
        Ok(&self.pose_input_buf)
    }

    /// 直接从原帧 NV12 DMA-BUF 中按 ROI 坐标执行硬件级局部抠图并缩放到 416x416
    pub fn process_roi_crop(
        &mut self,
        frame: &SafeFrame<'_>,
        roi_rect: (u32, u32, u32, u32), // (x1, y1, x2, y2)
    ) -> Result<&algo_sdk::cv::buffer::CvBuffer, AlgoError> {
        let (rx1, ry1, rx2, ry2) = roi_rect;
        // 校验 ROI 坐标有效性
        if rx2 <= rx1 + 16 || ry2 <= ry1 + 16 {
            return Err(AlgoError::InvalidArg("ROI 尺寸过小，无法满足 RGA2 处理下限".into()));
        }

        // 利用 RGA2 IM2D 接口执行原图物理截取与缩放
        // 零 CPU memcpy，零 CPU 插值
        // ... (写入 self.cig_input_buf)
        Ok(&self.cig_input_buf)
    }
}
```

---

## 7. 契约符合性与告警发射规范

### 7.1 结构化检测结果与告警发射契约

严格遵循 `detection-alarm-contract.md` 契约规范。当且仅当目标状态机判定为 **`Smoking`**（累积分数 $\ge 50$）时，向宿主发射检测目标与告警信息：

```json
{
  "schema_version": 1,
  "objects": [
    {
      "class_id": 0,
      "label": "smoking",
      "confidence": 0.88,
      "bbox": [0.3241, 0.1523, 0.6124, 0.7854]
    }
  ]
}
```

- **坐标体系**：对角两点式归一化浮点数组 `[x1, y1, x2, y2]`，数值严格落在 $[0.0, 1.0]$ 区间；
- **目标标签**：稳定输出 `"smoking"`，禁止伪装为 `"person"`；
- **证据链抓拍**：由宿主 Pipeline 根据触发规则生成 `eventId`，并自动调度 `snapshot_readback_path` 触发原帧的高清全景图与目标特写抓拍。

---

## 8. 工业级鲁棒性与异常闭环设计 (Robustness & Fault Tolerance)

### 8.1 并发过载阻断（Multi-Target Throttling）
在公共区域，若画面中出现 3 人以上同时做出疑似吸烟动作（如多人群体进食、整理衣领），若为每个人串行触发香烟细检，NPU 将面临算力雪崩：
- **最大细检并发配额**：单帧内最多允许针对 **Top-1 置信度最高**的疑似目标触发香烟细检（`max_concurrent_cig_checks = 1`）；
- 其余疑似目标在当前帧沿用上一帧状态机分数，并在后续帧通过时间片轮转（Round-Robin）调度细检，强行保证单帧推理总时延 $\le 50\text{ ms}$。

### 8.2 硬件失效降级与断路器 (`FailureTracker`)
- **RGA2 失败追踪**：若由于驱动崩溃或显存对齐异常导致 RGA2 硬件调用失败，`FailureTracker` 记录连续失败次数；达到阈值（30 帧）时主动向宿主上报 `AV_ERR_INCOMPATIBLE_FRAME`，避免静默死锁；
- **NPU 超时断路器**：单次 NPU 推理硬超时设为 $100\text{ ms}$。若 NPU 发生硬件 Hang 死，触发重启并重置当前实例会话。

### 8.3 零泄漏 RAII Guard 资源规范
- 所有打开的 DMA-BUF 文件描述符、`rga_buffer_handle_t` 以及 NPU 绑定内存，均由 Rust RAII 结构体（`Drop` 实现）统一纳管；
- 严禁在 `process()` 热路径中调用 `imimportbuf` / `imreleasebuf`，所有句柄在 `init()` 阶段完成硬件映射。

---

## 9. 性能验证与排查指令 (Benchmark & Diagnostic Steps)

在目标 RK3568 板端部署验证时，执行以下指令闭环追踪物理硬件指标：

### 9.1 NPU 负载与算子映射验证
```bash
# 1. 验证 RKNPU 驱动版本与硬件单核状态
cat /sys/kernel/debug/rknpu/version
cat /sys/kernel/debug/rknpu/driver_version

# 2. 实时监控 NPU 核心单核负载 (确保常态维持在 30%~40%，无异常 100% 阻塞)
watch -n 1 "cat /sys/kernel/debug/rknpu/load"

# 3. 验证模型算子是否有 CPU Fallback (正常情况下 NPU Op 应为 100%)
cat /sys/kernel/debug/rknpu/dump_tensor
```

### 9.2 RGA2 硬件加速器工作状态监测
```bash
# 1. 监控 RGA2 硬件负载
cat /sys/kernel/debug/rkrga/load

# 2. 观察 RGA 硬件中断触发频次 (确认每帧有中断产生，证明硬件加速介入)
watch -n 1 "cat /proc/interrupts | grep -i rga"
```

### 9.3 内存带宽与 CPU 占用追踪
```bash
# 1. 监控 DMC 动态内存控制器频率 (确认内存未受总线争用降频)
cat /sys/class/devfreq/dmc/cur_freq

# 2. 监控 4 核 A55 CPU 利用率 (整机综合 CPU 占用率应低于 25%)
top -d 1
```

---

## 10. 实施里程碑 (Milestones)

1. **M1: 模型量化与格式导出**：完成 `640×384` 切头无头 Pose 模型与 416×416 Cigarette 模型在 RKNN-Toolkit2 针对 `target_platform: rk3568` 的 INT8 量化编译，确认 0 算子回退并记录实际输出 shape；
2. **M2: C ABI 插件工程骨架构建**：在 `algo-packages/rknn/rk3568/smoking_detection/` 建立工程，通过 `algo-sdk` 接入并打通静态自检（`is_self_test`）；
3. **M3: RGA2 双路零拷贝与 NEON 向量化后处理**：实现全图 Letterbox 与动态 ROI 硬件抠图，补齐 ARMv8 NEON 向量化 DFL 运算；
4. **M4: 状态机防抖与整机联调**：接入内聚关键点的 ByteTrack 与时序平滑积分状态机，跑通真实视频流并验证 1080P 30fps 稳态运行。
