# 全局约定

跨层硬约束集中定义于此；专题文件引用本页，不再重复展开。

## 时间

- 内部绝对时间戳：**13 位 UTC Unix 毫秒**（Rust `i64` / TypeScript `number`）。
- 相对时长带 `Ms` 后缀（`timeoutMs`、`latencyMs`）。
- ABI 边界纳秒换算只在 [算法 SDK 帧契约](../backend/algo-sdk-guidelines.md#帧契约) 进行（1ms = 1,000,000ns）。
- 时间戳对应源帧，不替换为回调到达时间或图片完成时间。

## 坐标

- 全系统唯一坐标契约：**`[x1, y1, x2, y2]` 归一化对角两点式**，取值 `[0.0, 1.0]`，满足 `x1 <= x2`、`y1 <= y2`。
- ROI / Mask / Line 传输与落库均为 `[0, 1]`；在动态实时流画面上叠加矢量交互层绘制，禁止静态死图标注。
- 坐标经 `unmap_box` 去 padding / 逆缩放后归一化；正逆变换共用布局参数。

## 队列与通道

- 所有帧路径、事件缓冲、批次和缓存必须有**固定容量上限**。
- 帧路径使用 `crossbeam_channel::bounded`（通常 1～4），满载**丢旧不阻塞**，不阻塞硬件解码反压。
- 媒体压缩流分发使用 `PacketDispatcher` + 独立有界 `ConsumerMailbox` 隔离（单流/全局预算限制，满载清空并立即 Replay 完整 GOP，慢客户端不反压或拖慢正常消费端）。
- 普通控制面事件与告警通知广播使用有界 `tokio::sync::broadcast`（`Lagged` 丢弃旧消息）；异步/同步任务控制用有界 `tokio::sync::mpsc`。
- 写盘经有界通道攒批提交，禁止逐事件单事务刷盘。

## 边缘资源与内存预算

- **CMA 连续物理内存**：受限平台（如 RK3568 CMA 仅 16MB）多模型算法通过单例 Actor 或算力租约（`AlgoLease`）复用常驻上下文，杜绝频繁初始化销毁导致 CMA 碎片化。
- **DMA-BUF 映射权限**：DMA-BUF 映射为张量虚拟地址时必须声明 `PROT_READ | PROT_WRITE`，严防 `rknn_inputs_set` 触发内核缺页写保护段错误。
- **热路径零动态分配**：热路径预分配并复用缓冲，杜绝逐帧 `format!`、临时像素 Vec 与模型重载；FFI 及超 1ms 计算必须分流至固定专用 OS Worker。

## 跨层数据流与 DTO 契约

- **流转原则**：数据按 `来源 → 转换 → 存储 → 消费` 单向流转；边界处（HTTP/WS/FFI）完成入口校验与收窄，内部链路复用类型，不重复猜测 payload。
- **DTO 映射**：字段与 Rust serde 的 camelCase 逐字对齐，`Option<T>` 严格对应 `T | null`，事件 ID 仅生成一次且全局复用（落库、通知与幂等）。
- **空间坐标变换闭环**：`原图 → 缩放/letterbox → 模型输出 → 去 padding / 逆缩放 → 归一化 → 视频内容区`；正逆变换共用布局参数，三平台输出格式一致。

## 资源释放与 RAII

- fd、设备句柄、内存池租约和模型在正常、错误、停机路径均释放。
- C ABI 创建/销毁成对，RAII 保证错误路径释放。
- `CString` 绑定具名变量覆盖 FFI 调用期；裸指针不逃逸到安全层。

## 外部命令

- 解析外部命令输出时强制 `LC_ALL=C.UTF-8` + `LANG=C.UTF-8`，统一经模块内的 `run_command_with_c_locale` helper 注入，不在调用点各写一套。
- 仅 `LC_ALL=C` 不足够：`C` 语言环境会把非 ASCII 名称降级成占位字符（glib 的 `g_print` 走 locale 转换）；`C.UTF-8` 自 glibc 2.35 起内建，旧 BSP 缺失时 glibc 只往 stderr 打 warning 并回退到 `C`，修复会静默失效（板端以 `locale -a | grep -i 'c\.utf'` 确认）。
- 解析一律取机器可读列（UUID、`ipv4.*`、`connection.interface-name`），不依赖人类可读文本、本地化连接名或 `DEVICE` 等与激活状态绑定的列。注意：这些 `setting.property` 只能用于 detail 模式，见下条。
- **nmcli 字段命名空间严格分离**：`nmcli connection show` 列表模式只接受元字段（`NAME`/`UUID`/`DEVICE`/`ACTIVE`…），而 `connection.interface-name`、`ipv4.method` 等 `setting.property` 只在 `nmcli connection show <ID>`（detail 模式，带 ID 实参）下有效。混用直接报 `invalid field 'connection.interface-name'; allowed fields: NAME,UUID,TYPE,...`（RK3568 板端实证）。
- 另一个坑：`DEVICE` 元字段**仅对活跃连接**取值，未激活 profile 恒为 `--`。查“网卡绑定了哪个 profile”必须走 detail 模式的 `connection.interface-name`，不能用列表模式的 `DEVICE` 兜底。

## 防御性错误处理

- `unwrap`/`expect` 仅用于启动、测试或有明确证明并注释的不变量；逐帧和可恢复故障路径禁止使用。
- C++ 异常 / Rust panic 不得跨 FFI 边界传播；FFI 入口用 `catch_unwind` 隔离。
- 每个 `unsafe` 块标注 `// SAFETY:`；不以静默回退伪装硬件成功。

## 测试

- 硬件依赖测试标记 `#[ignore]`，按平台 feature 隔离；开发机 `cargo test` / `pnpm test` 仍须全绿。
- 声明支持不等于已完成真机验证；性能结论必须附对应环境记录。

## 存储保护

- 基于 `statvfs` 监控容量、inode 与只读状态，禁止 `du` 递归扫描。
- 淘汰优先清除无告警普通抓拍；物理文件与 DB 记录在同一事务内配套删除（"图在案在，图销案销"）。
