# 推理后端

平台预处理、模型加载与 SDK 调用收敛在 `infer`/媒体 FFI 及算法实现内，上层只使用统一契约。

## 实现入口

| 入口                                               | 职责                                   |
| -------------------------------------------------- | -------------------------------------- |
| [backend.rs](../../../crates/infer/src/backend.rs) | 当前 `InferenceBackend` 公开接口       |
| [worker.rs](../../../crates/infer/src/worker.rs)   | 有界队列与常驻推理 Worker              |
| [package.rs](../../../crates/infer/src/package.rs) | `AlgoPackage`、实例和注册表            |
| [sandbox.rs](../../../crates/infer/src/sandbox.rs) | 包验证与平台匹配                       |
| [算法 SDK 规范](./algo-sdk-guidelines.md)          | 插件 trait、C ABI、帧/预处理与模型会话 |

当前公开方法为：

```rust
async fn detect(&self, frame: &FrameRef) -> Result<Vec<Detection>, InferError>;
```

旧 `LoadedModel::infer/RawOutput/BackendCapabilities` 示例是未落地草案，不能据此调用或新增重复抽象。
对外 async 不代表 SDK 可以在 Tokio Worker 内执行；同步硬件工作仍须按 [并发规范](./concurrency-guidelines.md) 隔离。

## 后端选择与模型

- feature 名称和依赖见 [infer/Cargo.toml](../../../crates/infer/Cargo.toml)：默认 `backend-cpu`，另有 `backend-coreml/rknn/ascend`。
- feature 只决定可用实现，不控制业务行为；部署明确选择后端，不隐式猜测或伪装硬件成功。
- 至少需要一个可用后端；请求未编译/不可用的后端应明确报错。旧“零后端编译期断言”尚未在 `lib.rs` 实现。
- 开发机测试不依赖 NPU；CPU 仅作物理无加速器时的显式调试回退，不能代表硬件实现已验证。
- 模型和 session 常驻固定 Worker，同一 session 不并发调用，不逐帧加载模型或迁移线程。
- 硬件预处理在对应后端/插件内完成，不在 Pipeline 建全量 CPU 像素转换抽象。
- 模型、输入尺寸、类别、归一化/量化参数由算法包元数据或模型描述提供；转换脚本记录完整参数和量化数据来源，变化时同步更新。
- `.rknn/.om/.mlpackage` 按平台交付，不混用；模型二进制走独立分发，转换脚本受版本控制。

## 输出与后处理

系统级规则、跟踪与平台无关后处理归 `pipeline`；模型私有张量解码留在算法包并复用 SDK 数学工具。
SDK 内置 NMS 等特殊路径必须适配成同一对外结果，不能把平台张量/错误码泄露给上层。
坐标去 padding 和逆缩放共用预处理参数，最终格式与时间基准一致，详见 [跨层检查](../guides/cross-layer-thinking-guide.md)。

CoreML 计算单元、RKNN 核心掩码、映射缓存和输出 RAII 的约束统一放在 SDK 的 [Apple Silicon](./algo-sdk-guidelines.md#apple-silicon) / [Rockchip RKNN](./algo-sdk-guidelines.md#rockchip-rknn) 小节，避免两处漂移。

## 验证

- 无硬件测试验证类型/形状、坐标范围、配置与失败分支。
- 真机测试按平台 feature + `#[ignore]` 隔离，记录设备、SDK、预处理和模型版本。
- 同图跨后端比较统一结果，量化误差按实测阈值验收，不要求逐位相等。
- 三平台精度/资源预算需要各自基线；声明支持不等于已完成真机验证。
