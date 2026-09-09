# 实现计划：人脸识别算法包 RK3576 迁移

## API 契约跳过说明

单侧任务（纯 Rust 算法包，无前后端拆分），跳过 api.md。C ABI 接口签名已在 PRD 的验收标准中定义。

## 前置条件

- RK3576 板子可通过 `ssh rk3576` 访问 ✅
- 板子上已有 librknnrt.so、Python 3.11、rknn-toolkit2 ✅
- 模型文件已在板子上验证 ✅
- `algo-packages/rknn/rk3576/general_detection` 作为架构参考 ✅

## 实现步骤

### Step 1: 项目脚手架
- 创建 `algo-packages/rknn/rk3576/face_recognition/` 目录结构
- 编写 `Cargo.toml`（依赖 workspace + libloading + libc）
- 编写 `manifest.json`（platform_id: "rknn-rk3576"）
- 复制模型文件到 `model/` 目录
- 验证：`cargo check -p face-recognition` 通过

### Step 2: RKNN 绑定层
- 从 `general_detection/src/rknn.rs` 复制并精简
- 移除 `infer_with_dma_buf` 和 `new_fallback`
- 修改 `get_hardware_outputs` 的量化判断逻辑
- 添加 `want_float` 控制参数
- 验证：`cargo check` 通过

### Step 3: 检测解码（核心）
- 新建 `detect.rs`，实现 `compute_dfl()`
- 实现 `decode_scale()` 单尺度解码
- 实现 `decode_yolov8_face()` 三尺度合并 + NMS
- 定义 `RawFace` 结构体（兼容 Mac 版）
- 编写单元测试：用已知输入验证 DFL 解码和 keypoint 解码
- 验证：`cargo test` 通过

### Step 4: 对齐与质量
- 实现 `align.rs`：5 点仿射 → 112×112 RGB
- 从 Mac 版移植 `quality.rs`（无需修改，输入已是归一化坐标）
- 验证：`cargo test` 通过

### Step 5: C ABI 入口
- 重写 `lib.rs`：移除 CoreML 代码，接入 RKNN session
- 实现 `av_algo_extract_face()` 完整流程
- 保留 `normalize_embedding()` 和 `cosine_similarity()`
- 验证：`cargo build` 生成 .so

### Step 6: 插件层
- 实现 `plugin.rs`：FaceRecognizer 持有 runtime + 2 sessions
- 实现 `process()` 检测路径
- 编写 `config.rs` 配置结构
- 验证：`cargo check` 通过

### Step 7: 本地测试入口
- 编写 `run_local.rs`：加载模型 → 读图 → 检测 + 嵌入 → 输出结果
- 在 RK3576 板子上交叉编译并运行
- 验证：检测框和 embedding 输出正确

### Step 8: 硬件集成测试
- 编写 `tests/hardware_infer_test.rs`（`#[ignore]`）
- 用 rknn_model_zoo 测试图片验证端到端流程
- 对比 Python 参考实现的 cosine similarity
- 验证：`cargo test -- --ignored` 在板子上通过

### Step 9: 质量门禁
- `cargo fmt --all`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test --workspace`
- 验证：全部通过

## 验证命令

```bash
# 开发机
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test -p face-recognition

# RK3576 板子（交叉编译后）
ssh rk3576 "cd /path/to/face_recognition && ./run_local model/yolov8n-face-640x384_mixed.rknn testimage.jpg"
```

## 回滚点

- Step 1-2：删除目录即可
- Step 3-6：git checkout 恢复
- Step 7-8：测试代码不影响核心库
