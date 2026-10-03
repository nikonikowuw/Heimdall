# 实施与验证计划

> planning / 待确认。本计划不是启动授权。需求见 [prd.md](./prd.md)，技术设计见 [design.md](./design.md)。

## 0. 开工门禁与变更边界

- [x] 用户确认最终规划摘要（含 design 第 5 节的卡片标题方案 a 及其 `secondaryLine` 修订）。
- [x] 该摘要之后的明确实施批准，再执行 `task.py start`；当前保持 planning。
- [x] 保留工作区已有修改；本次任务是新增目录 `.trellis/tasks/10-02-task-list-search-filter/`，不触碰其他任务与无关改动。
- [x] **跳过 `api.md` 的理由**：不新增/修改任何 HTTP 字段、DTO、信封或错误码；筛选为纯前端本地过滤，后端契约零变更（design 第 9 节）。
- [x] 按 web 包 spec 读取规范：`web/frontend/index.md` 的 Pre-Development Checklist（目录与 i18n、组件、Hook、类型、样式、错误处理）；上下文清单见 `implement.jsonl` / `check.jsonl`。

### 变更边界（不做什么）

- 不碰 `crates/**`（无后端改动）。
- 不改 `TaskSummaryDto` / `Camera` / 任何 `@/types` 定义。
- 不做服务端 `q` / `LEFT JOIN cameras`。
- 不做分页、虚拟滚动、卡片 `React.memo`、`backdrop-blur` 降级（性能话题已搁置；`quality-guidelines.md:29` 的虚拟滚动门槛为 200 条，30 条不适用）。
- 不引入 `useDeferredValue` / `useDebounce`（PRD 约束节已记录触发条件）。
- 不新增第二个 `defineFilters` 上提（当前不满足「两个以上 feature 共用」判据，取舍见 design 第 8 节）。
- 不改 `CreateTaskModal` 内既有的通道搜索。
- 不新增 CSS 类族、不硬编码颜色（用主题 token / 语义 utility）。
- 不引入新依赖。

## 1. 交付拆分与风险隔离

| 单元 | 顺序 | 可独立验证 / 回滚点 |
| --- | --- | --- |
| U1 纯逻辑 `taskFilter.ts` + 单测 | 首先 | `pnpm test` 新测试全绿；不影响任何现有页面 |
| U2 `TaskFilterBar` 组件 + jsdom 测试 | 依赖 U1 | 组件测试全绿；未被 `TasksPage` 引用前零影响 |
| U3 `TasksPage` 取数改造（`allSettled` + 算法映射） | 依赖 U1 | 任务列表既有行为不回归；算法取数失败不阻塞 |
| U4 `TasksPage` 筛选编排（状态、派生、四分支） | 依赖 U1/U2/U3 | 手工核对 + 组件测试 |
| U5 `TaskCameraCard` 标题改造（D3 方案 a） | 独立 | 卡片视觉核对；无既有测试断言标题 |
| U6 i18n 三语键 | 依赖 U2/U4 | 三语键数对齐（当前各 70 键） |
| U7 spec 更新 | 最后 | 文档自检 |
| U8 全量门禁 + 手工核对 | 最后 | 见第 3 节命令 |

U1/U2/U5 相互独立；U3/U4 依赖 U1/U2。每单元自带测试与回滚点。若需缩小范围，U5 与 U7 可最后并入或单独评估。

## 2. 实施清单

### U1 纯逻辑层

- [x] 新建 `web/src/features/tasks/taskFilter.ts`：按 design 第 2 节实现 `normalizeTaskQuery`、`matchesTaskQuery`、`ARM_STATUS_FILTERS` + `defineFilters` 本地复刻、`matchesTaskAlgorithm`、`filterTasks`、`countByArmStatus`、`deriveAlgorithmOptions`。
- [x] `defineFilters` 复刻自 `features/alarms/filters.ts`，加注释说明「本地复刻而非跨域导入」的理由与将来上提条件（design 第 8 节）。
- [x] `filterTasks` 在无筛选时 `return entries` 原引用（用 `toBe` 断言防回归）。
- [x] 仅 `import type { Camera, TaskConfigDto } from '@/types'`，不引入任何 feature。
- [x] 新建 `web/src/features/tasks/taskFilter.test.ts`：按 design 第 7 节覆盖归一化、三字段命中、`camera === undefined`、预归一化、多实例任一命中、实例缺失、组合 AND、原引用、计数、选项派生。

### U2 组件层

- [x] 新建 `web/src/features/tasks/components/TaskFilterBar.tsx`：按 design 第 3 节实现受控组件；搜索框用 `SearchInput`（`showKbdHint`），算法用 `SelectField<string>`（`icon={Layers}`、`emphasis`、`allOption`），状态药丸照搬 `CamerasPage.tsx:498-543` 的 `aria-pressed` + 计数 + 语义色结构。
- [x] 溢出处理对齐 `AlarmsPage.tsx:1009` 的 `overflow-x-auto` + 隐藏滚动条类名组合。
- [x] 新建 `web/src/features/tasks/components/TaskFilterBar.test.tsx`：首行 `// @vitest-environment jsdom`（对齐 `NetworkSettings.test.tsx` 等 5 处既有 opt-in）；显式 `afterEach(cleanup)`。
- [x] 组件内所有可见文本走 `t()`（namespace `task`），**不写 defaultValue 兜底文案**（键在三语中必须真实存在，U6 补齐）。

### U3 取数改造

- [x] `TasksPage.tsx:43` 的 `Promise.all` 改为 `Promise.allSettled([cameraApi.list(), taskApi.list(), algorithmApi.list()])`。
- [x] **不导入 `@/features/algorithms/algoFilters`**：该 feature 无 `index.ts` barrel，且全仓无任何跨域导入 `@/features/algorithms` 的先例（`directory-structure.md:23` 禁止深层导入）。`LiveRulesStudio.tsx:431` 已确立本域取算法的正确路径——直接调 `algorithmApi.list()`（来自 `@/lib/api`，叶子层）。
- [x] `algorithmApi.list()` **不传 `pageSize`**，对齐 `LiveRulesStudio.tsx:431` 的调用形态（服务端默认 20、上限 200）。若现场算法数超过首页返回量，下拉标签按 `nameById` 缺失回落原始 ID，功能不降级——**不为此新增分页循环**（边缘设备算法资产为个位数到几十条）。
- [x] 算法名映射：`useMemo(() => new Map(items.map(i => [i.algorithmId, i.name])), [items])`；取数失败时为空 Map，不阻塞、不报错。
- [x] 保持既有的 `.then/.catch/.finally` 链写法（`async` + `try/finally` 会被 `set-state-in-effect` 判为同步 setState）。
- [x] 摄像头/任务失败时的既有优雅降级行为不变（仅算法失败是新增的独立分支）。

### U4 筛选编排

- [x] 新增三个 `useState`（`query` / `armStatus` / `algorithmId`）与三个 `useMemo`（`armStatusCounts` / `algorithmOptions` / `visibleEntries`）。
  > **实施偏离（已核实为等价简化）**：计划中的第四个 `preNormalized` useMemo 未在页面层落地——
  > `filterTasks` 内部已先 `normalizeTaskQuery` 再进 `.filter()`，归一化同样是「循环外只做一次」，
  > 无逐条重复开销。`matchesTaskQuery` 的 `preNormalized` 形参保留并在 U1 单测覆盖（§design 2.3）。
  > 页面层少一个 useMemo，语义不变。
- [x] **`entries` 按 `cameras` 数组顺序构建，不重排**（D6）；`filterTasks` 顺序保持，筛选后相对先后不变。
- [x] `PageHeader` 的 KPI 改用全量口径（当前已是 `camerasWithTasks.length` / `totalArmed`，确认不随筛选变化）。
- [x] 原三分支扩为四分支，新增「筛选无命中」态（`Search` 图标 + 双行文案 + 清除筛选按钮），**不使用 Emoji**。
- [x] 卡片网格改用 `visibleEntries`；`clearFilters` 一次性复位三个状态。
- [x] 筛选栏门控 `cameras.length > 0`（对齐 `CamerasPage.tsx:396`）。

### U5 卡片标题改造

- [x] `TaskCameraCard.tsx:249-254` 按 design 第 5 节改为 `displayName` + `secondaryLine` 条件副标题。
- [x] `aria-label`（`:312`）同步使用 `displayName`。
- [x] 手工核对三种情形：未重命名任务（视觉不变）、已重命名任务（显示任务名 + 摄像头身份）、`config` 缺失（回落摄像头名）。

### U6 i18n

- [x] 按 design 第 6 节的 11 个 `filter.*` 键，同步写入 `web/src/i18n/{zh-CN,zh-TW,en}/task.json`。
- [x] 计数用 i18next 插值 `{{count}}`；**不使用 plural / ICU 语法**（未安装 ICU 插件）。
- [x] 校验三语键数对齐且无遗漏（当前各 70 键，新增后应各 81 键）。

### U7 spec 更新

- [x] 在 `.trellis/spec/web/frontend/` 记录任务列表检索契约：三个维度、维度内 OR / 维度间 AND、计数口径（KPI 全量 vs 命中数）、`defineFilters` 的本地复刻取舍与上提条件、算法名映射的注入式降级策略。
- [x] 按 `component-guidelines.md` 的组织方式落笔，引用 `taskFilter.ts` 与 `TaskFilterBar.tsx` 作为参考实现。

### U8 门禁

- [x] 见第 3 节完整命令；任一失败必须修复或在交付说明中明确列出。

## 3. 验证命令

```bash
cd web
pnpm format
pnpm lint          # eslint --max-warnings=0
pnpm typecheck     # tsc -b
pnpm test          # vitest run
pnpm check:cycles  # 模块图不得有循环依赖（含 import type）
pnpm build
```

定向验证（实施过程中逐步执行）：

```bash
cd web
pnpm test src/features/tasks/taskFilter.test.ts
pnpm test src/features/tasks/components/TaskFilterBar.test.tsx
pnpm test src/features/tasks      # tasks 域全量，确认无回归
```

手工核对清单：

- [x] 30 路任务下输入片段命中正确任务；清空后恢复全量。
  → 由 `taskFilter.test.ts` 自动化覆盖（无需手工）。
- [x] 药丸三档切换与计数正确；`PageHeader` KPI 不变。
  → 由 `TaskFilterBar.test.tsx` + KPI 全量口径代码核对覆盖。
- [x] 多算法任务在筛其非主实例算法时仍被命中。
  → 由 `taskFilter.test.ts` 自动化覆盖。
- [x] 三维度 AND 组合正确；无匹配时空态可一键清除。
  → 由 `taskFilter.test.ts` + `TasksPage` 第四分支代码核对覆盖。
- [x] `/` 快捷键聚焦任务筛选框；打开创建模态框后 `/` 不抢其焦点。
  → **收尾时补齐自动化**：`use-global-shortcuts.test.ts` 新增 4 条用例（原先该路径零覆盖），
  > 并用变异测试确认「模态打开不抢焦点」可捕获回归。
- [ ] 双主题（light / dark）下筛选栏视觉正确。
  **未验证**：需真实浏览器，本次收尾未执行。
- [x] 三语切换文案无缺失（无 `filter.` 原始键名泄漏）。
  → 由 `catalog.test.ts` 强制三语键对齐 + `t()` 字面量可解析（三语各 283 键）覆盖。
- [ ] 窄视口下筛选栏横向滚动不破版。
  **未验证**：需真实浏览器，本次收尾未执行。

## 4. 回滚点

每个单元都是独立 commit 候选：

| 单元 | 回滚影响 |
| --- | --- |
| U1 / U2 | 纯新增文件，删除即回滚，零影响 |
| U3 | 取数改造，回滚恢复 `Promise.all` 两项 |
| U4 | 筛选编排 + 空态，回滚恢复三分支与原网格 |
| U5 | 卡片标题，回滚恢复 `camera.name || camera.cameraId` |
| U6 | i18n 键，回滚删除 11 个键 |
| U7 | 文档，独立回滚 |

## 5. 已确认项（用户 2026-10-02 批准「采用你推荐的方案」，无需再议）

1. **D3 卡片标题方案 a**：主标题改为任务名（未重命名任务视觉不变），副标题按 `secondaryLine` 规则给摄像头身份。**确认采用。**
2. **Q2 布防状态三档**：`全部 / 已布防 / 未布防`，**不加第四档「异常」**。
3. **药丸点击语义**：点击已选中档位保持选中（不回退「全部」），清除走「清除筛选」按钮——与 `CamerasPage` 一致。**确认采用。**
4. 算法维度形态：独立下拉 + 友好名（B1）。
5. 卡片顺序：维持 `cameraApi.list()` 既有顺序，筛选不改变排序规则（D6 / 方案 A）；底层探活排序导致的跨刷新跳动另开任务处理。
