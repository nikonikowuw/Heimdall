# RK3568 人脸识别算法包 (EdgeFace Face Recognition)

基于 Rockchip RK3568 平台的工业级人脸识别算法包，集成 **YOLOv8n-face** 检测器（含 5 点关键点）与 **EdgeFace-xs** 512 维特征提取器。

## 平台契约

- **平台**: Rockchip RK3568 (NPU 1.0 TOPS @ INT8)
- **操作系统**: Linux aarch64 (Debian / Ubuntu / Buildroot)
- **运行时支持**: librknnrt.so >= 1.5.0
- **算法 ABI**: 女娲标准算法插件 C ABI (`av_algo_get_abi`)

## 模型清单

- `model/yolov8n-face-640x384_rk3568_mixed_face.rknn`: 人脸检测与 5 点关键点模型（640×384 混合精度）
- `model/edgeface_xs_gamma_06_rk3568_fp16.rknn`: 512 维人脸特征提取模型（112×112 FP16）

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
