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
- 不吞错误；返回明确错误状态，由调用方决定展示。

## 依赖与复用

- Hook 不条件调用，条件写在 effect 内；依赖数组完整，不关闭 `exhaustive-deps`。
- 用稳定回调或 ref 解决引用/闭包问题，不能靠遗漏依赖减少重跑。
- 请求取消、代次检查和分页合并参考 [useOplogs](../../../web/src/features/oplog/hooks/useOplogs.ts)；其现有命名与分页协议以返回类型为准。
- 使用 WS 的 hook 订阅全局连接，不能每个组件各建一条业务事件连接。

验证取消、迟到响应、依赖变化、错误暴露和清理后不再收到回调；测试要求见 [质量规范](./quality-guidelines.md)。
