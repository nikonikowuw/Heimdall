# Implementation Plan: Rockchip RGA 硬件加速驱动与 CvEngine

- 任务：.trellis/tasks/09-07-rockchip-rga-cv-engine
- 状态：implementation complete; board validation pending
- 模块：`crates/algo-sdk`
- 负责人：niko

---

## 0. API 契约跳过说明 (API Contract Skip)

本任务为算法套件内部底层硬件加速驱动与 SPI 扩展（纯 Rust / FFI / Linux UAPI），不增加或变更任何 HTTP 或 WebSocket API 接口，不涉及前端交互，因此依规范跳过 `api.md` 契约编制。

---

## 1. 执行阶段规划 (Execution Phases)

### Step 1: 特征门控与平台骨架搭建
- **目标**：在 `crates/algo-sdk` 中配置 `rga` feature 与目录结构。
- **改动文件**：
  - `crates/algo-sdk/Cargo.toml`：新增 `[features]` 中的 `rga = ["dep:tracing"]`；
  - `crates/algo-sdk/src/cv/platforms/mod.rs`：在 `target_os = "linux"` 条件下引入 `rockchip` 模块；
  - 新建 `crates/algo-sdk/src/cv/platforms/rockchip/` 目录与模块骨架。
- **验证**：`cargo check -p algo-sdk` 与 `cargo check -p algo-sdk --features rga` 编译通过。

### Step 2: RGA C FFI 结构与动态加载器 (`ffi.rs`)
- **目标**：声明 librga / im2d 的 C 结构体与类型，实现零依赖动态加载。
- **改动文件**：
  - `crates/algo-sdk/src/cv/platforms/rockchip/ffi.rs`；
  - 声明 `rga_buffer_t`, `im_rect`, `rga_buffer_handle_t` 等 C POD 结构体，包含字段对齐与常量定义；
  - 基于 `libc::dlopen` / `libc::dlsym` 实现 `RgaApi` 动态查找封装；
  - 实现 `RgaHandleGuard` RAII 守卫（退出时调用 `releasebuffer_handle`）。
- **验证**：编写 mock / ABI layout 单元测试，验证结构体大小和对齐符合 C ABI 规范。

### Step 3: 硬件防御策略、DMA 分配器与缓冲池 (`policy.rs`, `dma_alloc.rs`, `pool.rs`, `config.rs`)
- **目标**：移植并适配 RGA 硬件限制防御，实现输出 DMA-BUF 缓冲池化与多层级参数配置。
- **改动文件**：
  - `crates/algo-sdk/src/cv/platforms/rockchip/policy.rs`：RGA2/RGA3 最小分辨率（68px / 2px）、缩放倍率（1/8x~8x vs 1/16x~16x）、步长对齐与 4GB MMU 寻址防线；
  - `crates/algo-sdk/src/cv/platforms/rockchip/dma_alloc.rs`：封装 Linux `/dev/dma_heap` 的 ioctl 分配，以及对齐虚拟内存分配作为优雅保底；
  - `crates/algo-sdk/src/cv/platforms/rockchip/pool.rs`：实现 `RgaBufferPool`，预分配输出 DMA-BUF 并单次执行 `importbuffer_fd` 长期持有 handle，通过 RAII Lease 提供 0 成本借还机制；
  - `crates/algo-sdk/src/cv/platforms/rockchip/config.rs`：实现 `RgaPoolConfig` 及其 Serde 反序列化、环境变量解析（`ARGUS_RGA_POOL_*`）、Builder 模式与边界合法性硬校验。
- **验证**：编写单元测试验证：
  - `test_rga_pool_config_serde_and_env_override`（反序列化与环境变量覆盖优先级）；
  - `test_rga_pool_config_validation`（边界合法性拦截）；
  - `test_rga_buffer_pool_lease_and_recycle`（Buffer Pool 借还循环与析构时的资源回收）。

### Step 4: 实现 `RgaCvEngine` 与 `CvEngine` Trait (`engine.rs`)
- **目标**：实现针对 Rockchip 平台的完整图像预处理引擎。
- **改动文件**：
  - `crates/algo-sdk/src/cv/platforms/rockchip/engine.rs`；
  - 实现 `RgaCvEngine::new()` 与内部池化实例；
  - 实现 `letterbox()`：
    1. 输入 DMA-BUF 解包；
    2. `policy` 防御性校验；
    3. 获取输入 `src_handle`（基于单帧同步生命周期创建 `RgaHandleGuard`，杜绝跨帧非受控缓存导致内核 fd 轮转复用时命中陈旧句柄）；
    4. 从 `RgaBufferPool` 租借输出缓冲区（直接使用预导入的 `dst_handle`，零开销）；
    5. 调用底色填充（`imfill`）；
    6. 等比居中缩放渲染（`improcess`）；
    7. 返回 `CvBuffer`（挂载池租约 guard）与 `PreprocessMode::Letterbox`。
  - 实现 `resize()`：类似流程，执行无黑边强制拉伸；
  - 在遇到硬件不支持或动态库缺失时，无缝委托给 `CpuCvEngine`。
- **验证**：构造测试帧模拟调用，验证函数签名与结果契约。

### Step 5: 视觉门面入口自动路由整合 (`cv/mod.rs`)
- **目标**：将 `RgaCvEngine` 接入全局分发。
- **改动文件**：
  - `crates/algo-sdk/src/cv/mod.rs`：在 `default_engine()` 中加入 Linux + `rga` 分支：
    ```rust
    #[cfg(all(target_os = "linux", feature = "rga"))]
    {
        Arc::new(platforms::rockchip::RgaCvEngine::new())
    }
    ```
- **验证**：验证不同编译配置下的引擎选择行为。

### Step 6: 完整门禁与质量自检
- **目标**：运行完整代码格式化、Lint 与测试。
- **检查命令**：
  ```bash
  cargo fmt --all -- --check
  cargo clippy -p algo-sdk --all-targets --features rga -- -D warnings
  cargo test -p algo-sdk --all-targets
  ```
- **质量确认**：
  - 确认所有 `unsafe` 块包含符合规范的 `// SAFETY:` 注释；
  - 确认无调试打印残留；
  - 确认非 Linux 开发机环境依然能够 100% 通过编译与测试。

## 7. Implementation outcome

- [x] Linux + `rga` feature routing, runtime `librga` loading, ABI layout checks, and CPU/host isolation.
- [x] DMA-heap allocation with deterministic heap candidates, DMA32 filtering for `Auto`/`Rga2`, and bounded `RgaBufferPool` with prewarming, timeout, lazy idle eviction, cached handles, and RAII leases.
- [x] `RgaCvEngine` letterbox/resize using imported DMA-BUF handles, `imcheck_t`, `imfill_t`, synchronous `improcess`, source color metadata, and typed errors.
- [x] Defensive checks for modifiers, plane offsets, representable NV12/I420 strides, RGA2/RGA3 dimensions, scaling ratios, alignment, maximum stride, and checked allocation sizes.
- [x] Host frames and unavailable runtime use the existing CPU path; DMA-BUF failures do not perform implicit CPU readback.
- [x] Pure-logic, ABI, pool, allocator-policy, source-layout, and cross-feature tests pass on the x86_64 development host.
- [ ] RK3576 board validation with the deployed kernel, DMA heaps, `librga`, and RGA driver remains pending.
