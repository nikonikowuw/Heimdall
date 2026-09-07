# PRD: Rockchip RGA 硬件加速驱动与 CvEngine 实现 (工业级 IoC 架构)

## 1. 目标与背景

在边缘 AI 与视频分析系统中，“**算法包开发者体验 (Developer Experience)**”与“**系统物理显存管控 (System Governance)**”是一对经典矛盾：
- 若放任算法包自管显存：算法插件各行其是，直接打开内核驱动，多路并发时极易吃爆 Linux CMA 显存导致整机 OOM 崩溃；
- 若宿主完全垄断强耦合：算法工程师在独立仓库开发时无法脱离复杂的宿主工程进行单测（`cargo test` / `run_local` 困难）。

**核心目标**：
采用工业界成熟的 **“契约声明 + 宿主托管 + SDK 降级保底” (IoC 控制反转模式)**，在 `crates/algo-sdk` 中实现高性能 Rockchip RGA 硬件加速驱动 `RgaCvEngine` 与类数据库连接池模式的显存池 `RgaBufferPool`：
1. **算法端零心智负担**：算法仅静态声明输入规格（如 640×640 RGB），代码中直接调用 `cv::letterbox(&frame, 640, 640)`，无需触碰任何驱动与池化底层；
2. **底层高性能零拷贝**：直接消费 MPP 解码产出的 DMA-BUF，通过 RGA 硬件 2D 引擎完成色彩转换与等比缩放，输出符合 NPU 对齐规范的 `CvBuffer`；
3. **类数据库连接池机制**：通过 `min_idle`、`max_size`、`acquire_timeout` 和 RAII 租约借还，彻底消除每帧分配与 IOMMU 映射开销；
4. **双模运行与平滑保底**：优先无缝对接宿主 C ABI 注入的受控显存，并在独立开发与单元测试环境中自动提供本地微型池保底。

---

## 2. 需求规范 (Requirements)

### 2.1 算法端契约与开发者体验 (Developer Experience)

1. **统一极简门面**：
   - 算法开发者在算法包中仅通过标准门面调用：
     ```rust
     let (buf, mode) = cv::letterbox(&frame, 640, 640, [114, 114, 114])?;
     let (buf, mode) = cv::resize(&frame, 640, 640)?;
     ```
   - 算法包不包含任何关于 `/dev/dma_heap`、`importbuffer_fd`、IOMMU 映射或池化大小的硬编码配置。
2. **纯粹声明式规格**：
   - 算法包仅在 `manifest.json` 中声明模型的输入张量规格（宽、高、通道、色彩格式），由运行时系统读取匹配。

### 2.2 类数据库连接池模式的显存管理 (`RgaBufferPool`)

彻底消除高频多路并发下每帧分配 DMA-BUF 与频繁调用 `importbuffer_fd`（内核 IOMMU 页表建立/拆卸）的吞吐瓶颈：

1. **连接池四维生命周期模型**：
   - **`min_idle`（核心常驻预热）**：初始化时预先分配固定数量的 DMA-BUF，并**单次调用 `importbuffer_fd` 长期持有 `rga_buffer_handle_t`**，首帧即达峰值性能，零冷启动延迟；
   - **`max_size`（物理显存安全硬顶）**：严格限制最大缓冲分配上限，应对多核 NPU 并发或级联多目标切片（Crop）突发，杜绝吃爆 Linux CMA 内存产生 OOM；
   - **`acquire_timeout`（获取超时熔断）**：当下游 NPU 严重卡顿或缓冲耗尽时，超过设定时间（如 50ms）主动熔断丢帧，坚决防止向上游解码器传播阻塞死锁；
   - **`idle_timeout`（弹性缩容归还）**：应对突发多切片高峰而临时扩容的物理显存，在空闲超时（如 10s）后自动注销 RGA 句柄并关闭 fd，平滑退还 Linux 系统。
2. **RAII 租约借还机制**：
   - 从池中借出 `PooledRgaBuffer`，直接使用已缓存的 `handle` 进行 `wrapbuffer_handle`，运行时 `alloc`、`free`、`import`、`release` 开销全部为 0；
   - 租约随下游 `CvBuffer` 在推理结束析构时触发 `Drop`，原子归还池可用队列，**绝不调用 `releasebuffer_handle`**；
   - 仅在缓冲池彻底析构或空闲缩容时才注销硬件句柄。

### 2.3 双模运行与控制反转 (Inversion of Control)

1. **生产环境（宿主托管优先）**：
   - 当算法运行在 Heimdall 宿主进程中时，宿主通过 C ABI `AvImageOps` 统一注入全局显存池；
   - 算法 SDK 优先走 `HostCvEngine`，消费宿主全局配额内的池化显存。
2. **独立开发与单测环境（SDK 本地保底）**：
   - 当在独立算法仓库进行 `cargo test` 或使用本地调试器 `run_local` 时（未注入 `image_ops`）：
   - `algo-sdk` 自动降级为本地 `RgaCvEngine`，并自建安全的最小默认池（`min_idle=2, max_size=4`）；
   - 保证算法开发者在没有完整宿主监控服务的情况下，本地单测 100% 顺畅通过。

### 2.4 非功能性需求与底层硬件防御 (Non-Functional Defenses)

严格遵照 `rknn-pro` 嵌入式生产实践避坑准则：

1. **根除句柄级联崩溃**：统一使用 Handle 模式，严禁调用每帧 `wrapbuffer_fd`，从架构上切断内核 fd 复用命中陈旧脏缓存（Stale Handle）的硬件崩溃链条；
2. **RGA3 与 RGA2 规格硬隔离**：
   - 尺寸拦截：RGA3 核心输入输出必须 $\ge 68\text{px}$（RGA2 $\ge 2\text{px}$）；小于 68px 时强制规避 RGA3 核心，调度至 RGA2 或 CPU；
   - 缩放比拦截：RGA3 缩放倍率必须在 $[1/8\times, 8\times]$（RGA2 为 $[1/16\times, 16\times]$）；
3. **RGA2 4GB MMU 寻址防线**：
   - RGA2 为 32 位 MMU，物理地址超 4GB 会引发内核段错误；DMA-BUF 分配优先请求 `/dev/dma_heap/system-dma32`；
4. **无硬件环境平滑降级**：
   - 在 macOS、CI x86_64 或缺少 `librga.so` 的 Linux 容器中，`RgaCvEngine` 运行期探测失败后无缝回退至 `CpuCvEngine`，打印 warning 日志，绝不引发 panic 崩溃。

---

## 3. 验收标准 (Acceptance Criteria)

- [x] **接口与 Trait 契约**：在 `crates/algo-sdk` 中实现 `RgaCvEngine` 与 `RgaBufferPool`，接入统一 `CvEngine` 门面。
- [x] **池化借还验证**：编写单元测试验证 `RgaBufferPool`：
  - `test_rga_pool_lease_and_recycle`：验证初始化 `min_idle` 预热分配与借还循环；
  - `test_rga_pool_burst_and_idle_eviction`：验证突发超过 `min_idle` 时的弹性扩容与空闲淘汰；
  - `test_rga_pool_timeout_protection`：验证满载时 `acquire_timeout` 熔断不发生永久死等。
- [x] **硬件规格与防御单测**：
  - 覆盖 RGA2/RGA3 最小尺寸、缩放比极限、NV12/I420 步长对齐和 RGA 最大步长的合法性拦截；
  - 验证 Handle RAII 行为，保证异常路径下句柄 100% 被释放。
- [x] **独立开发测试与平滑降级**：
  - 在无 `librga.so` 的环境中，`algo-sdk` 单元测试整体全绿，优雅降级为 `CpuCvEngine`；
  - 在 `cargo test -p algo-sdk --all-targets` 下无任何内存泄漏与崩溃。
- [x] **代码质量门禁**：
  - `cargo fmt --all -- --check` 通过；
  - `cargo clippy -p algo-sdk --all-targets --features rga -- -D warnings` 零警告零错误；
  - 严谨的 `// SAFETY:` 注释，无 `dbg!` / `println!` / `todo!` 残留。
