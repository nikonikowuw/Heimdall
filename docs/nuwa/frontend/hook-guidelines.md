# Hook 规范

## 提取与接口

- 按职责提取复杂或重复的有状态逻辑，不为一次性几行代码建立抽象。
- 使用描述能力的 `useXxx` 名称，一个文件一个主要 hook；单 feature 放本域 `hooks/`，共享基础 hook 放 `src/hooks/`。
- 简单数据获取 hook 统一 `{ data, loading, error, refetch }`，成对值可用元组；分页扩展的接口显式定义。
- 暴露给依赖数组或 memo 组件的函数用 `useCallback`，对象/数组按需用 `useMemo` 稳定。
- hook 返回数据和行为，不返回 JSX；请求复用 [API client](../../../web/src/lib/api.ts)，不散落 URL。

## 副作用与清理

| 建立的资源      | 必须执行的清理                                  |
| --------------- | ----------------------------------------------- |
| 请求            | `AbortController.abort()`，忽略过期响应         |
| 事件/WS 订阅    | unsubscribe/removeEventListener                 |
| 定时器 / RAF    | clearTimeout/clearInterval/cancelAnimationFrame |
| 播放器 / Worker | destroy/terminate                               |

- 清理覆盖卸载、依赖变化和重复挂载，不能只处理正常响应。
- 筛选条件或分页请求变化时防止旧响应覆盖新状态；中止请求不显示为业务故障。
- 不用同时维护代次 ref 与 AbortController 两套机制：控制器身份（`dataAbortControllerRef.current === controller`）本身就能回答「本次请求是否仍是最新」，双机制只会让两侧守卫不一致。
- 不吞错误；返回明确错误状态，由调用方决定展示。

### 长生命周期订阅与高频依赖

WS 订阅 effect 只在「影响订阅本身」的依赖上重建。仅参与回调内判定的高频值（搜索关键字、通道名映射等）不进依赖数组，否则每次按键、每次列表刷新都会拆建全部订阅：

- 用 ref 承载最新快照（参考 [AlarmsPage](../../../web/src/features/alarms/AlarmsPage.tsx) 的 `LiveFilterSnapshot`），由一个小 effect 同步写入；
- 不得为了消除依赖警告而在回调里读过期闭包值，也不能关闭 `exhaustive-deps` 来回避，两者会把「订不到」换成更难查的「判定用旧值」。
- 实时事件的服务端等价匹配必须与列表查询同语义：本地字符串匹配要与 SQL `LIKE` 的 ASCII-only 折叠对齐（见 [API 契约](../backend/api-guidelines.md#证据与告警列表的-q)）。

## 依赖与复用

- Hook 不条件调用，条件写在 effect 内；依赖数组完整，不关闭 `exhaustive-deps`。
- 用稳定回调或 ref 解决引用/闭包问题，不能靠遗漏依赖减少重跑。
- 请求取消与代次检查参考 [useOplogs](../../../web/src/features/oplog/hooks/useOplogs.ts)；其现有命名与分页协议以返回类型为准。日志中心两个查询 hook 均为「按页替换」而非追加合并：筛选下推服务端，前端不对已取回的一页做二次过滤。
- 使用 WS 的 hook 订阅全局连接，不能每个组件各建一条业务事件连接。

验证取消、迟到响应、依赖变化、错误暴露和清理后不再收到回调；测试要求见 [质量规范](./quality-guidelines.md)。
