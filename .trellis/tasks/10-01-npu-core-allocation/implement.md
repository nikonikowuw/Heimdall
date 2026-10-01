# 实施与验证计划

> planning / 待确认。用户当前只要求优化任务；此计划不是启动授权。

## 0. 开工门禁

- [ ] 用户确认 prd.md、design.md 和 api.md 最新摘要，特别是严格 manual、执行组粒度与可选 ABI 扩展。
- [ ] 保留工作区已有修改；本任务不混入前端抽屉任务。
- [ ] 按目标包读取 spec index/checklist；上下文见 implement.jsonl / check.jsonl。
- [ ] 用目标板证据核对错误码；不能把任意非零或裸 -13 当可恢复错误。
- [ ] 若拆分子任务，当前任务作为父需求/集成验收入口；子任务 A->B->C->D->E 的依赖需写入各自计划。本轮不自动创建或激活子任务。

## A. 冻结跨层协议与故障基线（R01,R02,R05,R06）

- [ ] 先建复现测试：环境变量覆盖实例、非法 mask 截断、重复共享初始化、过期代际释放、字段省略被抹掉。
- [ ] types 定义 affinity、运行态与错误；区分缺失/null，校验无效组合。
- [ ] 冻结 placement 可选扩展的精确 C 函数签名、版本协商、固定布局、载荷上限、回执聚合、错误码；SDK/宿主双侧断言和 fixture。
- [ ] 为老插件无扩展、拒绝未知字段、新插件无注入建立动态库兼容 fixture。
- [ ] 确认所有生产创建/提取/替换/自检调用图，形成测试清单。

Review gate：基础 AvAlgoAbi 及现有结构布局完全不变；新扩展契约已可由纯 mock 测试。未通过不可进入硬件接入。

## B. 宿主资源管理（R01,R03,R04,R08,R09）

- [ ] 复用 crates/infer/src/npu，建立 topology generation 与 capability；不新增重复 SoC 判断。
- [ ] 实现纯 PlacementSolver + 短锁 CoreLedger 原子提交；确定性同分轮转、严格 manual、幂等成员登记/释放。
- [ ] 引入 GroupOwner / reservation，独立于 AlgoLease；补充初始化 single-flight 和失败/取消守卫。
- [ ] 接入 Worker 启动超时、退出隔离、冷却、回收、包替换；未退出资源继续占额。
- [ ] 配置有限的组数/成员/启动并发/隔离/内存预算；尽量复用已有配置。
- [ ] 缓存只读诊断快照，限制指标基数和列表尺寸。

Review gate：在 mock backend 的所有失败点测试额度守恒；不持锁 IO/FFI/await，不产生无界状态或重试。

## C. SDK 与全部 RKNN 插件（R05,R06,R09,R10）

- [ ] SDK 宏剥离 __heimdall_placement，扩展 Rust InitContext；本地 runner、构造点与 update_config 同步。
- [ ] 使用版本化解析、checked narrowing、字段/位校验；业务 Config 不接触内部字段。
- [ ] 删除 SDK 和插件生产路径 RKNN_CORE_MASK 覆盖；搜索全部核心选择入口，不仅原计划列出的四个文件。
- [ ] shared_models 的 key、单航班启动和配置一致性按执行组实现；不把 mask 加入 key 来复制模型。
- [ ] 组内检测/识别等所有必要 session 回执聚合；设置失败按目标 profile 分类、有界回退。
- [ ] 接入离线提取成员保活与内部硬件线程退出；禁止请求 future 取消后提前释放 lease。
- [ ] 各平台源码/API 兼容测试，所有跨 ABI panic/指针/长度/清理路径覆盖。

Review gate：共享 3 路仍只有 1 个已管理组；独立组可分核；回执不冒充硬件采样；老插件 auto 可运行但 manual 明确不支持。

## D. 持久化、运行时收敛、API 与 Web 兼容（R07,R08,R10）

- [ ] 新增前向 affinity_json 迁移、entity/repository 类型；旧数据 auto，三态写入在已有事务内。
- [ ] 扩展任务配置类型及 InstanceLaunchConfig，所有生产 Worker 路径从同一管理入口取得组资源。
- [ ] 保留 configRevision、desiredRevision/appliedRevision；普通参数热更新与 affinity 重建分离。
- [ ] 候选 Ready 后帧边界替换；失败保留旧 Worker，隔离/旧结果遵守代际栅栏。
- [ ] API 按 api.md 接线，handler 不读 SDK；稳定机器错误映射和国际化。
- [ ] web 共享类型、归一化及 taskDraft/LiveRulesStudio 保存回归；不新增编辑器或 UI 设计改动。
- [ ] restart/旧客户端/字段省略/null/多算法集合替换/冲突保存回归。

Review gate：DB commit 与 runtime apply 分离可观察；HTTP 保存成功不等于掩码已生效；新旧客户端往返不损失配置。

## E. 板端验收、发布与规范闭环（R11，AC01-AC11）

- [ ] 对 RK3568/RK3576/RK3588 各发布 profile 完成 research.md evidence packet。
- [ ] shadow -> 已验证 auto spread -> manual 分阶段发布；未验证 profile 不启用强绑定。
- [ ] 记录 Runtime 默认与 spread 同输入对照；每组至少 3 轮，预热后固定测量窗口，保留原始结果。
- [ ] 输出吞吐、P50/P95、队列等待、丢帧、CPU、RSS/PSS、CMA/设备内存、fd、温度/频率、session 数量。
- [ ] 性能门槛在运行实验前按产品负载冻结；若 spread 明显回归，不宣称优化成功，保持默认调度并记录范围。
- [ ] 重复启停/配置修改/失败重试压力测试至少 100 轮；长稳至少 8h，覆盖隔离额度耗尽和恢复。
- [ ] 验证无设备、权限不足、设备消失、未知拓扑、内存不足、fallback 失败。
- [ ] 演练关闭 auto spread 和回滚旧宿主，manual 不得被静默取消；DB 不破坏性回退。
- [ ] 更新已实现且验证过的 infer/algo-sdk/db/api/pipeline spec；未实现项仍留在任务文档。

## 故障测试矩阵

| 注入点 | 必须断言 |
| --- | --- |
| reservation 后、线程启动前取消 | reservation 释放且无 session |
| rknn_init 卡死/await 取消 | owner 保活、占额、无新无界重试 |
| 部分多模型初始化失败 | 已创建资源释放，未释放资源隔离 |
| set_core_mask 拒绝 | auto 有界已验证回退；manual 失败；状态不假成功 |
| 回执不存在/错误/旧代际 | 不提交新的 acknowledged |
| 并发共享组首次启动 | 单航班，不瞬时双份模型 |
| 冷却时新成员加入 | 作废旧定时器、不重复分配 |
| Worker 替换与旧结果迟到 | 新旧 reservation 分离、迟到结果不污染新代际 |
| 宿主退出但内部 Worker 未退出 | 库和资源继续保活，额度不提前归还 |
| 隔离额度耗尽 | 拒绝新准入，不清空隔离表伪装恢复 |
| 旧客户端保存/null reset | 保留或显式重置符合契约 |
| DB 成功后运行时失败 | desired/applied 不一致可诊断，旧健康实例保留 |

## 验证命令

先格式化后检查；只对本任务变更执行，不覆盖用户已有格式改动。完整门禁在具备环境时运行并报告所有未跑项。

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo nextest run --workspace

for manifest in algo-packages/rknn/rk3568/Cargo.toml \
  algo-packages/rknn/rk3576/Cargo.toml algo-packages/rknn/rk3588/Cargo.toml; do
  cargo fmt --manifest-path "$manifest" --all
  cargo clippy --manifest-path "$manifest" --workspace --all-targets -- -D warnings
  cargo nextest run --manifest-path "$manifest" --workspace
done
# SDK InitContext/宏影响 macOS 插件，须在相应环境额外验证 macOS workspace。
# 平台 SDK 可用时补 --all-features；硬件测试 #[ignore]，板端单独运行。

cd web
pnpm format
pnpm lint
pnpm typecheck
pnpm test
pnpm check:cycles
pnpm build
```

## 本轮文档验证

只检查 task JSON/JSONL、引用路径、文档状态与需求覆盖；不运行无关 Rust/Web 构建。不得把文档验证写成实现验收通过。保持 task.json.status=planning，不执行 task.py start、归档或 Git 提交。
