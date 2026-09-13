# CoreML 模型目录

本目录存放 Apple Silicon (CoreML / ANE FP16) 专用算法模型包：

- `yolo26n.mlpackage`（COCO 人体检测，输出 `[1, 300, 6]` 的 `xyxy + score + class_id`，640x384）
- `yolov8_face.mlpackage`（人脸检测与 5 点定位，640x384）
- `edgeface_s.mlpackage`（EdgeFace-s 512D 人脸特征提取，112x112）

`convert_models.py` 负责从包根目录的 `weights/` 输入权重导出模型；生产包只收录上述运行时实际加载的模型。
