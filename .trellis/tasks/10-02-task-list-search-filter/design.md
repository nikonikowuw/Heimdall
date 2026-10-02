# AI 任务列表检索与条件筛选 — 技术设计

> 需求与验收见 [prd.md](./prd.md)。本文只描述实现设计，不是启动授权。

## 1. 分层与归属

三层职责，单向依赖（`features/tasks/components` → `features/tasks` → `@/components/ui` → `@/types`，无环）：

```text
features/tasks/components/TaskFilterBar.tsx     展示组件：搜索框 + 药丸 + 算法下拉（受控，无状态）
        │
features/tasks/TasksPage.tsx                    编排：持有筛选状态、派生 options 与计数、过滤任务集
        │
features/tasks/taskFilter.ts                    纯逻辑：归一化 / 匹配 / 计数 / 选项派生（无 React）
        │
@/types                                         仅类型（TaskSummaryDto / Camera / TaskAlgorithmInstanceSummaryDto）
```

**归属理由**：

- `taskFilter.ts` 放 `features/tasks/` 而非 `lib/`。判据是 `directory-structure.md:25` 的「两个以上 feature 实际共用时上提」——目前只有 `tasks` 消费，将来若 `live` 或 `cameras` 也需要任务检索再上提，与 `algoConfigSchema.ts` 的上提时机判断一致。
- `TaskFilterBar.tsx` 独立成组件，对齐 `features/algorithms/components/AlgoFilterBar.tsx` 的既有做法（筛选栏是独立组件，不是页面内联 JSX）。`TasksPage.tsx` 已 323 行，插入约 60 行内联 JSX 会进一步压缩可读性。
- **零 feature 跨域依赖**：算法名映射由 `TasksPage` 通过 `algorithmApi.list()`（`@/lib/api`，叶子层）获取后注入，`taskFilter.ts` 只 `import type from '@/types'`，模式对齐 `features/algorithms/algoUsage.ts` 与 `LiveRulesStudio.tsx:431` 的既有做法。

**注**：`features/algorithms` **没有 `index.ts` barrel**，全仓无任何跨域导入 `@/features/algorithms` 的先例。因此本次**不导入**该 feature 的任何模块（包括 `ALGO_LIST_PAGE_SIZE`），取数直接走 `@/lib/api` 的 `algorithmApi.list()`。

## 2. 纯逻辑契约（`features/tasks/taskFilter.ts`）

```ts
import type { Camera, TaskConfigDto } from '@/types'

/** 查询串归一化：trim + lowercase。供调用方循环外算一次，避免每条任务重复归一化。 */
export function normalizeTaskQuery(query: string): string

/**
 * 文本匹配：命中任务名、摄像头名、摄像头 ID 任一即通过。
 *
 * @param query           原始查询串
 * @param preNormalized   已归一化的查询串；传空串等价于「无查询」
 * @param camera          该任务对应的摄像头；找不到时仅按任务名匹配（数据缺口不误伤）
 */
export function matchesTaskQuery(
  config: TaskConfigDto,
  camera: Camera | undefined,
  query: string,
  preNormalized?: string,
): boolean

/** 布防状态维度取值表。与 `alarms/filters.ts` 的 `defineFilters` 同范式双向锁定。 */
export const ARM_STATUS_FILTERS = defineFilters<'armed' | 'disarmed'>()(['all', 'armed', 'disarmed'])
export type ArmStatusFilter = (typeof ARM_STATUS_FILTERS)[number]

/**
 * 算法维度匹配：任务挂载的实例中**任一** algorithmId 命中即通过。
 * 传入 `'all'` 表示不过滤。实例数组缺失（`undefined`）视为不匹配任何具体算法。
 */
export function matchesTaskAlgorithm(
  config: TaskConfigDto,
  selectedAlgorithmId: string,
): boolean

/** 三维度组合过滤（AND）。空查询 + 全档 + 算法 all ⇒ 返回原数组引用，不新建。 */
export function filterTasks(
  entries: ReadonlyArray<{ camera: Camera; config: TaskConfigDto }>,
  filters: {
    query: string
    armStatus: ArmStatusFilter
    algorithmId: string
  },
): Array<{ camera: Camera; config: TaskConfigDto }>

/** 各档位计数（全量口径，不随筛选变化）。 */
export function countByArmStatus(
  configs: ReadonlyArray<TaskConfigDto>,
): { all: number; armed: number; disarmed: number }

/**
 * 从已加载任务派生 distinct algorithmId，按友好名排序。
 * 选项派生自实际在用集合 ⇒ 每个选项必然有结果，不会出现选中后空列表。
 */
export function deriveAlgorithmOptions(
  configs: ReadonlyArray<TaskConfigDto>,
  nameById?: ReadonlyMap<string, string>,
): Array<{ value: string; label: string }>
```

**关键设计点**：

1. **`defineFilters` 复刻而非导入**。`alarms/filters.ts` 的 `defineFilters` 是 alarms 域私有实现，跨域导入违反 `directory-structure.md:23`。选择**本地复刻**（约 10 行泛型技巧）而非上提到 `lib/`：上提会改动 alarms 既有代码并扩大 diff，而当前只有两个消费点，尚不满足「两个以上 feature 共用」的上提判据。**代价**：未来第三个 feature 需要时应上提，此决定需在 spec 中记录为已知取舍。

2. **`filterTasks` 在「无筛选」时返回原数组引用**。让 `useMemo` 的依赖变化可被下游察觉，避免每次渲染产生新数组引用（对未 memo 的卡片矩阵尤其重要——引用变化会触发全量重渲染）。

3. **`preNormalized` 参数**：对齐 `cameraSearch.ts` 的既有约定，让循环外归一化成为可能。

4. **`camera === undefined` 的降级**：任务存在但摄像头记录缺失（数据不一致）时，仅按任务名匹配，不抛错、不静默排除。

## 3. 组件契约（`features/tasks/components/TaskFilterBar.tsx`）

```tsx
export interface TaskFilterBarProps {
  query: string
  onQueryChange: (value: string) => void
  onQueryClear: () => void

  armStatus: ArmStatusFilter
  onArmStatusChange: (value: ArmStatusFilter) => void
  armStatusCounts: { all: number; armed: number; disarmed: number }

  algorithmId: string
  onAlgorithmChange: (value: string) => void
  /** 已派生的算法选项（含「全部算法」档），由 TasksPage 从任务集派生 */
  algorithmOptions: Array<{ value: string; label: string }>

  /** 是否存在任一非默认筛选条件 */
  hasActiveFilters: boolean
  onClearFilters: () => void
  /** 当前筛选命中数，与 PageHeader 的全量 KPI 并置展示 */
  matchedCount: number
}
```

**布局**：单行毛玻璃容器，结构对齐 `AlarmsPage.tsx:1008` 的筛选工作台与 `AlgoFilterBar`：

```
┌─────────────────────────────────────────────────────────────────────┐
│ [🔍 搜索任务名/摄像头名/通道 ID]   [算法 ▾]   [全部 30│已布防 15│未布防 15] │
│                                                  ↑ 命中 3 / 共 30    │
└─────────────────────────────────────────────────────────────────────┘
```

- 搜索框：`SearchInput`，`showKbdHint`，`containerClassName` 给定弹性宽度。
- 算法下拉：`SelectField<string>`，`icon={Layers}` 提示维度，`emphasis={algorithmId !== 'all'}`，使用 `allOption` 承载「全部算法」。**泛型参数是 `string` 而非字面量联合**，因为选项在运行时派生（对齐 `AlgoFilterBar` 从清单派生 `typeOptions` 的做法）。`SelectField` 的 `narrowSelectValue` 会在选项不匹配时返回 `undefined`，此处选项即真源，无需额外校验。
- 状态药丸：`<div role="group">` 内含 3 个 `<button aria-pressed>`，结构、类名与语义色照搬 `CamerasPage.tsx:498-543`。选中态用 `bg-[var(--accent)] text-white`；「已布防」未选中态用 `hover:text-status-success`，与 KPI 的绿色语义呼应。
- 命中数：`matchedCount !== armStatusCounts.all` 时展示 `筛选命中 N / 共 M`，对齐 `CamerasPage.tsx:632` 的 `pageFiltered` / `total` 并置口径。

**药丸点击语义**：点击当前已选中档位时**保持选中**（不回退到全部），清除路径统一走 `hasActiveFilters` 时展示的「清除筛选」按钮。理由：`CamerasPage` 的药丸亦为单向选中（`:500` 的 `onClick` 直接 `setStatusFilter('all')`，无 toggle 回退），保持一致可避免同类控件行为分裂。

## 4. `TasksPage` 改动

### 4.1 取数

```ts
// 现有（:43）：Promise.all([cameraApi.list(), taskApi.list()])
// 改为三项并行，算法列表失败不阻塞任务列表
Promise.allSettled([cameraApi.list(), taskApi.list(), algorithmApi.list()])
```

- `algorithmApi.list` **不传 `pageSize`**，对齐 `LiveRulesStudio.tsx:431` 的既有调用形态（服务端默认 20、上限 200）。若现场算法数超过首页返回量，下拉标签按 `nameById` 缺失回落原始 ID，功能不降级——**不为此新增分页循环**（边缘设备算法资产为个位数到几十条）。
- **用 `allSettled` 而非 `all`**：算法名仅影响下拉标签，其获取失败不得让整个任务列表报错。任务与摄像头仍按现有逻辑在 `catch` 中优雅降级。
- 算法名映射：`useMemo(() => new Map(items.map(i => [i.algorithmId, i.name])), [items])`；取数失败时为空 Map，不阻塞、不报错。

### 4.2 状态与派生

**排序约束（D6）**：`entries` 必须按 `cameras` 数组的既有顺序构建（对 `camerasWithTasks` 做映射），**不得按 `cameraId` 或任何其他键重排**。

`cameras` 的顺序来自 `CameraRepo::list_all`（`crates/db/src/repository/camera.rs:29-44`）：① `last_probe_status` 优先级 ② `last_success_at` 倒序 ③ `id` 倒序。该顺序与 `CamerasPage` / `LivePage` **共享同一口径**（三页均消费 `cameraApi.list()`），在任务页单独改排序会让三页视觉口径分裂。`filterTasks` 为顺序保持过滤，因此筛选后相对先后与筛选前一致。

底层排序键 ① ② 是探活运行时字段，一次探活失败即导致跨刷新跳动——这是既有行为，本次不修（理由与后续任务候选见 prd.md 的 Out of Scope 与 D6）。

```ts
const [query, setQuery] = useState('')
const [armStatus, setArmStatus] = useState<ArmStatusFilter>('all')
const [algorithmId, setAlgorithmId] = useState('all')

const preNormalized = useMemo(() => normalizeTaskQuery(query), [query])
const armStatusCounts = useMemo(() => countByArmStatus(allConfigs), [allConfigs])
const algorithmOptions = useMemo(() => deriveAlgorithmOptions(allConfigs, algoNameById), [allConfigs, algoNameById])
const visibleEntries = useMemo(() => filterTasks(entries, { query, armStatus, algorithmId }), [...])
```

**KPI 与命中数的口径分离**（R6）：

| 位置 | 数据源 | 筛选时是否变化 |
| --- | --- | --- |
| `PageHeader` `任务总数` / `已布防` | `camerasWithTasks` 全量 | **不变** |
| 筛选栏药丸计数 | `armStatusCounts`（全量） | **不变** |
| 筛选栏 `筛选命中 N` | `visibleEntries.length` | 变化 |
| 卡片网格 | `visibleEntries` | 变化 |

### 4.3 网格与空态

现有三分支（无摄像头 / 无任务 / 网格）扩为四分支，新增「有任务但筛选无命中」：

```tsx
{cameras.length === 0 ? (
  /* 现有：无摄像头 */
) : camerasWithTasks.length === 0 ? (
  /* 现有：尚无任务 */
) : visibleEntries.length === 0 ? (
  /* 新增：筛选无命中 —— 双行结构 + 清除筛选按钮 */
  <div className="flex flex-col items-center justify-center py-24 text-center">
    <Search className="mb-2 h-8 w-8 opacity-40" />
    <p>{t('filter.noMatchTitle')}</p>
    <p className="mt-1 text-xs">{t('filter.noMatchHint')}</p>
    <button onClick={clearFilters}>{t('filter.clearAll')}</button>
  </div>
) : (
  /* 现有：网格 */
)}
```

空态图标用 `lucide-react` 的 `Search`，与 `CamerasPage:587` 一致（该页无匹配态同样用 `Search`）。**严守无 Emoji 约束**。

## 5. `TaskCameraCard` 改动（D3 方案 a）

```tsx
// 现状（:249-254）
<h4>{camera.name || camera.cameraId}</h4>
<p className="font-mono text-[11px]">{camera.cameraId}</p>

// 改为（方案 a）
<h4>{config?.name || camera.name || camera.cameraId}</h4>   {/* 主标题：任务名 */}
<p className="font-mono text-[11px]">{camera.name ?? camera.cameraId} · {camera.cameraId}</p>  {/* 副标题：摄像头身份 */}
```

**依据**：`LiveRulesStudio.tsx:1087` 的默认任务名是 `taskName.trim() || camera.name || \`Task-${camera.cameraId}\``，即未重命名的任务其 `config.name` 已等于摄像头名。因此：

- **未重命名任务**：主标题视觉不变（仍是摄像头名）；副标题新增一行摄像头名重复，需避免——见下方修订。
- **已重命名任务**：主标题显示用户自定义的任务名，与页面语义（一通道一任务、以任务为中心）一致。

**修订（避免副标题重复）**：

```tsx
const displayName = config?.name || camera.name || camera.cameraId
const cameraIdentity = camera.name ?? camera.cameraId
const secondaryLine = displayName === cameraIdentity
  ? camera.cameraId                                   // 任务名 == 摄像头名：副标题只留 ID
  : `${cameraIdentity} · ${camera.cameraId}`          // 任务名独立：副标题给摄像头名 + ID
```

`aria-label`（`:312`）同步更新为使用 `displayName`，保持可访问名称与视觉一致。

## 6. i18n 键（`task.json`，三语同步）

新增 `filter.*` 命名空间，与既有的 `card.*` / `studio.*` 并列：

| 键 | zh-CN | zh-TW | en |
| --- | --- | --- | --- |
| `filter.searchPlaceholder` | 搜索任务名 / 摄像头名 / 通道 ID | 搜尋任務名 / 攝影機名 / 通道 ID | Search task, camera, or channel ID |
| `filter.algorithmLabel` | 按算法筛选 | 依演算法篩選 | Filter by algorithm |
| `filter.algorithmAll` | 全部算法 | 全部演算法 | All algorithms |
| `filter.armStatusLabel` | 按布防状态筛选 | 依布防狀態篩選 | Filter by arm status |
| `filter.armAll` | 全部 | 全部 | All |
| `filter.armed` | 已布防 | 已布防 | Armed |
| `filter.disarmed` | 未布防 | 未布防 | Disarmed |
| `filter.matched` | 筛选命中 {{count}} | 篩選命中 {{count}} | {{count}} matched |
| `filter.noMatchTitle` | 未找到匹配的布防任务 | 未找到符合條件的布防任務 | No matching tasks |
| `filter.noMatchHint` | 尝试清除搜索条件或调整布防状态、算法筛选 | 嘗試清除搜尋條件或調整布防狀態、演算法篩選 | Try clearing the search or adjusting the arm status and algorithm filters |
| `filter.clearAll` | 清除筛选 | 清除篩選 | Clear filters |

计数用 i18next 插值（`{{count}}`），不拼句子；**注意本仓库未安装 ICU 插件**（`directory-structure.md:43`），因此不使用 plural 语法。

## 7. 测试策略

### 纯逻辑（`features/tasks/taskFilter.test.ts`，node 环境）

对齐 `cameraSearch.test.ts` 的组织方式（fixture 常量 + 分维度 `describe`）：

- 归一化：`trim` + 大小写折叠；纯空白 → 空串。
- 文本覆盖：任务名命中 / 摄像头名命中 / 摄像头 ID 命中 / 三者皆不命中。
- `camera === undefined` 时仅按任务名匹配，不抛错。
- 预归一化参数：传 `preNormalized` 与不传结果一致。
- 多实例任一命中：`[general_detection, fire_detections]` 在筛 `fire_detections` 时命中。
- 实例数组为 `undefined` 时不匹配具体算法，但在 `'all'` 下通过。
- 组合 AND：三维度同时施加，验证交集。
- 无筛选时返回原数组引用（`toBe` 断言，防回归）。
- 计数：`all` / `armed` / `disarmed` 之和与总数一致。
- 选项派生：去重、按友好名排序、`nameById` 缺失时回落 ID。

### 组件（`features/tasks/components/TaskFilterBar.test.tsx`，jsdom）

jsdom 环境按 `10-01-numeric-input-blur-validation` 的先例：**按文件首行 `// @vitest-environment jsdom` opt-in**，不改全局 `test.environment`；显式 `afterEach(cleanup)`（仓库未开 vitest `globals`）。

- 搜索框输入触发 `onQueryChange`；清空按钮触发 `onQueryClear`。
- 药丸渲染三个档位及计数；点击回调收到正确档位值；`aria-pressed` 反映选中态。
- 算法下拉渲染传入选项与「全部算法」档；切换回调正确。
- `hasActiveFilters` 为真时展示「清除筛选」。

**不引入** `@testing-library/user-event`（`fireEvent` 足够，与既有约定一致）。

### 卡片（`TaskCameraCard` 相关既有测试）

若既有测试断言了标题文本，需同步更新；`DeleteTaskModal.test.tsx` 不涉及标题渲染，预期无影响。

## 8. 风险与取舍

| 风险 | 判断 | 缓解 |
| --- | --- | --- |
| 每次按键触发全量重渲染（卡片无 `memo`） | **已知且接受**。过滤立即缩小渲染集；本次不改卡片渲染策略（性能话题已搁置） | 若实测输入卡顿，单独评估 `useDeferredValue`（PRD 约束节已记录） |
| `defineFilters` 在 alarms 与 tasks 各有一份 | **接受的重复**。上提会改动 alarms 既有代码；当前不满足「两个以上 feature 共用」判据 | 第三个消费点出现时上提至 `lib/`，并在 spec 记录 |
| 算法名映射取数失败 | 下拉标签回落原始 ID，功能不降级 | `allSettled` 隔离；无 loading 阻塞 |
| 算法数超过 `algorithmApi.list()` 首页返回量 | 超出部分的标签回落原始 ID（选项本身仍从任务集派生，不受影响） | 边缘设备算法为个位数到几十条；不为此新增分页循环 |
| 筛选栏挤压窄视口 | 对齐 `AlgoFilterBar` / `AlarmsPage` 的溢出处理（`overflow-x-auto` + 隐藏滚动条） | 实现时按 `AlarmsPage:1009` 的类名组合 |
| 卡片主标题语义变更影响现有用户认知 | 未重命名任务视觉不变（默认名即摄像头名）；仅重命名任务体现差异 | 方案 a 的 `secondaryLine` 修订避免信息重复 |
| 筛选后卡片跳位（用户感知） | **预期行为**。`filterTasks` 是顺序保持过滤，相对先后不变，仅被滤除项消失 | 不做稳定排序干预（D6）；底层探活排序导致的跳动另行处理 |

## 9. 不涉及的部分

- **无 wire 变更**：不改 `TaskSummaryDto` / `Camera` / 任何 HTTP 字段、信封或错误码。跳过 `api.md` 的理由记录在 `implement.md`。
- **无后端改动**：不碰 `crates/**`。
- **无新增依赖**。
- **无 spec 之外的视觉类族**：复用 `.search-field` / `.select-field` 与既有主题 token，不新增 CSS 类。
