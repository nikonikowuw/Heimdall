# FFI 边界

适用于平台 SDK 与插件 ABI；具体插件布局见 [算法 SDK](./algo-sdk-guidelines.md)。

## 封装

`C ABI / sys → FFI 包装与 RAII → 安全领域接口`。

- `unsafe` 与防御性规则见 [全局约定](../guides/conventions.md#防御性错误处理)；业务层不感知 C 类型。
- C/C++ 垫片只处理必要的驱动初始化与参数转发，不承载业务或调度。

## ABI

- 仅使用 `extern "C"`、不透明句柄、固定布局 POD 与固定宽度数值；Rust 结构使用 `#[repr(C)]`。
- 不跨界传 C++ 类、STL 容器、Rust 默认布局类型；不依赖名称修饰或隐式 packing。
- panic 不跨 FFI 边界传播（见 [全局约定](../guides/conventions.md#防御性错误处理)）。
- 双侧断言 `size_of/sizeof`、`align_of/alignof`、关键 `offset_of/offsetof`；改字段同步生产者、消费者和布局测试。
- 原始错误码在绑定层转换，保留操作名与 SDK 原码，不向安全层暴露 C 类型。

## 生命周期

- 创建/销毁成对，句柄用 RAII，检查创建失败后是否仍有部分资源需要释放。
- `CString` 必须绑定具名变量覆盖 FFI 调用期；禁止临时 `CString::new(...).as_ptr()`。
- `Send`/`Sync` 仅在 SDK 契约允许时实现；可转移不等于可并发，同一 session 绑定所属 Worker。
- 明确借用/转移、retain/release 与回调数据有效期；不得把回调期裸指针存入异步任务。
- DMA-BUF fd、stride、cache sync 与设备读回遵循 [媒体管线](./media-pipeline.md)，超时隔离遵循 [并发模型](./concurrency-guidelines.md#停机)。
- 设备侧快照编码与裁剪（MPP/RGA/VT/DVPP）FFI 规范：
  - DMA-BUF 跨硬件 IP 流转后，必须执行 `DMA_BUF_IOCTL_SYNC` 保证写入 Cache 刷新至物理内存，杜绝读脏导致花屏或绿屏；
  - 动态裁剪尺寸需在驱动层显式重配（如 MPP `MPP_ENC_SET_CFG` 重配 prep 步长与宽高）；
  - Apple VideoToolbox 严禁逐帧重建 Session，特写尺寸走固定画板或轻量处理；
  - 昇腾 DVPP 严格由 `acldvppMalloc` 64 字节对齐分配，单实例串行固定预分配双缓冲池。

## 构建与验证

薄 C 垫片由对应 crate 的 `build.rs` 通过 `cc` 编译，声明 `cargo:rerun-if-changed`；SDK 路径通过配置发现，不硬编码绝对安装路径。
验证 ABI 版本/尺寸/偏移、空指针、部分初始化失败、重复销毁防护与错误提前返回后的释放。
测试规则见 [全局约定](../guides/conventions.md#测试)。
