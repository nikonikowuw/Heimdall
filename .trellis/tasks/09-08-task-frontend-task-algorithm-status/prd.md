# 子任务 PRD：前端任务算法绑定与运行状态回显

## 1. 目标

将算法选择、分析帧率、算法参数和任务实际运行状态接入任务工作台，消除 Task API 与 AlgorithmInstance API 的前端双写不一致。

## 2. 功能要求

- 扩展 TypeScript `TaskConfigDto`、`TaskSummaryDto` 和相关请求类型，字段与后端 API 契约一致。
- 在 `CreateTaskModal`、`LiveRulesStudio` 或现有算法设置侧栏中使用 `algorithmApi.list()` 展示当前平台可用算法，并默认选择后端约定的激活检测算法。
- 支持设置分析 FPS 和 JSON 算法参数；参数输入必须按算法配置 schema 或现有 UI 约束校验。
- 保存任务时只调用 Task API；移除保存成功后再调用 `instanceApi.create/update` 的双写流程。
- 在任务卡片和编辑工作台回显算法名称/版本、分析 FPS、desired 状态、actual status 和 status message。
- 状态显示使用 i18n 和既有主题 token；不得将高频 pump 指标放进 React 高频 state，实时数据应使用既有 WS、ref 或低频刷新策略。

## 3. 验收标准

1. 新建和编辑任务可以选择算法、设置 FPS/参数并成功保存，刷新页面后配置保持一致。
2. 后端返回 Running、Degraded、Error、Stopped 时，任务卡片和工作台能显示对应状态和错误信息。
3. 删除或停用任务后前端状态及时更新，不再调用过时的 AlgorithmInstance 双写接口。
4. 算法列表为空、接口失败和后端回退算法场景有明确的可用状态，不阻塞规则编辑。
5. `pnpm lint`、`pnpm typecheck`、相关 Vitest 测试和 `pnpm build` 通过。

## 4. 依赖

- 依赖 `task-algo-binding-contract` 和 `task-api-pipeline-orchestration` 的稳定 API 字段。
- 与 `09-08-realtime-detection-metadata-canvas-overlay` 保持实时状态和 WebSocket 契约兼容，但不在本子任务实现检测框渲染。
