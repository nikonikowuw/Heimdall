# CoreML 模型目录

将转换后的模型包放置为：

- `yolov8_face.mlpackage`（或兼容软链 `yolov5n_face.mlpackage`）
- `edgeface_s.mlpackage`

转换输入权重位于包根目录的 `weights/`，转换工具链和参数见任务设计文档。`.mlpackage` 属于平台构建/发布产物，未完成转换前不由源码测试伪造。
