# RK3576 RKNN 通用目标检测算法包实施计划

## 实施步骤

1. **目录结构与资产初始化**
   - 将原目录 `algo-packages/rknn/rk3568/general_detection` 移动并重命名为 `algo-packages/rknn/rk3576/general_detection`。
   - 创建并配置 `manifest.json`、`config.schema.json`、`Cargo.toml`。
   - 从 `algo-packages/macos/arm64/general_detection` 复制自测图 `testimage.jpg`。
   - 更新根目录 `Cargo.toml` 工作空间 `members`。

2. **核心配置与类别定义 (`src/config.rs`)**
   - 移植 COCO 80 类别定义、`ClassMask` 位掩码与实例配置反序列化逻辑。
   - 编写单元测试验证类别查找与掩码过滤。

3. **RKNN 运行时动态加载与安全绑定 (`src/rknn.rs`)**
   - 声明 RKNN C ABI 结构体与函数指针（`rknn_init`, `rknn_inputs_set`, `rknn_create_mem_from_fd`, `rknn_set_io_mem`, `rknn_run`, `rknn_outputs_get`, `rknn_outputs_release`, `rknn_destroy`）。
   - 实现 `libloading` 动态查找并加载 `librknnrt.so`。
   - 封装 RAII 安全会话结构 `RknnSession`。

4. **张量后处理与坐标反算 (`src/postprocess.rs`)**
   - 实现针对 `[1, 84, 5040]` NCHW 张量的解码逻辑。
   - 整合 `algo_sdk::math::fast_nms` 和 `algo_sdk::math::unmap_box`。
   - 编写包含多种假数据（阈值过滤、重叠框抑制、黑边消除）的完整单元测试。

5. **插件核心实现与生命周期接入 (`src/plugin.rs` & `src/lib.rs`)**
   - 实现 `AlgoPlugin` 和 `AlgoInstance` trait。
   - 组装 `RgaCvEngine` 预处理与 RKNN 推理双模输入。
   - 使用 `export_algo!` 导出标准 C ABI 接口。

6. **编译验证与质量门禁**
   - 运行 `cargo fmt --all -- --check`。
   - 运行 `cargo clippy --all-targets -- -D warnings`。
   - 运行 `cargo test --package general-detection-rknn`。
   - 验证动态库能够成功编译并产出 `target/debug/libgeneral_detection.so`。

7. **板端硬件实测与基准测试 (RK3576 NPU + RGA)**
   - **Linker 根因解决**：使用 Podman + aarch64 独立 sysroot 消除 Fedora 缺失用户态 glibc 的编译障碍。
   - **`rknn_set_io_mem` 返回 -4 (RKNN_ERR_MALLOC_FAIL) 根因分析与修复**：
     - 在 RK3576 librknnrt 驱动下，`rknn_create_mem_from_fd` 的 `virt_addr` 参数不能传 NULL。若传 NULL，底层的 `rknn_set_io_mem` 会尝试在驱动内部为张量分配转换虚拟内存，因缺少分配器而报错 `-4` (`RKNN_ERR_MALLOC_FAIL`)。
     - 修复方案：在 `dma_mem_cache` 中为 DMA-BUF fd 通过 `libc::mmap` 获取有效用户态虚拟地址，传入 `rknn_create_mem_from_fd`；并在 `Drop` 中通过 `libc::munmap` 统一 RAII 释放。经 C 程序与 Rust 算法包双向验证均 100% 成功。
   - **`hardware_infer_test` 离线路径修正**：增加针对运行目录下 `model/` 的回退探测，使脱离 Cargo 源码目录的单板独立执行可直接定位模型。
   - **硬件压测与基准性能数据** (100 次循环，5 次预热)：
     - **预处理 (RGA 2D 硬件缩放/Letterbox)**: 平均 **1.42 ms** (对比 C++ 版本的 2.77 ms，快 1.35 ms，因 Rust 直接以 DMA-BUF 直通免去 Host 内存回拷)
     - **NPU 推理 (RKNN 640x384 INT8)**: 平均 **8.42 ms**，中位数 P50 **8.07 ms** (对比 C++ 版本的 10.01 ms / P50 9.69 ms，通过双核 `CORE_0_1` 调度与优化内部通道实现)
     - **后处理 (INT8 DFL 解码 + NMS)**: 平均 **2.09 ms** (消除循环内 30 万次边界检查)
     - **端到端处理耗时**: 平均 **11.94 ms** (P50 11.61 ms) / **FPS: 83.8**
     - **ABI 完整调用链**: 平均 **12.01 ms** / **FPS: 83.2** (对比 C++ 版本的 13.62 ms / 73.42 FPS，**提升 +10 FPS (+14%)**)
     - 成功检出目标：bus 90.50%, person 83.84%, person 75.29%, person 70.91%，精准绘制检测框并保存 `result.jpg`。
