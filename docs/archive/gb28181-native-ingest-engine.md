# Design: 国标 GB/T 28181 纯 Rust 原生接入与设备发现引擎 (GB28181 Native Ingest & Discovery Engine)

> **状态**: 已实现 (Implemented)；SIP UAS 状态机、PS 容错解复用、RTP 抖动缓冲、动态端口池、ONVIF 嗅探与前端纳管已闭环落地
> **作者**: Heimdall Engineering
> **日期**: 2026-09-13
> **关联规范**: [AGENTS.md](../../AGENTS.md)、[媒体管线](../nuwa/backend/media-pipeline.md)、[并发模型](../nuwa/backend/concurrency-guidelines.md)、[实时预览重构](./realtime-preview-overhaul.md)、[API 规范](../nuwa/backend/api-guidelines.md)、[数据库规范](../nuwa/backend/database-guidelines.md)
> **代码落地**:
>
> - `crates/media/src/gb28181/` (`Gb28181SipServer`, `PsDemuxer`, `JitterBuffer`, `PortPool`, `Gb28181Ingestor`, `scan_lan_cameras`)
> - `crates/media/src/media_ingestor.rs` & `stream_hub.rs` (接入层与连接复用)
> - `crates/db/src/` (`SysGb28181ConfigRepo`, `Gb28181DeviceRepo`, migration `V11`)
> - `crates/api/src/routes/system/gb28181.rs` (REST API 端点)
> - `web/src/features/` (`Gb28181Settings.tsx`, `BatchImportGbModal.tsx`, `LanDiscoveryModal.tsx`, `CameraModal.tsx`)

---

## 1. 背景与核心定位

### 1.1 现状与业务诉求

Heimdall 当前的媒体接入能力主要面向客户端拉流（Client-Pull）的 RTSP 协议（由 `retina_ingest` 驱动），下游衔接统一的 `StreamHub` 隔离分发与异构硬件加速管线（MPP / DVPP / VideoToolbox）。

在安防工业化落地中，**GB/T 28181（公共安全视频监控联网系统）** 是国内强制性行业标准：

1. **行业存量标准设备接入**：大量安防 IPC、热成像仪及多通道 NVR 仅提供 GB28181 主动注册与点播接口，缺乏可直接拉取的 RTSP 地址；
2. **多通道批量纳管**：单个 NVR 下挂载 4~64 个物理摄像头通道，急需支持**自动目录扫描（Catalog Query）**，一键同步通道列表，杜绝逐路配置 RTSP 的运维成本；
3. **局域网设备自发现**：边缘一体机（盒子）在工业现场开箱即用时，需支持一键“局域网扫描”，快速嗅探并纳管新接入的摄像机。

### 1.2 核心定位：Heimdall 本身即是独立的 SIP 服务端 (SIP UAS / Registrar)

**本项目不依赖任何外部第三方 SIP 代理或流媒体中间件（如 ZLMediaKit、WVP-PRO、FreeSWITCH、LiveGBS 等）。**

Heimdall 在单二进制启动时，直接在 `crates/app` 中拉起一个常驻的异步服务，在宿主机物理绑定 **5060 端口（UDP/TCP）**，将自身注册为现场局域网内的**中心 SIP 服务端（UAS / 注册服务器）**。现场所有 IPC/NVR 摄像头均将 Heimdall 识别为上级管理平台，主动向 Heimdall 发起 SIP 注册与推流。

| 机制 / 维度 | 外部进程桥接方案 (如 ZLMediaKit/WVP) | Heimdall 纯 Rust 原生服务端方案 | 架构决策理由 |
| :--- | :--- | :--- | :--- |
| **交付形态** | 多进程/多容器协同，需额外维护 SIP 进程与环境 | **单二进制原生嵌入**，无外部运行时依赖 | 严格契合 [AGENTS.md](../../AGENTS.md) 单二进制交付契约 |
| **信令与媒体开销** | 跨进程多级缓冲与两次本地 TCP 协议栈转发 | 统一异步运行时，PS 解包后 `Bytes` 直达 `StreamHub`，**0 额外拷贝** | 极致能效，满足低功耗边缘芯片资源预算 |
| **内存安全与稳定性** | 易因安防厂商私有畸变 PS 包引发 C/C++ 内存越界 Crash | **Rust 严格所有权与安全状态机**，RAII 管理端口与生命周期 | 杜绝野指针、UAF、内存泄漏与死锁隐患 |
| **异常恢复协同** | 进程间无法精确协同 Epoch，IPC 断流重连易花屏串流 | 深度接入 `StreamHub` 的 `SourceReset` 与 `GopSnapshot` 契约 | 源流断开或重连时秒级无损对齐与原子恢复 |

---

## 2. 核心架构与数据流拓扑

### 2.1 系统全局拓扑

```
                             [ 外部 IPC / NVR ]
                                 │       ▲
          1. SIP REGISTER / 心跳 │       │ 3. SIP INVITE (SDP: 动态接收端口)
          4. RTP (PS over UDP/TCP)│       │ 6. SIP BYE (闲时拆链)
                                 ▼       │
┌─────────────────────────────────────────────────────────────────────────────┐
│ Heimdall Core Runtime (crates/app 启动组装)                                 │
│                                                                             │
│ ┌────────────────────────────────────┐ ┌──────────────────────────────────┐ │
│ │        控制面：Gb28181SipServer     │ │      数据面：Gb28181Ingestor      │ │
│ │ ────────────────────────────────── │ │ ──────────────────────────────── │ │
│ │ • 5060 UDP/TCP 双栈监听器          │ │ • RAII 动态 RTP/RTCP 端口分配    │ │
│ │ • Digest MD5 鉴权状态机            │ │ • Bounded JitterBuffer 乱序重组  │ │
│ │ • XML 状态机 (Keepalive / Catalog) │ │ • MPEG-PS 容错流式解复用器       │ │
│ │ • 呼叫状态机 (Invite / Ack / Bye)   │ │ • 33-bit PTS 回环展开与时序平滑  │ │
│ └──────────────────┬─────────────────┘ └─────────────────┬────────────────┘ │
│                    │ 注册通道 / 触发点播                  │ 裸 NALU 输出     │
│                    ▼                                     ▼                  │
│ ┌─────────────────────────────────────────────────────────────────────────┐ │
│ │                     crates/media::StreamHub                             │ │
│ │ ─────────────────────────────────────────────────────────────────────── │ │
│ │ • 规范化键: "gb28181://{deviceId}/{channelId}"                          │ │
│ │ • CameraStreamSession (管理 active_viewers / ai_task_refs)              │ │
│ │ • PacketDispatcher (独立 Mailbox 隔离 + KeyframeCache + GOP Replay)     │ │
│ └─────────┬───────────────────────────────┬───────────────────────────────┘ │
└───────────┼───────────────────────────────┼─────────────────────────────────┘
            │                               │
            ▼                               ▼
┌───────────────────────┐       ┌───────────────────────┐
│     AnalysisPump      │       │ HTTP-FLV / WebCodecs  │
│  (MPP/DVPP 硬件硬解)   │       │  (前端低延迟实时预览)   │
│           │           │       └───────────────────────┘
│           ▼           │
│   RGA/VPC 硬件预处理   │
│           │           │
│           ▼           │
│    RKNN/ACL 推理      │
└───────────────────────┘
```

### 2.2 接入层抽象解耦契约

重构 `crates/media/src/stream_hub.rs`，消除对 `RetinaIngestor`（RTSP）的硬编码绑定，提取统一的 `MediaIngestor` Trait：

```rust
#[async_trait]
pub trait MediaIngestor: Send + Sync + 'static {
    /// 运行拉流主循环，由外部传入生命周期取消信号
    async fn run_loop(
        self: Arc<Self>,
        cancel_signal: Arc<AtomicBool>,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
    );
}
```

- RTSP 设备：`RetinaIngestor` 实现 `MediaIngestor`；
- 国标设备：`Gb28181Ingestor` 实现 `MediaIngestor`；
- 会话统一由 `StreamHub::ensure_ingestor_started` 按协议前缀或元数据路由分发。

---

## 3. 控制面：SIP 协议栈与信令服务设计

### 3.1 监听器架构与系统部署约束

- **网络绑定**：默认绑定 `0.0.0.0:5060`，支持 UDP 与 TCP 并行双栈；
- **Linux 权限模型约束**：
  - Linux 下 $<1024$ 端口默认受限。二进制部署时通过文件能力授权：

    ```bash
    sudo setcap 'cap_net_bind_service=+ep' /usr/local/bin/heimdall
    ```

  - Docker 容器部署时通过端口映射放行：

    ```bash
    -p 5060:5060/udp -p 5060:5060/tcp -p 30000-30500:30000-30500/udp
    ```

### 3.2 注册与 Digest 鉴权状态机

遵循 RFC 3261 与 GB/T 28181-2016 第 9.1 节规范：

```
Device (IPC/NVR)                        Heimdall (SIP Server)
      │                                           │
      ├──── 1. REGISTER (无 Authorization) ───────▶│ (生成 16 字节随机 Nonce)
      │◀─── 2. 401 Unauthorized (Digest challenge)┤
      │                                           │
      ├──── 3. REGISTER (携带 Digest response) ───▶│ (校验: MD5(HA1:nonce:HA2))
      │                                           │
      │◀─── 4. 200 OK ────────────────────────────┤ (设备状态更新为 Online)
      │                                           │
      ├──── 5. MESSAGE (Keepalive 心跳 XML) ──────▶│ (重置保活定时器)
      │◀─── 6. 200 OK ────────────────────────────┤
```

- **安全防护**：
  - Nonce 设置 300 秒有效窗口；
  - 连续 5 次认证失败锁定该 IP 300 秒，防穷举攻击；
  - 心跳超时判定：默认 3 个心跳周期（$3 \times 60\text{s} = 180\text{s}$）无响应原子更新设备为 `Offline`。

### 3.3 按需点播（INVITE）与闲时停流（BYE）

依托 `StreamHub` 现有的引用计数机制（`active_viewers` 与 `ai_task_refs`）：

1. **触发点播**：当某个通道的活跃消费者从 0 变为 1 时，`Gb28181Ingestor` 启动并通知 `Gb28181SipServer`：
   - 向 `PortPool` 申请一对空闲端口（如 RTP: `30004`，RTCP: `30005`）；
   - 生成 10 位国标 SSRC（格式为 `0` + 域编码后4位 + 5位流水号）；
   - 发送 SIP `INVITE`，SDP 声明载荷：`m=video 30004 RTP/AVP 96`，`a=rtpmap:96 PS/90000`；
   - 收到 `200 OK` 后回送 `ACK`，IPC 开始向 `30004` 推送 RTP/PS 媒体流。
2. **触发停流**：通道活跃消费者归零后，启动 10 秒 Cooldown 定时器。超时确认无新连接后发送 `cancel_signal`，`Gb28181Ingestor` 发送 SIP `BYE`，设备停流并归还端口。

### 3.4 目录树扫描（Catalog Query）与多通道批量同步

当 NVR 或多目全景相机首次注册成功，或前端主动触发“同步通道”时：

1. 向设备发送 SIP `MESSAGE`，载荷为标准 XML：

   ```xml
   <?xml version="1.0" encoding="GB2312"?>
   <Query>
     <CmdType>Catalog</CmdType>
     <SN>10001</SN>
     <DeviceID>34020000001180000001</DeviceID>
   </Query>
   ```

2. 解析设备返回的 `<DeviceList Num="N">`，提取通道国标 ID（20位）、通道名称、在线状态及 PTZ 属性；
3. 在单一 SQLite 事务中基于 `(gb28181_device_id, gb28181_channel_id)` 联合键批量 upsert `cameras` 表。

---

## 4. 数据面：RTP 传输与 MPEG-PS 流式解复用引擎

### 4.1 传输模式与乱序重组

- **TCP 被动模式（RFC 4571）**：生产环境默认推荐。每个 RTP 包前置 2 字节大端网络长度，天然规避丢包乱序；
- **UDP 模式**：兼容支持。内置容量为 64 包的 `Bounded JitterBuffer`，依据 RTP 16 位 Sequence Number 重排，超时空洞包强制弃帧并下发 `Discontinuity` 标记。

### 4.2 工业级防守型 PS Demuxer 状态机

```
                      [ 输入: 原始 RTP Payload 字节流 ]
                                     │
                                     ▼
                     [ 扫描 0x000001 起始码 (Start Code) ]
                                     │
         ┌───────────────┬───────────┴───────────┬───────────────┐
         ▼ (0xBA)        ▼ (0xBB)                ▼ (0xBC)        ▼ (0xE0..0xEF)
  [ Pack Header ]  [ System Header ]      [ PSM 映射表 ]    [ 视频 PES 包 ]
         │               │                       │               │
  解析 SCR 时间戳   提取各流频度/时长        提取 StreamType  提取 PTS/DTS
         │               │               (0x1B:H264/0x24:H265)   │
         └───────────────┼───────────────────────┘               │
                         ▼                                       ▼
                 [ 容错/跳过切片 ]                     [ 拼装 ES Elementary Stream ]
                                                                 │
                                                                 ▼
                                                    [ NALU 切分 (00 00 00 01) ]
                                                                 │
                                        ┌────────────────────────┴────────────────────────┐
                                        ▼ (SPS/PPS/VPS/IDR)                               ▼ (P/B Slice)
                                 [ 提取关键帧参数集 ]                             [ 封装 EncodedPacket ]
                                 [ 标记 is_keyframe ]                                     │
                                        │                                                 │
                                        └────────────────────────┬────────────────────────┘
                                                                 ▼
                                                 [ 注入 StreamHub::dispatcher ]
```

### 4.3 现场极端畸变防御策略

1. **缺失 PSM 时的启发式自愈探测（Heuristic Codec Sniffing）**：
   - 现场大量低端 IPC 不发送 `0xBC`（PSM）。解复用器内置特征嗅探：跳过 PES Header，在载荷前 32 字节内搜索 `0x00000001`：
     - 若后继字节为 `0x67`（SPS）或 `0x65`（IDR），自动识别并固化为 `CodecType::H264`；
     - 若后继字节高 6 位为 `0x40`（VPS）或 `0x42`（SPS），自动固化为 `CodecType::H265`；
2. **33 位 PTS 回环溢出防护（PTS Unwrapping）**：
   - MPEG-PS 的 90kHz PTS 仅有 33 位（在 $2^{33} \approx 8.589 \times 10^9$ 刻度，即约 26.5 小时后自动溢出归零）；
   - 内置 `PtsUnwrapper` 状态机，相邻帧反向差值超过 $2^{32}$ 时自动累加 $2^{33}$ 偏移，平滑输出单调毫秒时间戳；
3. **起始码重同步（Start Code Resynchronization）**：
   - 报文畸变或遇到未知扩展头时，不中断连接，逐字节滑动搜寻下一个 `0x000001BA` 或 `0x000001E0`，跳过破损切片并在下一关键帧恢复解码。

### 4.4 纯视频管线与音频强隔离

严格遵循 Nuwa [媒体管线](../nuwa/backend/media-pipeline.md) 的硬件隔离红线：

- 解析出 `0xC0..=0xDF`（音频 PES）时，封装为 `StreamTag::Audio` 进入总线；
- `AnalysisPump`（AI 硬解）与 `MainStreamRingBuffer` 在订阅时**强制过滤音频包**，确保只有纯 Annex-B 视频 NALU 送往 MPP / DVPP / VideoToolbox 底层硬件，杜绝冲撞硬件内核。

---

## 5. 局域网物理设备主动嗅探扩展 (Active LAN Discovery)

解决全新摄像头开箱插线后免配置快速纳管问题：

- **ONVIF WS-Discovery 组播引擎**：基于 `tokio::net::UdpSocket` 向 `239.255.255.250:3702` 发送 `Probe` 广播，3 秒内平滑收口，解析摄像机 IP、MAC、厂商与 ONVIF 接口；
- **RTSP 快速探活**：对目标网段并发探测 554 端口，提取有效流路径。

---

## 6. 前端交互与信息架构（IA）设计规范

前端严格遵循工业级规范：**严禁使用 Emoji**，统一使用 Lucide 矢量图标，Tailwind CSS 变量 Token，支持 i18n 国际化。

### 6.1 核心信息架构（IA）分离准则：拒绝反模式

安防系统中最常见的反模式（Anti-Pattern）是将不同协议接入的摄像头分散在各自的协议设置页中，导致用户在配 AI 任务、查看实时画面和回放时被迫在不同页面之间频繁切换。

本设计严格贯彻**“底盘基础设施与业务资产彻底解耦”**的原则：

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ 1. 系统设置页 (/settings?tab=gb28181) ──▶ 【纯基础设施底盘 (Infrastructure)】│
│    • 面向角色：系统管理员 / 网络运维人员                                     │
│    • 核心职责：只配置服务本身（我是不是 SIP 服务？我的端口、密码、RTP 端口池是啥）│
│    • 设备展现：★ 绝不在此管理具体摄像头！仅展示底层服务健康度与连接统计      │
└─────────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────────┐
│ 2. 摄像头管理页 (/cameras)            ──▶ 【全协议统一资产中心 (Asset Pool)】│
│    • 面向角色：安防运营人员 / 算法布防工程师                                 │
│    • 核心职责：全量摄像头的全生命周期管理（名称、预览、分辨率、AI任务、录像、删除）│
│    • 协议平等：不论是 RTSP 填 URL 进来的，还是 GB28181 国标注册进来的，       │
│      在业务层全部平等地展现为一个个标准的【Camera 卡片 / 列表行】！            │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

### 6.2 系统设置页设计：新增「国标服务」Tab (`/settings?tab=gb28181`)

在 `web/src/features/system/SettingsPage.tsx` 的左侧导航栏新增 `gb28181` 配置项，对应配置组件 `Gb28181Settings.tsx`。页面定位为**纯粹的服务底座配置，不堆叠任何摄像头业务列表**：

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│  系统设置 > 国标 GB28181 服务配置                                                 │
├──────────────────────────────────────────────────────────────────────────────────┤
│                                                                                  │
│  ┌─ 服务运行状态 (Service Health) ────────────────────────────────────────────┐  │
│  │  [● 运行中 (Online)]  监听端口: 5060/UDP+TCP  已握手会话: 2 个物理设备     │  │
│  └────────────────────────────────────────────────────────────────────────────┘  │
│                                                                                  │
│  ┌─ 本机 SIP 平台参数 (供摄像机 Web 后台填写) ────────────────────────────────┐  │
│  │  平台国标编码 (SIP ID):  [ 34020000002000000001 ]  [ 复制 ]                │  │
│  │  SIP 服务器域 (Domain):  [ 3402000000           ]  [ 复制 ]                │  │
│  │  SIP 监听端口 (Port):    [ 5060                 ]                         │  │
│  │  设备接入密码 (Password):[ ••••••••             ]  [ 显示 ]                │  │
│  │  心跳超时判定:           [ 180 秒               ]                         │  │
│  └────────────────────────────────────────────────────────────────────────────┘  │
│                                                                                  │
│  ┌─ 媒体流端口池 (RTP Media Port Pool) ───────────────────────────────────────┐  │
│  │  端口范围: 起始 [ 30000 ] ~ 结束 [ 30500 ]   (最大支持并发流: 250 路)        │  │
│  │  首选传输协议:  (●) TCP 被动模式 (推荐，抗丢包)     ( ) UDP 模式            │  │
│  └────────────────────────────────────────────────────────────────────────────┘  │
│                                                                                  │
│  ┌─ 自动化策略 ───────────────────────────────────────────────────────────────┐  │
│  │  [✓] 设备注册上线后自动触发目录树同步 (Auto Catalog Sync)                    │  │
│  │  [✓] 离线设备保留已有通道元数据 (避免已布防 AI 任务因网络抖动中断)          │  │
│  └────────────────────────────────────────────────────────────────────────────┘  │
│                                                                                  │
│                          [ 保存配置 ]    [ 导出 IPC 对接指导卡 ]                  │
└──────────────────────────────────────────────────────────────────────────────────┘
```

---

### 6.3 平台端与摄像机端参数映射全景对照（IPC 现场配置指南）

用户登录海康、大华、宇视等 IPC Web 管理后台时，参数填写映射关系如下：

| 参数名称 | Heimdall 本机平台参数（在设置页配置并复制） | 摄像机端（IPC / NVR Web 管理后台填写） |
| :--- | :--- | :--- |
| **服务器地址** | Heimdall 宿主机网卡 IP（如 `192.168.1.100`） | **SIP 服务器 IP**：填 Heimdall 实际 IP |
| **服务端口** | 默认 `5060` | **SIP 服务器端口**：`5060` |
| **平台国标 ID** | `34020000002000000001`（20位，前10位为域） | **SIP 服务器 ID**：填相同 20 位编码 |
| **平台域** | `3402000000`（10位行政区划代码） | **SIP 服务器域**：填相同 10 位编码 |
| **接入密码** | 平台设置统一密码（如 `Admin@123`） | **密码 / 接入密码**：填相同密码 |
| **设备自身编码** | 无须预先配置，IPC 注册时自动识别 | **设备国标编号 (DeviceID)**：20位（IPC为 `132`，NVR为 `118`） |
| **心跳周期** | 超时 180 秒判定断线 | **心跳间隔**：建议设置为 `60` 秒 |
| **传输模式** | 支持 TCP 被动 / UDP | **流传输方式**：首选 `TCP` 或 `TCP被动` |

---

### 6.4 统一摄像头资产管理页 (`/cameras`) 与批量纳管设计

所有被纳管的摄像头统一聚集在 `/cameras` 页面，针对 GB28181“一台 NVR 注册后上报 16~64 个通道”的安防特性，交互设计如下：

#### 1. 列表呈现与协议徽标（Badge）

所有摄像头在网格或列表中统一呈现，卡片打上协议徽标：

- `[RTSP]`：自填 URL 接入设备；
- `[GB28181]`：国标注册设备，副标题显示物理上级设备（如 `海康16路NVR - 通道01`）。
- 顶部支持按协议类型快速过滤：`[全部 (18)]`、`[RTSP (2)]`、`[GB28181 (16)]`。

#### 2. 未纳管通道发现条（Inbox Banner）与批量导入抽屉

当 NVR 首次注册成功并通过 Catalog 扫描出多通道时，`/cameras` 顶部自动展示非阻塞的待纳管横幅：

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│ [📡] 检测到新注册的国标设备「海康威视 16路 NVR」，发现 16 个可用通道             │
│      当前已纳管: 0 / 16 通道                       [ 批量纳管通道 ]  [ 忽略 ]    │
└──────────────────────────────────────────────────────────────────────────────────┘
```

点击 **[ 批量纳管通道 ]** 打开抽屉（Drawer），支持：

- 勾选需要用于 AI 分析与预览的通道（支持全选/反选）；
- 批量设置初始码流模式（`auto` / `main` / `sub`）；
- 点击“一键导入资产池”，选中的通道立即批量落入 `cameras` 表，进入主网格开始工作。

#### 3. 摄像头接入弹窗 (`CameraModal.tsx`)

手动添加摄像头时，顶部提供协议切换：

- **RTSP 模式**：输入主/子码流 RTSP 地址（原有逻辑）；
- **GB28181 模式**：通过树状下拉选择器（Tree Select）在已注册的设备与通道中直接勾选，并提供“一键扫描局域网新摄像头（ONVIF）”快捷入口。

---

## 7. 数据契约与接口设计

### 7.1 数据库模式扩展 (SQLite / SeaORM)

```sql
-- 1. 复用 cameras 表已预留字段:
-- protocol: 'rtsp' | 'gb28181'
-- gb28181_device_id: TEXT
-- gb28181_channel_id: TEXT

-- 2. 新增系统级国标配置表 (单行约束单例)
CREATE TABLE IF NOT EXISTS sys_gb28181_config (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    sip_id TEXT NOT NULL DEFAULT '34020000002000000001',
    sip_domain TEXT NOT NULL DEFAULT '3402000000',
    sip_port INTEGER NOT NULL DEFAULT 5060,
    sip_password TEXT NOT NULL DEFAULT 'admin123',
    rtp_port_range_start INTEGER NOT NULL DEFAULT 30000,
    rtp_port_range_end INTEGER NOT NULL DEFAULT 30500,
    auto_catalog_sync INTEGER NOT NULL DEFAULT 1,
    heartbeat_timeout_sec INTEGER NOT NULL DEFAULT 180,
    updated_at_ms INTEGER NOT NULL
);

-- 3. 新增国标设备注册信息表 (NVR / 独立 IPC)
CREATE TABLE IF NOT EXISTS gb28181_devices (
    device_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    ip_addr TEXT NOT NULL,
    sip_port INTEGER NOT NULL,
    transport TEXT NOT NULL DEFAULT 'udp',
    status TEXT NOT NULL DEFAULT 'online',
    channel_count INTEGER NOT NULL DEFAULT 0,
    last_keepalive_ms INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
```

### 7.2 RESTful API 路由定义

遵循 [API 规范](../nuwa/backend/api-guidelines.md)（根信封 `{ code: 0, message: "success", data, timestamp }`，13位毫秒，camelCase）：

| 方法 | 路径 | 说明 |
| :--- | :--- | :--- |
| `GET` | `/api/v1/system/gb28181/config` | 获取本机 SIP 服务配置与运行健康指标 |
| `PUT` | `/api/v1/system/gb28181/config` | 更新本机 SIP 服务配置（动态热重载端口与密码） |
| `GET` | `/api/v1/system/gb28181/devices` | 获取当前已注册的 GB28181 设备树及下属通道 |
| `POST` | `/api/v1/system/gb28181/devices/{deviceId}/sync` | 主动向目标设备触发 Catalog 目录扫描与批量入库 |
| `GET` | `/api/v1/system/discovery/scan` | 触发局域网摄像头主动嗅探（ONVIF + RTSP 探活） |

---

## 8. 资源约束、异常防御与并发模型

### 8.1 物理资源严格受界 (Bounded Budgets)

1. **RTP 端口池 RAII 租约**：
   - 端口分配返回 `Arc<PortLease>`，任务完成、异常中断或 Drop 时自动回收，物理杜绝端口泄漏；
2. **防僵尸 Dialog 机制**：
   - SIP 点播（INVITE）设置 15 秒协商硬超时；未收到 `200 OK` 自动注销事务并释放已分配的端口；
3. **内存单包上界**：
   - 单个 PS 帧缓冲上限为 10MB，超限立即告警并重置状态机，防畸变包造成 OOM。

### 8.2 并发与异步红线

遵循 [并发模型](../nuwa/backend/concurrency-guidelines.md)：

- **Tokio Worker 零阻塞**：SIP 消息处理与轻量 PS 解析在 Tokio 协程中异步流转；所有视频解码与推理严格隔离在专有的硬件专用线程中；
- **锁粒度控制**：设备路由表采用 `parking_lot::RwLock`，持锁期间严禁跨越 `.await` 或网络 I/O。

---

## 9. 分阶段实施与验证计划

### 阶段一：PS 解封装器与 RTP 乱序重组开发（Data Plane）

- [ ] 在 `crates/media` 中实现 `gb28181/ps` 模块（`PsDemuxer`、`PtsUnwrapper`、`JitterBuffer`）；
- [ ] 导入海康/大华真实国标 PCAP 数据包进行单元测试，断言平滑输出连续 Annex-B NALU 与单调 PTS。

### 阶段二：SIP 原生服务端与设备会话表（Control Plane）

- [ ] 在 `crates/media` 中实现 `gb28181/sip` 模块，绑定 5060 监听器；
- [ ] 实现 Digest MD5 鉴权、Keepalive 心跳与 Catalog XML 通道批量入库；
- [ ] 编写模拟 IPC 测试脚本，验证注册、保活与点播停流事务。

### 阶段三：StreamHub 解耦与 AI 硬解链路联调

- [ ] 重构 `stream_hub.rs` 提取 `MediaIngestor` Trait，接入 `Gb28181Ingestor`；
- [ ] 在 RK3588 (MPP) / 华为昇腾 (DVPP) 上进行 24 小时常驻解码与 AI 分析压测，验证纯视频管线无音频污染。

### 阶段四：前端系统设置与摄像头管理接入

- [ ] 在 Web 控制台 `SettingsPage` 实现「国标服务」配置 Tab 与状态看板；
- [ ] 在 `CameraModal` 中实现 GB28181 协议切换、设备通道树选择与局域网一键嗅探向导。
