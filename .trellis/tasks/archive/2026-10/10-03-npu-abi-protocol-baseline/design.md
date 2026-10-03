# NPU 跨层协议、有界清理与路线 A/B 验证技术设计

> 所属任务：`10-03-npu-abi-protocol-baseline`（阶段 A0 & A）。

## 1. 架构边界与职责

本任务负责定义并冻结宿主（`crates/infer`）、算法 SDK（`crates/algo-sdk`）以及原生插件之间的跨层交互边界：

```text
Host (crates/infer)                  Plugin (crates/algo-sdk / C++)
  InferenceWorker                      AvAlgoAbi (96 字节稳定布局)
     |                                          |
     +-- [可选扩展符号加载] --------------------->+-- AvAlgoPlacementExtension (可选 v1)
     |                                          |     query_capabilities()
     |                                          |     prepare_weight_root()
     |                                          |     release_weight_root()
     |                                          |     query_instance_receipt()
     |                                          |     query_cleanup_receipt()
     |                                          |
     +-- [配置注入: wire json] ---------------->+-- export_algo! 宏
     |     __heimdall_placement                 |     安全剥离 placement
     |                                          |     剩余 json 传插件 Config
     |                                          |
     +-- [退出协议重构] ------------------------+
           backend.cleanup() 在 Worker 线程内
           发送 Finished 确认
           超时隔离 (QuarantineSupervisor)
```

---

## 2. C ABI 可选 Placement 扩展设计

### 2.1 保持 96 字节基础 ABI 不变
`AvAlgoAbi` 保持 64 位系统下 96 字节固定布局，禁止在原有结构体中追加字段或改变函数指针偏移。

### 2.2 加法式可选扩展符号
借鉴现有的可选画廊扩展（`gallery`），在动态库导出表中引入独立的符号查询接口：

```rust
// FFI 导出符号名称
pub const AV_ALGO_PLACEMENT_EXTENSION_SYMBOL: &[u8] = b"av_algo_get_placement_extension\0";

#[repr(C)]
pub struct AvAlgoPlacementExtensionV1 {
    pub struct_size: usize,
    pub version: u32,
    pub reserved: u32,

    // 1. 能力查询：返回插件是否支持执行隔离与权重共享
    pub query_capabilities: unsafe extern "C" fn(
        out_caps: *mut AvAlgoPlacementCapsPod,
    ) -> i32,

    // 2. 权重根生命周期管理 (控制面专用)
    pub prepare_weight_root: unsafe extern "C" fn(
        req: *const AvAlgoWeightRootReqPod,
        out_receipt: *mut AvAlgoWeightRootReceiptPod,
    ) -> i32,

    pub release_weight_root: unsafe extern "C" fn(
        weight_id: *const c_char,
        generation: u64,
    ) -> i32,

    // 3. 执行实例回执查询 (实例 Worker 内读取)
    pub query_instance_receipt: unsafe extern "C" fn(
        instance_handle: *const c_void,
        out_receipt: *mut AvAlgoInstanceReceiptPod,
    ) -> i32,

    // 4. 清理回执查询 (supervisor 在实例销毁后读取，不依赖已销毁指针)
    pub query_cleanup_receipt: unsafe extern "C" fn(
        reservation_id: *const c_char,
        out_receipt: *mut AvAlgoCleanupReceiptPod,
    ) -> i32,
}
```

### 2.3 内存与 POD 契约
* 所有入参/出参均为定长 POD 结构体，由调用方（宿主）在栈上或安全堆上分配，插件仅负责按大小填入数据；
* 字符串采用定长 UTF-8 缓冲区（如 `[u8; 128]`）及显式字节长度字段；
* 宿主与插件在编译期和单元测试中同时运行 `size_of::<T>()`、`align_of::<T>()` 以及关键字段的 `offset_of!()` 断言。

---

## 3. Worker 退出协议重构与隔离机制 (修复 H01 / T01–T03)

### 3.1 现状与根本缺陷
* `worker.rs:526-540` 在停止时：主线程通过 channel 发送退出命令后，立即进入 `thread.join()`。如果 backend 在 Drop 阶段出现 FFI 死锁、TLS 析构卡死或驱动挂起，主线程将**无限期卡死**。
* `worker.rs:457-471` 在初始化失败或启动通道断开时，同样存在直接 `join()` 的风险。

### 3.2 有界清理与锁外 Reaper 设计
1. **所属线程先清理**：
   * Worker 专用线程接收到 `Stop` 指令后，首先显式执行 backend 的资源释放与 C ABI 清理，确保所有的 GPU/NPU 上下文在创建它的 OS 线程内完成析构；
   * 清理完成后，向控制端发送 `ExitReceipt::Cleaned` 信号。
2. **有界等待与隔离转移**：
   * 宿主控制端设置有界超时（默认 3 秒）；
   * 若超时时间内收到退出信号且线程顺利退出，正常回收资源；
   * 若发生超时或通道异常断开，控制端**绝不无限等待 join**，而是将 Worker 句柄移交至全局 `QuarantineSupervisor` 观察队列，打上 `Quarantined` 标签并计入熔断指标；
   * 宿主调用方立即收到明确的 `Timeout` 错误，避免上层阻塞。
3. **Reaper 锁外处理**：
   * 后台 Reaper 线程在互斥锁外部定期轮询 `Quarantined` 句柄的真实结束状态，确认退出后再从账本扣减占额，禁止持锁执行阻塞 join。

---

## 4. Wire 数据协议与兼容性设计

### 4.1 `__heimdall_placement` 结构
宿主通过扩展参数将放置意图下发给插件：

```json
{
  "__heimdall_placement": {
    "version": 1,
    "reservation_id": "boot-1:0",
    "group_id": "grp-cam1-det",
    "generation": 1,
    "device_id": "rknn-npu0",
    "runtime_device_index": 0,
    "strategy": "pinned",
    "core_mask": 2,
    "required": true,
    "weight_sharing": "required",
    "weight_bindings": [
      {
        "model_key": "yolov8n",
        "weight_id": "w-yolov8n",
        "generation": 1
      }
    ]
  }
}
```

### 4.2 SDK 宏安全剥离机制
* 插件 `export_algo!` 宏在调用插件自身 `Config::deserialize` 之前，先将原始 JSON 反序列化为 `serde_json::Value`；
* 检查并 `remove("__heimdall_placement")`，将提取出的 placement 数据包装为 `PlacementContext`；
* 剥离后的 JSON 对象再反序列化为插件的业务 `Config`；
* **收益**：即便业务 `Config` 标注了 `#[serde(deny_unknown_fields)]`，也不会因宿主新增的 placement 元数据而反序列化失败！

---

## 5. A0 路线 A 与路线 B 判定标准与实测探针

### 5.1 首选路线 A（设计主线）
* **胶囊设计**：
  ```rust
  // 独占所有权，只能跨线程转移，不能并发共享
  pub struct RknnChildContextCapsule {
      raw_ctx: rknn_context,
  }
  unsafe impl Send for RknnChildContextCapsule {}
  // 坚决不 impl Sync！
  ```
* 控制 Worker 串行执行 `rknn_dup_context` 并封装为胶囊，通过有界 channel 一次性移交实例 Worker；
* 实例 Worker 内部独占调用 `rknn_set_core_mask`、`rknn_inputs_set`、`rknn_run`、`rknn_outputs_get` 以及 `rknn_destroy`。

### 5.2 板端 4 条判定标准实测（T42）
建立独立的最小板端验证探针，逐项测试：
1. **跨线程设核与推理**：非创建线程能否顺利 `set_core_mask` 并推理成功？
2. **多实例并发独立性**：兄弟实例分别在 Core 0 / 1 / 2 并发推理，有无内存覆盖与串帧？
3. **独立销毁安全**：销毁实例 A 的 child context，实例 B/C 是否继续正常运行？
4. **根 context 保活**：所有 child 销毁前，root 是否稳定可用；先 child 后 root 释放是否无内存泄漏？

若路线 A 在目标 BSP 上全绿，则正式锁定路线 A；若出现跨线程 TLS 段错误，则切换为后备路线 B（实例线程内加锁 dup）。无论哪条，严禁使用全局推理 Mutex。
