# SDK placement 注入与 RKNN 共享权重运行时技术设计

> 所属任务：`10-03-algo-sdk-rknn-shared-weights`（阶段 C）。

## 1. 架构总览

```text
宿主 (InferenceWorker 启动)
  │
  ├─(下发 WirePlacementMetadata: core_mask, strategy)
  ▼
C ABI av_algo_instance_create
  │
  ├─(InitContext 携带 wire_placement 传入插件)
  ▼
插件 (FaceRecognizer / GenericDetector)
  │
  ├─(请求模型加载: RknnSharedWeightProvider::acquire_or_create)
  │     │
  │     ├── 若 Root 已驻留: 升级 Weak<RknnRootWeight> 引用 (零物理内存开销)
  │     └── 若 Root 未驻留: 调用 rknn_init 创建单一根权重上下文 (独占加载)
  │
  ├─(调用 rknn_dup_context(&root.ctx, &mut child_ctx))
  │     │
  │     └── 成功派生轻量 Child Context (共享物理权重，独立私有 workspace)
  │
  ├─(封装为 RknnChildSession: Send + !Sync 胶囊)
  ▼
实例 Worker 线程
  │
  ├── 独占绑定核心: rknn_set_core_mask(child_ctx, core_mask)
  ├── 独占分配/绑定输入输出: rknn_inputs_set / rknn_run / rknn_outputs_get
  └── 无锁并发推理 (多实例互不阻塞，彻底移除全局串行 Actor)
  │
(实例析构时)
  │
  ├── 实例先销毁: rknn_destroy(child_ctx)
  └── Root 引用计数归零: 安全调用 rknn_destroy(root.ctx) 释放物理显存
```

---

## 2. 详细设计与核心数据结构

### 2.1 InitContext 扩展与无污染设核 (`crates/algo-sdk/src/plugin.rs`)
```rust
#[derive(Debug)]
pub struct InitContext<'a> {
    pub package_root: &'a Path,
    pub platform_id: &'a str,
    pub instance_id: &'a str,
    pub is_self_test: bool,
    pub fallback_policy_override: Option<FallbackPolicy>,
    /// 宿主注入的硬件放置决策 (可选，保持向后兼容)
    pub wire_placement: Option<WirePlacementMetadata>,
}

impl<'a> InitContext<'a> {
    pub fn target_core_mask(&self) -> Option<u32> {
        self.wire_placement.as_ref().map(|p| p.core_mask)
    }

    pub fn with_placement(mut self, placement: Option<WirePlacementMetadata>) -> Self {
        self.wire_placement = placement;
        self
    }
}
```

- **消除环境变量进程污染**：严禁调用 `std::env::set_var("RKNN_CORE_MASK", ...)`，该函数非线程安全且会影响同一进程内的所有其他 Worker。
- **逐实例核心掩码控制**：在创建 `RknnChildSession` 或 `RknnSession` 时，直接在实例所属 Worker 线程调用 `rknn_set_core_mask(ctx, mask)`。

### 2.2 路线 A 共享权重提供者 (`crates/algo-sdk/src/runtime/platforms/rockchip.rs`)

#### 动态符号绑定
```rust
pub type PfnRknnDupContext = unsafe extern "C" fn(ctx_in: *mut RknnContext, ctx_out: *mut RknnContext) -> c_int;

pub struct RknnSymbols {
    // ... 已有符号
    pub rknn_dup_context: Option<PfnRknnDupContext>,
}
```

#### Root 权重与 Child 会话
```rust
/// 物理模型权重的根持有者
pub struct RknnRootWeight {
    pub ctx: RknnContext,
    pub symbols: Arc<RknnSymbols>,
    pub model_path: PathBuf,
}

impl Drop for RknnRootWeight {
    fn drop(&mut self) {
        if self.ctx != 0 {
            unsafe {
                (self.symbols.rknn_destroy)(self.ctx);
            }
        }
    }
}

// SAFETY: 路线 A 保证 Root Context 在创建后只用于派生 Child，派生过程受互斥锁保护，
// 且仅在所有 Child 析构后由最后的 Arc 持有者执行 Drop。
unsafe impl Send for RknnRootWeight {}
unsafe impl Sync for RknnRootWeight {}

/// 派生自 Root 的实例专用上下文
pub struct RknnChildSession {
    pub root: Arc<RknnRootWeight>,
    pub child_ctx: RknnContext,
    pub symbols: Arc<RknnSymbols>,
    pub core_mask: u32,
}

// SAFETY: RknnChildSession 严格独占绑定至单一 Worker OS 线程，实现 Send 以支持一次性移交，
// 绝不实现 Sync，杜绝跨线程并发访问同一个 context。
unsafe impl Send for RknnChildSession {}

impl Drop for RknnChildSession {
    fn drop(&mut self) {
        if self.child_ctx != 0 {
            unsafe {
                (self.symbols.rknn_destroy)(self.child_ctx);
            }
        }
        // self.root 在这里随 Arc drop，若当前是最后一个引用则自动销毁 root
    }
}
```

#### 共享权重池 (`RknnSharedWeightProvider`)
```rust
pub struct RknnSharedWeightProvider {
    pools: Mutex<HashMap<PathBuf, Weak<RknnRootWeight>>>,
}

impl RknnSharedWeightProvider {
    pub fn global() -> &'static Self;

    pub fn acquire_child_session(
        &self,
        symbols: &Arc<RknnSymbols>,
        model_path: &Path,
        core_mask: Option<u32>,
    ) -> Result<RknnChildSession, AlgoError>;
}
```

### 2.3 算法包解耦：移除人脸识别与 YOLO 中的全局串行 Actor

在 `algo-packages/rknn/rk3588/face_recognition`:
- **现状缺陷**：原设计使用 `static SHARED_MODELS: OnceLock<Mutex<HashMap<PathBuf, Weak<SharedModels>>>>`，其中包含一个全局 `InferenceWorker` 队列，所有摄像头的 `FaceRecognizer` 实例向同一个队列发送 `InferenceRequest` 串行执行，导致多核 NPU 算力闲置；
- **重构方案**：
  1. 废弃全局串行队列；
  2. `FaceRecognizer` 初始化时，通过 `RknnSharedWeightProvider` 直接派生专属的 YOLOv8-Face 检测与 EdgeFace 特征提取 `RknnChildSession`；
  3. 各实例在自己的专用 OS Worker 线程中直接调用 `rknn_run`，各摄像头完全独立并行推理；
  4. 底层物理权重通过 `RknnSharedWeightProvider` 维持单份驻留，既省显存，又完全释放多核并行吞吐！

### 2.4 C ABI 回执生成与宿主校验

在 `crates/algo-sdk/src/macros.rs`：
- 在插件导出宏中，实现 `av_algo_get_placement_extension`，返回全局静态 `AvAlgoPlacementExtensionV1` 表；
- `av_algo_instance_create` 成功后，构建 `AvAlgoInstanceReceiptPod` 并交由宿主读取；
- `av_algo_instance_destroy` 析构时，生成 `AvAlgoCleanupReceiptPod`，完成生命周期闭环。

---

## 3. 错误处理与硬核安全规范

1. **不可翻越安装自检硬门**：
   - 当 `is_self_test = true` 时，策略恒为 `RequireHardware`；
   - 若目标设备物理上不支持 NPU 或 `rknn_dup_context` 派生失败，严格抛出 `AlgoError::ModelLoad`（对应 C ABI `-5`），严禁静默回退到 CPU。
2. **所有权严格单向移交**：
   - 杜绝两个线程共享同一个 `RknnContext` 裸句柄；
   - 实例内部保证：派生 -> 一次性 Move 到实例线程 -> 在该线程内部设核 -> 在该线程内部推理 -> 在该线程内部销毁。
