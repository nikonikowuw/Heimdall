# RK3568 真机冒烟测试报告

**测试环境**
- 设备：RK3568 EVB（`root@192.168.17.140`），Linux 5.10.226 aarch64
- 部署路径：`/opt/heimdall`，存储根 `var/data/evidence`
- 硬件：`/dev/rga`、`/dev/mpp_service`、librknnrt 就绪
- 测试码流：Mac 侧 mediamtx + ffmpeg 循环推送 720p H.264（含人脸）
  经 `rtsp://192.168.19.198:8554/smoke` 接入，避免依赖现场人员走动

**构建与部署**
```bash
make sdk-sync-rk3568 RK3568_HOST=root@192.168.17.140   # 同步 MPP/RGA/RKNN 库
make cross-rk3568                                       # aarch64-unknown-linux-gnu
# 部署：备份旧二进制 → 替换 → nohup 启动
```

---

## 验证结论

| 验证项 | 结果 | 证据 |
|--------|------|------|
| 交叉编译与部署 | 通过 | aarch64 ELF，服务正常监听 8000 |
| V20/V21 迁移 | 通过 | `recordings`、`recording_events` 表 + `cameras.recording_config` 列 |
| 通道级配置 API | 通过 | `PUT /cameras/{id}` 写入 `recordingConfig` 并即时生效 |
| 录像 Worker 启动 | 通过 | 每启用通道一个 `rec-<camera_id>` OS 线程 |
| 冷启动恢复 | 通过 | 服务重启后两个 Worker 自动重建 |
| 事件触发录像 | 通过 | 累计产出 10 条录像 |
| 前置缓冲 | 通过 | 首事件 `offset_ms=1500`，文件起始早于事件时刻 |
| 事件延长合并 | 通过 | 单文件关联 **16 个事件**，时长 54.5s |
| fMP4 合法性 | 通过 | `ffprobe` 解析正常；`ffmpeg -f null` 全量解码零错误（290 帧） |
| HTTP 播放接口 | 通过 | 200 + `video/mp4`；Range 请求返回 **206** |
| 事件反查接口 | 通过 | `by-event/recognition/{id}` 返回录像与 `offsetMs` |
| 事件类型校验 | 通过 | 非法类型返回 `40001` |
| 客户端导出路径 | 通过 | fMP4 → 标准 MP4 `-c copy` remux 成功且可播 |
| SourceReset 截断 | 通过 | `status=truncated`，截断文件仍完整可解码（50.99s） |
| **源流中断存活** | **修复后通过** | 见下方缺陷章节 |
| 性能 | 通过 | CPU 29%、RSS 127MB、RGA 34%、load 1.32（4 路并发） |

---

## 发现的缺陷（已修复）

### 僵尸消费者回收误杀管线内部消费者

**现象**：源流中断超过约 10 秒后，该会话的**分析泵与录像 Worker 永久退出**，且不会自愈；任务仍上报 `actualStatus=2`，UI 无任何异常提示。录像静默停止，必须重启服务或重新保存配置才恢复。

**故障链**
```
源流中断 → source_reset() 将全部消费者置为 recovering
    → ingestor 指数退避重试 1/4/8/16/30s
    → 退避超过 zombie_no_progress_ms（10s）
    → evict_stalled() 关闭 mailbox
    → AnalysisPump / RecordingWorker 收到 Closed 后 break，线程退出
```

**根因**：`PacketDispatcher::evict_stalled()` 对所有 `ConsumerKind` 一视同仁。

僵尸回收的设计意图是清理**断线后不再读取的预览客户端**（`HttpFlv` / `WebCodecs`），它们会自行重连。而管线内部消费者（`Analysis` / `MainStreamEvidence` / `Recording`）是长生命周期组件：中断期间只是暂时无包可读，源流恢复后由 `Replay` 自愈。将其回收会破坏管线且无人重启。

**修复**：`evict_stalled()` 仅作用于预览类消费者。

**修复后真机验证**（同一进程、同一线程，未重启）
- 中断 35 秒（远超 10 秒阈值）：无新僵尸回收日志；`mpp-dec-01a0f4e`、`rec-01a0f4ee-8e`、`gate-01a0f4ee-8` 线程全部存活
- 源流恢复后：捕获量持续增长（104 → 123），自动产出新录像（`b535ed30` 9.7s、`032cbfc0` 23.8s）
- 中断时的在途录像被正确标记 `truncated`（44.7s 素材保留且可播）

---

## 遗留事项

- 分析泵无独立健康监督：若线程因 panic 等非回收原因退出，任务状态不会更新，也无自动重启。本次修复覆盖了"源流中断"这一主因，其他死亡原因仍无兜底。
- 前端未实现（原计划 Phase 6）：录像配置 UI、告警详情回放 tab、mp4box.js 导出按钮。后端接口已全部就绪并验证。
- 冒烟测试通道（`冒烟测试通道` / `01a0f4ee-...`）及其任务、Mac 侧 mediamtx 推流为测试用途，需按需清理。
