# C ABI 算法包宿主与沙箱加载体系 (Subtask 1)

## Goal

在 Rust 宿主内 1:1 承接原有 C ABI 规范（兼容 `sdk/include/argus/algo.h`），构建安全沙箱与动态加载器，支持现有 `algo-packages/macos/arm64/yolo26n` 等算法包热插拔与自测推理。

## Requirements

1. **C ABI 结构体与虚表 1:1 映射 (`crates/types` 或 `crates/infer`)**：
   - 映射 `av_algo_abi`、`av_algo_instance_args`、`av_frame_desc`、`av_algo_result`、`av_image_ops`、`av_frame_ops` 等核心 C 结构体与函数指针；
   - 必须使用 `#[repr(C)]`，并通过 `static_assertions` 严格验证各结构体的 `size_of` 和关键字段偏移（`offsetof`），确保 64 位 ABI 零漂移。
2. **平台感知型算法包拓扑 (Platform-Aware Directory)**：
   - 扫描 `algo-packages/{platform_id}/` 目录；
   - 启动时自动获取当前宿主平台的 `platform_id`（如 `macos-arm64-coreml` 或 `linux-arm64-rknn`），严格阻断不匹配架构的算法包载入。
3. **七步安全沙箱自检器 (Algo Sandbox)**：
   - 路径安全性防穿透检查；
   - SHA256 算法包完整性哈希校验；
   - `manifest.json` 与 `config.schema.json` 格式与平台兼容性校验；
   - 基于 `libloading` 动态打开 `.dylib` / `.so`，寻址符号并核对 `api_version` 与结构体尺寸；
   - **真实测试图自检（Self-Test）**：使用包内自带的 `testimage.jpg` 执行单次真实前向推理，验证返回状态码 `AV_OK` 且不发生崩溃；
   - 完成沙箱自测后热注册至可用算法注册表。
4. **算法实例创建与前向推理**：
   - 能够根据 `algorithm_id` 创建 `av_algo_instance`，传入帧描述符 `av_frame_desc` 并稳定获取目标感知结果（BBox, class, confidence）。

## Acceptance Criteria

- [ ] `cargo test -p types -p infer` 验证所有 C ABI 结构体 `size_of` 与内存对齐断言 100% 通过。
- [ ] 成功将原系统已有的 `algo-packages/macos/arm64/yolo26n` 载入 Rust 宿主。
- [ ] 算法包沙箱能对 `testimage.jpg` 执行一次真实 Core ML 推理自测，成功解析出检测框。
- [ ] 具备良好的 `unsafe` 边界隔离与异常捕获防护，动态库不可传导崩溃。
