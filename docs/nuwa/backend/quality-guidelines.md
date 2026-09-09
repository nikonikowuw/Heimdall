# 后端质量检查

## 门禁

按 [AGENTS.md](../../../AGENTS.md) 执行 Rust/Native 门禁：先格式化，再格式检查、Clippy（`-D warnings`）和 workspace 测试。
仅在具备对应 SDK 时增加 all-features 与平台测试；硬件用例必须 `#[ignore]`，开发机默认测试全绿。
文档变更检查格式、链接和引用，不要求运行无关产品测试。

- workspace lint 保持 `unsafe_op_in_unsafe_fn`、`undocumented_unsafe_blocks` 等检查，`Cargo.lock` 入库。
- 不提交 `dbg!`、`println!`、`todo!()` 或关闭 lint 的规避；不修改断言迎合错误行为。
- 新依赖必须说明必要性及体积/构建代价；完整工具配置以 [Cargo.toml](../../../Cargo.toml) 为准。

## 测试选择

| 变更                    | 验证重点                                                         |
| ----------------------- | ---------------------------------------------------------------- |
| NMS、几何、时间等纯计算 | 空值、极值、非等比缩放、贴边与微小框；正逆坐标共用参数并往返一致 |
| Repository              | 独立临时 SQLite 文件，WAL、迁移、查询上限、事务与文件清理        |
| API                     | `tower::ServiceExt::oneshot` 验证状态码/信封/鉴权，不开物理端口  |
| FFI / C ABI             | 双侧尺寸、对齐、偏移、版本与错误释放                             |
| Worker / 资源           | 满队列、断开、取消、超时隔离、重复启停                           |
| 硬件后端                | 平台 feature + `#[ignore]`，记录设备/SDK 与误差范围              |
| Bug                     | 先建立稳定失败的回归用例，再修复                                 |

测试优先覆盖可观察行为、边界转换与资源清理；不为简单可逆文案/样式改动新增重复实现的测试。

## 审查重点

检查 [资源预算](../guides/edge-constraints-guide.md)、[并发](./concurrency-guidelines.md)、[FFI](./ffi-guidelines.md) 与 [跨层契约](../guides/cross-layer-thinking-guide.md)。
重点核对错误/取消路径的 RAII、无阻塞反压、热路径预分配、平台隔离和准确的 SAFETY 说明。

## 既有系统服务的回归点

| 场景         | 契约与实现入口                                                                                                                |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------- |
| 存储         | `AppState::with_storage_cleaner` 注入 Pipeline 相同证据目录；`totalGb` 等字段保持 camelCase，读取失败返回 `51300`，不伪造成功 |
| macOS CPU    | `host_processor_info(PROCESSOR_CPU_LOAD_INFO = 2)`，不能用 flavor 1；两次采样 delta，结果限制 `0..=100`                       |
| macOS uptime | `kern.boottime` 使用完整 `libc::timeval`（64 位为 16 字节）；`uptimeSeconds` 是持续秒数，不是当前 Unix 时间戳                 |
| macOS 网络   | 只读，`canModifyIp/canSetDhcp = false`；`ifconfig -a` 至少含 `lo0`，`networksetup` 补充信息，外部命令隔离到阻塞任务           |

实现位置：[AppState](../../../crates/api/src/state.rs)、[system_info.rs](../../../crates/api/src/system_info.rs)、[network_service/macos.rs](../../../crates/api/src/network_service/macos.rs)。
这些既有路径不改变仓库的平台边界要求，新增功能不得继续向 Handler 扩散平台逻辑。

交付说明列出实际检查结果、未验证项与硬件限制，不把 spec 中的目标阈值当作实测结果。
