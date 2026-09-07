# Technical Design: Rockchip RGA 硬件加速驱动与 CvEngine (工业级 IoC 架构)

- 任务：.trellis/tasks/09-07-rockchip-rga-cv-engine
- 状态：draft
- 模块：`crates/algo-sdk`
- 作者：niko

---

## 1. 架构总览与 IoC 控制反转设计

### 1.1 系统上下文与双模调度
本模块采用工业界标准的 **IoC (Inversion of Control)** 模式，兼顾“宿主全局显存统管”与“算法独立开发测试体验”：

```text
                                [算法业务代码]
                                      │
            ┌─────────────────────────┴─────────────────────────┐
            │  cv::letterbox(&frame, 640, 640, [114, 114, 114])  │
            └─────────────────────────┬─────────────────────────┘
                                      │ active_engine() 解析
                                      ▼
             ┌─────────────────────────────────────────────────┐
             │       当前上下文是否存在宿主注入的 AvImageOps ?    │
             └───────────────┬─────────────────┬───────────────┘
                             │ 是              │ 否 (单测/独立开发)
                             ▼                 ▼
                     [HostCvEngine]     [RgaCvEngine] (Linux rga)
                            │                  │  或回退 [CpuCvEngine]
                            ▼                  ▼
                    宿主全局池分配缓冲    本地 RgaBufferPool 借调缓冲
                            │                  │
                            └────────┬─────────┘
                                     ▼
                          [RGA 硬件渲染 (im2d)]
                                     │
                                     ▼
                   [CvBuffer (带 RAII 租约的 DMA-BUF)]
                                     │
                                     ▼
                         [下游 RKNN NPU 硬件推理]
```

### 1.2 核心权责切分
1. **算法工程师视角**：
   - 零心智负担：只声明 `manifest.json` 输入张量尺寸，只调 `cv::letterbox` / `cv::resize`；
   - 彻底免除 Linux 内核 `ioctl`、DMA-BUF 申请与 IOMMU 映射等繁琐底层。
2. **宿主系统视角**：
   - 宿主可通过 C ABI `AvImageOps` 统一管控多算法显存分配总量，设定安全红线；
   - 本模块实现的 `RgaBufferPool` 同时也是一个高度通用的显存连接池，未来可直接复用到宿主侧。
3. **独立开发/单测视角**：
   - 当在算法团队独立仓库运行 `cargo test` 或 `run_local` 时，无需 mock 庞大的流媒体宿主，本地 `RgaCvEngine` 自带安全小池子自动兜底运行。

---

## 2. 模块结构与关键文件

```text
crates/algo-sdk/
├── Cargo.toml                          # 增加 features: [ "rga" ]
└── src/
    └── cv/
        ├── mod.rs                      # default_engine() 增加 RGA 路由分支
        └── platforms/
            ├── mod.rs                  # 导出 rockchip 模块
            ├── rockchip/
            │   ├── mod.rs              # 导出 RgaCvEngine, RgaBufferPool, RgaPoolConfig
            │   ├── engine.rs           # 实现 CvEngine trait (letterbox, resize)
            │   ├── ffi.rs              # librga / im2d C 结构体与运行时动态加载
            │   ├── pool.rs             # 显存连接池 (min_idle, max_size, acquire_timeout, RAII)
            │   ├── dma_alloc.rs        # Linux dma_heap 辅助分配器 (DMA-BUF 输出缓冲)
            │   └── policy.rs           # RGA2/3 尺寸、缩放比、对齐与寻址防御守卫
            ├── host_ops.rs             # 宿主 AvImageOps 代理（已存在，受控生产首选）
            └── cpu.rs                  # CPU 回退实现
```

---

## 3. 详细技术方案

### 3.1 C FFI 绑定与零链接依赖动态加载 (`ffi.rs`)
为了确保开发机（macOS / CI x86_64）在交叉编译与单测时不因找不到 `-lrga` 报错：
- 基于 `libc::dlopen("librga.so", ...)` 与 `libc::dlsym` 实现符号动态加载；
- 封装 `RgaApi` 结构体，集中持有 `importbuffer_fd`、`releasebuffer_handle`、`wrapbuffer_handle_t`、`improcess`、`imfill`、`imcheck` 等虚表；
- 若加载失败，`RgaCvEngine` 自动检测并优雅回退到 `CpuCvEngine`。

### 3.2 类数据库连接池模式的显存池 (`pool.rs`)

借鉴数据库连接池（如 HikariCP / r2d2）的四维生命周期管理：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RgaPoolConfig {
    /// 最小常驻预热缓冲数（初始化时立刻分配并 import 长期持有，零冷启动延迟）
    pub min_idle: usize, // 默认: 2
    /// 最大缓冲容量硬顶（防止并发突发吃爆物理显存产生 OOM）
    pub max_size: usize, // 默认: 4
    /// 获取缓冲等待超时（毫秒，超时主动熔断丢帧，防上游死锁）
    pub acquire_timeout_ms: u64, // 默认: 50 ms
    /// 弹性缓冲空闲回收阈值（秒，突发高峰后自动注销并归还 Linux 内核）
    pub idle_timeout_sec: u64, // 默认: 10 s
    /// 目标图像宽度、高度与像素格式
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    /// 首选 Linux DMA-BUF 堆节点（如 /dev/dma_heap/system-dma32）
    pub dma_heap_path: Option<String>,
}
```

#### 运行机制：
1. **初始化预热**：创建池时立即分配 `min_idle` 个 DMA-BUF，并对每个缓冲**单次调用 `importbuffer_fd`**，得到其专属的 `rga_buffer_handle_t`，永久保存在池槽位中；
2. **RAII 租约借调**：
   ```rust
   pub struct PooledRgaBuffer {
       slot_id: usize,
       fd: i32,
       handle: i32, // 长期持有，复用不释放！
       pool: Arc<RgaBufferPoolInner>,
   }
   impl Drop for PooledRgaBuffer {
       fn drop(&mut self) {
           // 归还槽位至可用队列，绝对不调用 releasebuffer_handle！
           self.pool.recycle_slot(self.slot_id);
       }
   }
   ```
3. **弹性扩容与缩容**：
   - 当遇到级联多目标抠图（如一帧瞬时需要 6 个切片）且空闲槽位不足时，在 $\le \text{max\_size}$ 限制下弹性分配新缓冲并临时 import；
   - 突发峰值过去后，超出 `min_idle` 的槽位在闲置超过 `idle_timeout_sec` 时自动调用 `releasebuffer_handle` 并关闭 fd，安全将物理内存归还 Linux；
4. **获取超时防死锁**：
   - 当所有槽位均处于借出态且达到 `max_size` 时，`acquire` 最多等待 `acquire_timeout_ms`；超时立即抛出 `AlgoError::Busy`，通知上游丢弃过时帧，绝不永久死等。

### 3.3 显存分配策略 (`dma_alloc.rs`)
- **仅使用 DMA-BUF**：尝试打开 `/dev/dma_heap/system-dma32`（适配 RGA2 4GB 限制）或 `/dev/dma_heap/system`，通过 `ioctl(DMA_HEAP_IOC_ALLOCATE)` 申请 DMA-BUF fd；
- **不伪造虚拟地址回退**：容器或开发机没有可用 dma_heap 时，本地 RGA 池返回 `AlgoError::OutOfMemory`；Host 帧仍由 `CpuCvEngine` 处理，DMA-BUF 快速路径不会把 CPU 内存伪装成设备零拷贝。

### 3.4 硬件防御与执行生命周期 (`policy.rs` & `engine.rs`)

执行 Letterbox 的完整闭环：
1. **输入参数检验**：从 `SafeFrame` 提取 DMA-BUF fd；校验输入尺寸 $\ge 68\text{px}$（RGA3 限制），校验缩放比在 $1/8\sim 8\times$ 范围内；
2. **双端统一 Handle 与陈旧句柄防御**：
   - 输入源：单帧获取 `src_handle`（通过 `RgaHandleGuard` 执行 RAII 保护，处理完成后立即注销句柄，杜绝跨帧缓存导致内核 fd 轮转复用时命中陈旧句柄 / Stale Handle 击穿显存）；
   - 输出目标：从受控 `RgaBufferPool` 租借 `PooledRgaBuffer`，直接使用初始化预热绑定的长效 `dst_handle`；
   - **严格杜绝 Mixed Handle 与 Stale Handle**：两端统一走 `wrapbuffer_handle`，输出端复用池化句柄，输入端单帧即时释放，从根源消灭 `Cannot get dst channel buffer` 与陈旧句柄硬件崩溃；
3. **底色填充与等比居中渲染**：
   - 计算等比缩放布局 `layout = compute_letterbox_layout(src_w, src_h, dst_w, dst_h)`；
   - 调用 `imfill` 将目标全图填充指定 `fill_color`；
   - 构造目标 ROI 矩形：`im_rect { x: layout.offset_x as i32, y: layout.offset_y as i32, width: layout.target_width as i32, height: layout.target_height as i32 }`；
   - 调用 `improcess` 带 `IM_SYNC` 硬件同步阻塞等待渲染完成；
4. **包装与输出**：
   - 将 `PooledRgaBuffer` 包装入 `CvBuffer::from_dma_buf(fd, w, h, fmt, Some(Box::new(pooled_buf)))`；
   - 返回 `(cv_buffer, PreprocessMode::Letterbox(layout))`；
   - 下游 NPU 推理完成后 `cv_buffer` 析构，内部租约自动触发还池。
5. **多分辨率池容量硬顶**：
   - 单个 `RgaCvEngine` 实例内部缓存最多 `MAX_CACHED_POOLS` (16) 个不同输出几何规格的显存连接池；超出上限且未命中时返回 `AlgoError::Preprocess`，防止动态请求不同尺寸造成内存无界累积。

---

## 4. 降级矩阵 (Fallback Matrix)

| 场景 / 触发条件 | 处理策略 | 预期行为 |
| :--- | :--- | :--- |
| 宿主注入 `AvImageOps` (生产环境) | 走 `HostCvEngine` | 统一消费宿主受控显存池，无需本地分配 |
| 宿主未注入 `AvImageOps` (本地单测) | 走 `RgaCvEngine` | 本地自建小容量 `RgaBufferPool` (min_idle=2) 兜底 |
| 非 Linux 系统 (macOS / CI) | 走 `AppleCvEngine` / `CpuCvEngine` | 平台隔离，单测 100% 通过 |
| 缺少 `librga.so` 动态库 | 运行时探测并回退 | Host 帧回退至 `CpuCvEngine`；DMA-BUF 帧返回不兼容错误，不执行隐式 CPU readback |
| DMA-BUF heap 不可用或 RGA driver 拒绝请求 | 返回 typed error | 保持设备快速路径的所有权与零拷贝契约，由上游决定丢帧或切换路径 |
| 显式选择 RGA3 但尺寸 $< 68\text{px}$ 或缩放比超限 | 硬件策略守卫拦截 | 返回 `AlgoError::Preprocess`，不向 driver 提交非法任务（默认 `Auto` 策略下支持 $\ge 2\text{px}$ 自动调度至 RGA2） |
| 池化缓冲耗尽且等待超时 | 超时熔断 | 抛出 `AlgoError::Timeout`，上游 Drop-Oldest 丢帧 |
