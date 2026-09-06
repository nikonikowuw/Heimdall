# 系统设置续接记录

日期：2026-09-06。代码基线：`646a60d`。本记录只恢复规划上下文，不代表用户已批准实现。

## 已确认的需求与停点

- 历史会话：Pi `01a07470-e2f1-72b5-a5fa-c737aa6f81ff`，通过 `trellis mem extract` 读取。
- 用户已确认单管理员模型，以及系统概览、账号与安全、网络/服务、存储与保留策略、对时服务五个分组。
- 用户已选择前后端一起实现，并明确授权创建 Trellis 任务、进入规划。
- 此前将网络配置理解为应用监听地址和端口是误解。用户在续接后明确纠正：“我要的网络配置是可以修改边缘设备的网络 ip 的”。网络需求已改为设备实际网卡 IP 配置；旧的监听配置提案不再作为该需求的候选方案。
- 用户随后追问通用 Linux 网络配置方式。技术规划调整为统一入口、运行时识别网卡管理服务并适配；具体发行版名称不再作为继续规划的前置问题。通用机制与持久化边界见 `research/linux-network-configuration.md`。
- 当前 `task.json.status` 为 `planning`，只有 PRD 和两个 seed-only JSONL；尚无 `design.md`、`api.md`、`implement.md`。
- 本次会话没有 current-task 指针；已按唯一未归档任务续接规划。不能为了绑定指针提前运行 `task.py start`。

## 代码事实

| 事项 | 证据 | 对设计的影响 |
| --- | --- | --- |
| 应用配置在启动时加载，随后应用 CLI 覆盖 | `crates/app/src/main.rs:45`；`crates/app/src/config.rs:264` | 持久化设置需要明确与配置文件、环境变量、CLI 的优先级，不能只落库而不接消费方。 |
| HTTP/WebSocket 监听地址在启动时解析并绑定 | `crates/app/src/main.rs:169`；`crates/app/src/main.rs:173` | 这是应用监听行为，不能用于推导设备网卡 IP 的修改方式或生效时机。 |
| 未发现设备网络配置适配代码或网络服务部署约定 | 在 `crates/`、`docs/`、`.trellis/spec/`、现有任务及配置中搜索 NetworkManager/nmcli、netplan、systemd-networkd/networkctl、connman、dhcpcd/dhclient、ifupdown、DHCP、网卡等 | 后端需检测目标网卡的实际管理服务和能力，再执行配置读取、持久化、应用和恢复；不能仅凭发行版名称、命令存在或服务正在运行推断网卡归属。 |
| 日志过滤器在启动时初始化，读取 `RUST_LOG` 或 `logging.filter` | `crates/app/src/main.rs:59` | 当前没有 reload handle；`logging.level` 虽在 `crates/app/src/config.rs:168` 定义，却未用于此初始化。不能把修改这个字段直接描述为可用的在线调级能力。 |
| API 共享状态没有通用运行时设置句柄 | `crates/api/src/state.rs:25` | 设置查询与应用需由 app 装配控制句柄，API 不应直接控制监听器或硬件。 |
| 系统配置仓储已有按 key 读写能力 | `crates/db/src/repository/system_config.rs:10`；`crates/db/src/repository/system_config.rs:27` | 可复用存储，但要按分组定义类型和允许写入的键；现有表还保存认证密钥，不能提供任意键枚举/更新接口。 |
| 应用存储配置目前只有证据目录；清理器另有水位配置 | `crates/app/src/config.rs:86`；`crates/pipeline/src/storage_cleaner/mod.rs:106` | 保留天数与水位的可编辑范围、消费方接线需要单独定义。 |
| 没有发现 NTP/chrony/timesyncd 或修改系统时钟的服务实现 | 搜索 `crates/` 中 `clock_settime`、`timedatectl`、`chrony`、`timesyncd`、`ntpd`；现有 NTP 提及主要是环形缓冲的时间回跳防护 | 对时服务的产品范围和宿主服务边界仍需确认，不能视为已有能力。 |

## 已发现的文档差异

- `.trellis/spec/backend/index.md` 的配置示例包含 `storage.retention`，但当前 `AppConfig` 没有对应字段，不能据此声称已支持按天保留。
- 历史建议把日志级别列为可在线编辑项；代码事实是尚无动态应用路径。后续设计必须补齐消费方或明确生效条件。
- PRD 已补充 R5 和设备网络验收条件，并修正“所有配置只写入应用系统配置存储”的歧义；网卡配置必须在宿主操作系统上真实生效且重启后保持。
- 本次先记录事实；实现与验证完成后，再按 spec 更新流程补回正式约定。

## 已完成的调研与设计（2026-09-06 续接）

### 1. Linux 网络管理服务适配技术调研
- **产出**：`research/linux-network-adaptation.md`
- 网卡管理服务识别矩阵（NetworkManager / systemd-networkd / Netplan / ifupdown / connman / Unmanaged）
- 各服务的读写能力和持久化机制
- 后端 `NetworkService` trait 定义
- 首版适配范围：P0 = NetworkManager + systemd-networkd，P1 = Netplan
- 权限与安全：需 `CAP_NET_ADMIN` 或 polkit 授权

### 2. 设备网络表单与应用流程设计
- **产出**：`research/network-form-and-flow.md`
- 网卡数据模型（`NetworkInterface`, `IpConfig`, `InterfaceCapabilities`）
- 表单字段与校验规则
- 管理接口 IP 变更完整时序（含断连指引、新地址确认、超时恢复）
- 操作状态模型（`pending_confirm` → `confirmed` / `restored`）
- API 端点设计（6 个端点）和错误码（51001–51010）
- 超时恢复机制：后端 30s 定时器检查，10 分钟确认窗口
- 前端组件结构和状态管理方案

### 3. 存储保留策略与对时服务调研
- **产出**：`research/storage-and-time-sync.md`
- **已决策**：采用工业级 NVR/VMS 存储模型（对齐海康/大华配额模式）
- 存储策略四层架构：按类型配额 → 按类型保留天数 → 磁盘水位安全网 → 循环覆盖
- 三种证据类型独立配置：告警图(30天)、识别图(14天)、抓拍图(7天)
- 配额模式：`overwrite`(循环覆盖) / `stop`(写满停止)
- 配置热更新：`StorageCleaner.config` 改为 `Arc<RwLock<>>`，即时生效无需重启
- 13 个可编辑字段，全部即时生效
- **已决策**：对时服务支持手动设置时间（含安全约束）
- NTP 适配：timedatectl 通用入口 + chrony/timesyncd 深度适配
- 手动设置时间安全措施：时间差≤1年、二次确认、操作日志审计、NTP 启用时警告
- 后端 `TimeService` trait 定义（含 `set_system_time`）

## 下一步

1. 基于三份调研产出，编写 `design.md`（技术设计文档）和 `api.md`（API 契约）
2. 用户确认最终规划后，创建执行计划 `implement.md`
3. 启动任务进入实现

本轮更新了规划记录和调研文档，未修改产品代码；未运行 Rust/Web 格式化、lint、测试或构建。
