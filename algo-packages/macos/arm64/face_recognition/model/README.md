# CoreML 模型目录

本目录存放 Apple Silicon (CoreML / ANE FP16) 专用算法模型包：

- `yolov8_face.mlpackage`（人脸检测与 5 点定位，640x384，兼容软链 `yolov5n_face.mlpackage`）
- `person_detect_640x384.mlpackage`（人体检测与航迹关联基底，640x384，兼容软链 `person_detect.mlpackage`）
- `person_detect_384x216.mlpackage`（人体检测轻量版，384x224 Stride-32 对齐，用于高帧率独立横向对比）
- `edgeface_s.mlpackage`（EdgeFace-s 512D 人脸特征提取，112x112）

转换输入权重位于包根目录的 `weights/`（由 `convert_models.py` 自动化导出并静态量化为 MIL FP16 架构）。
