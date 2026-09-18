# RK3568 人脸识别算法包 (EdgeFace Face Recognition)

基于 Rockchip RK3568 平台的工业级人脸识别算法包，集成 **SCRFD-2.5G BNKPS** 人脸专用检测器（含高精度 5 点关键点）与 **EdgeFace-S** 512 维特征提取器。

## 平台契约

- **平台**: Rockchip RK3568 (NPU 1.0 TOPS @ INT8)
- **操作系统**: Linux aarch64 (Debian / Ubuntu / Buildroot)
- **运行时支持**: librknnrt.so >= 1.5.0
- **算法 ABI**: 女娲标准算法插件 C ABI (`av_algo_get_abi`)

## 模型清单

- `model/yolov8n-640x384-rk3568.rknn`: YOLOv8n COCO 人体检测模型（640×384，9 个 NCHW 输出，类别 0 为 person）
- `model/scrfd_2.5g_bnkps_640x384_rk3568_mixed.rknn`: 视频流人脸检测与 5 点关键点模型（640×384 混合精度，SCRFD-2.5G BNKPS 骨干）
- `model/scrfd_2.5g_bnkps_640x640_rk3568_mixed.rknn`: 底库注册人脸检测与 5 点定位模型（640×640 混合精度）
- `model/edgeface_s_gamma_05_rk3568_fp16.rknn`: 512 维人脸特征提取模型（112×112 FP16，EdgeFace-S 骨干）

## 算法核心流水线

本算法包提供两条能力与全套端侧优化流水线：

- `instance_process()`：从同一原始帧生成一份 640×384 Letterbox RGB 输入，同时供人脸和人体检测模型使用；DMA-BUF 路径只保留一份 RGA 输出，再分别绑定两个 RKNN session，结合空间几何关联挂载与特写虚拟躯干 (Pseudo-body) 保底，由纯 Rust `ByteTracker` 维护稳定航迹。
- `BestShotManager` 动态抓拍与时域超球面特征融合：
  1. **低频算力门控**：初次入镜合格人脸立即触发特征提取；后续帧仅在姿态质量显著改善（$\Delta Q > 0.08$）或满足采样间隔（$\ge 6$ 帧）且未达上限（最多 4 帧）时触发 EdgeFace，彻底避免 RK3568 1.0 TOPS 算力逐帧空转；
  2. **超球面加权聚合**：按质量平方对 512 维单位特征向量增量加权累加，并重新 L2 归一化投影至单位超球面；
  3. **防漂移校验 (Anti-Drift Outlier Defense)**：新提取特征与当前融合特征的余弦相似度必须 $\ge 0.55$，拦截遮挡误检或跟踪漂移对特征池的污染；
  4. **低频抓拍侧载**：best-shot 目标在 `face.embedding` 携带 512 维 Float32 的 Base64 sidecar，仅供后端识别对账消费。
- `av_algo_extract_face()`：C ABI 独立特征提取符号，供宿主低频抓拍与人员底库注册建档路径传入单帧图像，执行检测、五点仿射对齐和 EdgeFace-S 提取，返回 L2 归一化 512D embedding 与 112×112 JPEG。**注册阶段专项引入 TTA (Test-Time Augmentation，水平镜像增强)**，分别对 112×112 对齐人脸与其水平翻转图像提取特征并执行加和超球面归一化，极大提升单照底库表征鲁棒性；常驻视频流 `instance_process()` 路径严格保持单次前向，杜绝 NPU 翻倍开销。可通过 `HEIMDALL_DISABLE_REGISTRATION_TTA=1` 环境变量显式关闭以作基线对照。

## 资源与并发契约

单进程内同一算法包目录只创建一个 RKNN worker（`heimdall-rk3568-face-npu` 线程），其内常驻
detector / embedder 两组 context；**所有通道实例共享该 worker**（RK3568 CMA 紧张下的强制约束，
见 `docs/nuwa/backend/algo-sdk-guidelines.md`）。由此产生三条硬约束：

- **聚合吞吐即 NPU 上限**：一次 `instance_process()` 对应一个请求，内含 yolov8n 与 SCRFD 两次
  前向（量级数十毫秒，以板端实测为准）。多路相机帧率之和不得超过该上限，否则超出的请求会被
  有界邮箱（容量 6）按「丢最旧」淘汰，被淘汰方以 `AV_ERR_TIMEOUT` 上报——该邮箱是同一算法包内
  所有通道共享的，被淘汰者往往是其它通道的帧，因此饱和时会打印带 `shed_total` 的 WARN。
  看到该 WARN 应下调路数或帧率，而不是排查网络。
- **像素阈值以分析帧为基准**：`min_face_size` 按分析帧像素判定，默认分析低分辨率子码流，
  切换到主码流后同一数值对应的实际人脸更小；`quality_min_score` / `max_yaw` / `max_pitch` /
  `max_blur` 由关键点几何与置信度导出，与分辨率无关。
- **RGA 输出几何预算**：SDK 的 `RgaCvEngine` 按 `(width, height)` 缓存输出池，全进程上限 16 种
  且**不淘汰**，超限后 `cv::crop_rgb` 永久失败。best-shot ROI 因此收敛到固定档位集合
  （`SNAPSHOT_ROI_TIERS` = 128/192/256/384，超出最高档位时退化为整帧该轴尺寸），几何种数与
  分辨率无关。新增任何 RGA 路径时必须遵守同一预算，否则会静默拖垮整包特征提取。
  同一引擎输出池 `max_size` 为 4，即最多 4 路并发 letterbox 缓冲，超出的调用方阻塞等待池回收。

## 现场诊断

```bash
# 包内诊断日志（含启动时的一次性 cls 激活模式判定与降级会话集告警）
RUST_LOG=face_recognition_rk3568=debug,algo_sdk=info <进程>
```

- `YOLOv8n 人体检测 cls 分支激活模式判定`：`probability_mode=true` 表示模型输出已是概率
  （直通 + 钳位），`false` 表示 raw logits（sigmoid 激活）。该判定按整张量做一次，不逐格切换。
- `RK3568 人脸算法包以降级会话集启动`：模型文件存在但 `RknnSession` 加载失败（版本不匹配 /
  权重损坏）。此时注册检测会自动回退到与视频流一致的 640×384 输入尺寸。
- `RKNN 邮箱饱和，已淘汰最旧请求`：按上文聚合吞吐核算路数。
- `exceeded maximum cached RGA buffer pools`：有 RGA 路径绕过了档位约束。

## C ABI 输出规范

`instance_process()` 输出符合女娲标准规范的检测 Envelope：

```json
{
  "schema_version": 1,
  "objects": [
    {
      "class_id": 0,
      "label": "person",
      "confidence": 0.95,
      "bbox": [0.1, 0.2, 0.4, 0.8],
      "face": {
        "bbox": [0.15, 0.22, 0.25, 0.35],
        "confidence": 0.92,
        "quality_score": 0.82,
        "embedding": "dHkevWKSYDxR4/K8..."
      }
    }
  ]
}
```

## 验证指令

开发机检查与单元测试：
```bash
cargo check -p face-recognition-rk3568-rknn --all-targets
cargo test -p face-recognition-rk3568-rknn --lib
cargo clippy -p face-recognition-rk3568-rknn --all-targets -- -D warnings
```

在具备物理 RK3568 NPU 的硬件设备上运行：
```bash
# 硬件前向集成测试
cargo test -p face-recognition-rk3568-rknn -- --ignored --nocapture

# 本地端到端自测工具
cargo run -p face-recognition-rk3568-rknn --bin face_recognition_rk3568_rknn_run_local -- testimage.jpg --loops 20
```
