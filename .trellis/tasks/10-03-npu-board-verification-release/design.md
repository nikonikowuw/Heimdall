# NPU 多核分配板端性能基线与发布验收 (Technical Design)

## 1. 物理链路与芯片级执行架构 (Mechanism & Hardware Pipeline)

在 Rockchip NPU (RKNPU2) 异构芯片微架构上，RK3568 具备 1 个物理 NPU 核心 (1.0 TOPS)，RK3576 具备 2 个物理 NPU 核心 (Core0, Core1, 共 6.0 TOPS)，RK3588 具备 3 个物理 NPU 核心 (Core0, Core1, Core2, 共 6.0 TOPS)。

在采用物理共享权重架构前，每个算法实例若独立调用 `rknn_init`，驱动会在 CMA (Contiguous Memory Allocator) 或特定 ION 堆中完整映射并重分配模型的权重常数段（Weight/Bias Tensor），导致显存呈 $O(N)$ 线性膨胀，并在并发初始化时产生严重总线带宽竞争。

采用新架构后，硬件物理流转路径如下：

```
[ 宿主进程 (Control Plane Thread) ]
        |
        | 1. rknn_init (加载物理模型权重，仅执行 1 次)
        v
[ RknnRootWeight (物理权重显存常驻 CMA，独占 Root Context) ]
        |
        | 2. rknn_dup_context(root_ctx, &child_ctx) (轻量浅拷贝句柄，派生子会话)
        +-----------------------------------------------+
        |                                               |
        v (路线 A: 独占句柄移交 Worker 0)                v (路线 A: 独占句柄移交 Worker 1)
[ 实例 Worker 0 线程 ]                           [ 实例 Worker 1 线程 ]
  - rknn_set_core_mask(child_0, 0x01/Core0)        - rknn_set_core_mask(child_1, 0x02/Core1)
  - 绑定独占输入输出缓冲区 (私有 IO)                - 绑定独占输入输出缓冲区 (私有 IO)
  - 独占私有 workspace (内部中间激活值)              - 独占私有 workspace (内部中间激活值)
  - rknn_run(child_0)                              - rknn_run(child_1)
        |                                               |
        v                                               v
[ NPU Core 0 执行硬件算子 ]                      [ NPU Core 1 执行硬件算子 ]
        \                                               /
         \                                             /
          +--> [ 共享物理只读权重 (CMA 零冗余，同一物理地址) ] <--+
```

## 2. 路线 A 线程归属判定设计 (Route A Verification Design)

### 2.1 路线 A 核心逻辑
- **所有权转移**：`RknnRootWeight` 驻留在控制层或服务生命周期中，派生出的 `RknnChildContext` 包装为具备 RAII 的独占类型（实现 `Send` 但非 `Sync`），直接 `move` 进专属的 OS Worker 线程。
- **线程本地控制**：在实例 Worker 线程执行 `rknn_set_core_mask` 和后续的单帧 `rknn_inputs_set`、`rknn_run`、`rknn_outputs_get`。
- **生命周期顺序**：当实例停止时，Worker 线程内部执行 `rknn_destroy(child_ctx)`；当所有实例均已释放且模型卸载时，控制侧执行 `rknn_destroy(root_ctx)`。

### 2.2 四项裁决判据 (Four Criteria for Route A)

| 判据编号 | 验证目标 | 观测与断言方法 | 通过标准 |
| :--- | :--- | :--- | :--- |
| **C1** | 跨线程设核能力 | 控制线程 `dup`，移交至新建 `std::thread` 后调用 `rknn_set_core_mask` | 返回值为 `0 (RKNN_SUCC)` |
| **C2** | 跨线程推理能力 | 在该新建线程内执行 `rknn_run` 并取回输出 tensor | 成功返回 0，输出 tensor 校验和或数值有效 |
| **C3** | 并发隔离与正确性 | 启动 2 个 Worker 线程，分别绑定 Core0 与 Core1，同时并发推理 | 两路结果均有效，`/sys/kernel/debug/rknpu/load` 双核均有负载 |
| **C4** | 析构与生命周期安全性 | 先后 `rknn_destroy(child)`，最后 `rknn_destroy(root)` | 进程退出码 0，内核无 dmesg 报错，CMA 释放 |

若 C1~C4 全部满足，正式采纳路线 A（无需引入路线 B 的控制侧串行派生代理）。

## 3. 板端内存增量与多核负载取证架构 (Measurement Architecture)

为了获得无可辩驳的真实板端物理证据，设计专门的轻量板端探针程序 `board_npu_probe`，其指标采集点包括：

1. **CMA 与显存监控**：
   - 探针直接读取 `/proc/meminfo` 中的 `CmaAllocated`、`CmaReleased`、`CmaTotal`、`CmaFree`。
   - 读取 `/proc/self/status` 中的 `VmRSS`、`VmHWM`。
   - 读取 `/proc/self/smaps_rollup`（若支持）提取 `Pss`。
2. **NPU 核心利用率**：
   - 探针或外部采集脚本读取 `/sys/kernel/debug/rknpu/load`（单核显示单百分比，多核显示 `Core0: x%, Core1: y%`）。
3. **文件描述符与句柄监控**：
   - 检查 `/proc/self/fd/` 的 fd 总数，确保没有驱动 fd 泄漏。

### 对比实验设计

- **Baseline 1 (独立加载)**：
  - 循环 1~3 次调用 `rknn_init`，测量系统内存与 CMA 增加曲线。预期每个实例增加约 `ModelSize + WorkspaceSize`（如 15MB + 2MB = 17MB/实例，3 实例消耗 ~51MB）。
- **Baseline 2 (共享权重派生 - 路线 A)**：
  - 1 次 `rknn_init` + 3 次 `rknn_dup_context`，测量系统内存与 CMA 增加曲线。预期 Root 消耗 ~17MB，后续每个 Child 仅增加 ~1~2MB（私有 workspace 与 IO），3 实例总消耗 ~21MB（节约 ~60% 显存）。

## 4. 目标板配置 Profile 规范 (Target Board Profiles)

为彻底消解阻塞项 `B-TGT` 与 `B-CAP`，固化各平台硬件能力与并发预算上限：

```rust
pub struct BoardNpuProfile {
    pub platform: &'static str,
    pub num_cores: u32,
    pub supported_masks: &'static [u32],
    pub default_core_mask: u32,
    pub supports_weight_sharing: bool,
    pub max_instances_per_model: usize,
    pub cma_budget_mb: usize,
}

pub static RK3568_PROFILE: BoardNpuProfile = BoardNpuProfile {
    platform: "rk3568",
    num_cores: 1,
    supported_masks: &[0x01],
    default_core_mask: 0x01,
    supports_weight_sharing: true,
    max_instances_per_model: 4,
    cma_budget_mb: 16,
};

pub static RK3576_PROFILE: BoardNpuProfile = BoardNpuProfile {
    platform: "rk3576",
    num_cores: 2,
    supported_masks: &[0x01, 0x02, 0x03],
    default_core_mask: 0x01, // 宿主按 spread 算法分散调度
    supports_weight_sharing: true,
    max_instances_per_model: 8,
    cma_budget_mb: 48,
};

pub static RK3588_PROFILE: BoardNpuProfile = BoardNpuProfile {
    platform: "rk3588",
    num_cores: 3,
    supported_masks: &[0x01, 0x02, 0x04, 0x07],
    default_core_mask: 0x01,
    supports_weight_sharing: true,
    max_instances_per_model: 12,
    cma_budget_mb: 128,
};
```

## 5. 故障注入与安全回滚设计 (Fault Injection & Recovery)

1. **子会话失败与局部隔离**：
   - 若某子会话在 `rknn_run` 或设置 core mask 时失败，只影响该实例 Worker，通过 `Coordinator` 局部置为 `Failed` 并释放该子 context；根权重与其他兄弟实例不受任何影响。
2. **生命周期回收防御**：
   - 使用 RAII Guard 包装 `RknnChildContext`，其 `Drop` 实现保证优先调用 `rknn_destroy`；内部持有 `Arc<RknnRootWeight>` 的弱引用或强引用计数，保证只有所有 Child 均销毁后，Root 才调用销毁，杜绝 UAF (Use-After-Free)。
3. **驱动不可恢复故障防护**：
   - 若遇到硬件 NPU 驱动挂死（内核返回 `-13` / EACCES 等），记录硬件诊断错误，触发单任务熔断隔离，禁止无界重启或盲目 retry。
