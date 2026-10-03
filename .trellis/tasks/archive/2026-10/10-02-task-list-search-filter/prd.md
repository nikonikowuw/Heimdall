# AI 任务列表检索与条件筛选

## Goal

为 AI 任务页（布防配置控制台）增加**按身份检索 + 按状态收敛 + 按算法筛选**的能力，解决 30+ 路布防任务下只能逐张扫卡片矩阵的定位问题。全部为纯前端本地过滤，不引入服务端查询参数。

## Background

### 现状

- `web/src/features/tasks/TasksPage.tsx:43` 挂载即 `Promise.all([cameraApi.list(), taskApi.list()])` 全量拉取，`:143` 直接 `camerasWithTasks.map(...)` 铺开卡片矩阵。
- 顶部 `PageHeader` 只有 `RefreshButton` + 「新建布防任务」，**无任何检索控件**；页面唯一的搜索框在 `CreateTaskModal.tsx:282`，用于**创建时筛通道**，与任务列表检索无关。
- `crates/api/src/routes/task.rs:238` 的 `list_tasks` 不接 `Query` 提取器；`TaskRepo::list_all_with_instances`（`crates/db/src/repository/task.rs:81`）是无 WHERE 的全量枚举。

### 行业对标（2026-10-02 调研）

| 产品 | 任务/设备管理列表的检索能力 |
| --- | --- |
| ANSVIS（AI 摄像头分析） | `Show Running Tasks Only` 勾选 + 搜索栏「filter tasks by **camera name or task name**」；KPI 头为 `Total Running Tasks` / `Total AI Tasks`，与本页同构 |
| Milestone XProtect | 官方 Camera Search 插件：按 **name / IP / MAC** 搜索 + **filter by enabled/disabled** |
| Frigate | `CameraManagementView` 按 `enabled_in_config` **先分组再排序**，把使能当第一等维度 |
| AIPIX | 侧栏按 `Active / Inactive / Partially active / Blocked` 四档状态过滤 |
| BluSKY / Eagle Eye / Nx Witness | 多维筛选面板 + 点列头排序 |

**共识**：成熟产品的配置类列表普遍提供「身份检索（名称/ID）」+「运行状态筛选」+「关联实体（摄像头/算法）」。本任务按此收敛，**不含排序与时间范围**（理由见 Out of Scope）。

### 可直接复用的既有约定

| 能力 | 既有实现 | 复用方式 |
| --- | --- | --- |
| 文本输入控件 | `components/ui/SearchInput.tsx` | 直接用，带 `showKbdHint` |
| 状态药丸（含计数） | `CamerasPage.tsx:498-543` | 照搬结构：`aria-pressed` + 计数 + 语义色 |
| 筛选纯逻辑 + 测试形状 | `features/cameras/cameraSearch.ts` + `.test.ts` | 照搬「归一化 + 匹配（支持预归一化避免循环内重复 lowercase）」 |
| 枚举维度声明 | `features/alarms/filters.ts` 的 `defineFilters` | 状态维度按同一范式声明取值表 |
| 算法名回落策略 | `LiveRulesStudio.tsx:1188` `find(...)?.name ?? item.algorithmId` | 同一降级策略：映射缺失时回落原始 ID |
| 零 feature 依赖的跨域聚合 | `features/algorithms/algoUsage.ts`（只 `import type from '@/types'`） | 筛选纯逻辑只消费 `@/types`，不 import 任何 feature |
| 筛选命中数并置 | `CamerasPage.tsx:632` `pageFiltered` / `total` | KPI 与命中数互不覆盖 |

### 约束

- **纯前端本地过滤，不做服务端 `q`**。依据 `db/backend/database-guidelines.md:47`：通道名匹配需 `LEFT JOIN cameras`，而 `cameras.name` 非唯一列且无索引，该 join 只应在客户端传 `q` 时拼入，不得无条件预置。实测 30 任务规模下列表查询 ~1ms、响应体 38.8 KiB，服务端过滤无收益且会引入真正的全表扫描。
- **模块边界**（`directory-structure.md:23-26`）：禁止深层导入其他 feature 私有模块。筛选逻辑只消费 `TaskSummaryDto` / `Camera` 的本地字段；算法名映射作为**参数注入**（`ReadonlyMap`），拿不到时回落原始 ID。
- **KPI 语义不变**：`PageHeader` 的「任务总数 / 已布防」是舰队级 KPI，筛选后必须保持全量口径；筛选命中数在筛选栏内另行展示。
- **`/` 快捷键无冲突**：`use-global-shortcuts.ts:127` 用 `document.querySelector` 取**第一个** `input[data-search-input="true"]`；筛选栏位于 `PageHeader` 之后、网格之前，DOM 顺序占先。`CreateTaskModal.tsx:162` 为 `if (!isOpen) return null`，关闭时不渲染，且模态框打开时 `isAnyModalOpen()` 已整体抑制快捷键。
- **输入触发时机**：过滤随每次按键执行（与 `CamerasPage` 一致，不引入防抖）。渲染集随过滤立即缩小；本次不引入 `useDeferredValue`，若实测出现输入卡顿再单独评估。

## Requirements

- **R1 文本检索**：单个搜索框，大小写不敏感的子串匹配，覆盖**任务名**（`TaskConfigDto.name`）、**摄像头名**（`Camera.name`）、**摄像头 ID**（`Camera.cameraId`）。查询串先 `trim().toLowerCase()` 归一化一次，再传入匹配函数（避免循环内重复归一化）。
- **R2 布防状态筛选**：药丸控件，档位 `全部 / 已布防 / 未布防`，判定字段为 `desiredEnabled`（与 `PageHeader` 的 `armedCount` 同源）。每档展示计数，格式对齐 `CamerasPage` 的 `标签 (N)`。
- **R3 算法维度筛选**：按任务挂载的算法筛选，**独立下拉**（`SelectField`），选项从已加载任务的 `algorithmInstances` 派生 distinct `algorithmId`。**多实例语义 = 任一命中**（`instances.some(...)`）：任务挂了 `[通用检测, 火焰检测]` 时，筛「火焰检测」必须命中该任务，否则用户会得到静默的错误答案。下拉标签显示算法**友好名**（`AlgorithmItem.name`），经 `algorithmApi.list()` 取映射，缺失时回落原始 `algorithmId`。
- **R4 组合语义**：三个维度为 **AND** 关系；每个维度内部为 OR（文本命中任一字段即算命中）。
- **R5 无匹配空态**：全部维度均无命中时，展示独立的「无匹配」空态，区分于「尚无任务」空态，并提供清除筛选的入口。文案结构对齐 `CamerasPage.tsx:585-596` 的双行形态（标题 + 提示）。
- **R6 计数口径**：筛选栏内展示的计数一律为**全量口径**（各档位在未应用其他筛选时的总数），不随当前筛选变化；`PageHeader` 的 KPI 完全不受筛选影响。
- **R7 筛选栏门控**：仅在 `cameras.length > 0` 时渲染筛选栏（对齐 `CamerasPage.tsx:396` 的 `cameras.length > 0 &&` 门控）。
- **R8 i18n**：所有可见文本接入 `t()`，键按领域点分隔；**新增键同步 `zh-CN` / `zh-TW` / `en` 三语**（当前 `task.json` 各 70 键且三者对齐）。
- **R9 卡片显示任务名**：`TaskCameraCard` 当前只显示 `camera.name`（`:250`）与 `camera.cameraId`（`:253`），**任务名 `config.name` 完全不显示**。按任务名检索命中时，卡片上看不到匹配原因。建议一并补显示（方案见 Key Decisions D3），使检索结果自解释。
- **R10 测试**：筛选纯逻辑补单测，覆盖归一化、字段覆盖、多实例任一命中、组合 AND、空查询（应全部通过）。

## Acceptance Criteria

- [x] 输入任务名、摄像头名、摄像头 ID 的任一片段均可命中对应任务；大小写不敏感；查询串首尾空格被忽略。
  （`taskFilter.test.ts` 覆盖归一化与三字段命中）
- [x] 点击「已布防 / 未布防」药丸后，卡片矩阵只显示对应 `desiredEnabled` 的任务；每档显示计数；重复点击同一档回到「全部」或保持选中（实现择一，行为需在组件测试中固定）。
  （实现选「保持选中」；`TaskFilterBar.test.tsx` 的 `keeps the current bucket selected when it is clicked again` 固定该行为）
- [x] 多算法任务（`algorithmInstances` 长度 > 1）在按其非主实例算法筛选时**仍被命中**，有单测覆盖。
  （`matches a task through any of its instances, not just the primary one`）
- [x] 算法下拉的每个选项都对应至少一个任务（选项派生自实际在用集合）；标签显示友好名，`algorithmApi.list()` 失败/缺失时回落原始 `algorithmId` 且不阻塞任务列表渲染。
  （`deriveAlgorithmOptions` 单测 + `TasksPage` 改 `Promise.allSettled` 使算法取数失败不阻塞）
- [x] 文本 + 状态 + 算法三维度同时生效时为 AND 语义，有单测覆盖。
  （`combines all three dimensions with AND semantics`）
- [x] 无匹配时展示独立空态且可一键清除筛选；`cameras.length === 0` 时不渲染筛选栏。
  （`TasksPage.tsx` 第四分支 + `clearFilters`；门控 `cameras.length > 0 &&`）
- [x] `PageHeader` 的「任务总数 / 已布防」在任意筛选状态下数值不变。
  （KPI 用全量 `entries.length` / `totalArmed`，与 `visibleEntries` 分离）
- [x] `/` 快捷键在任务页聚焦任务筛选框；`CreateTaskModal` 打开时不抢焦点。
  （收尾时补 4 条单测钉住该 AC：聚焦成功/无搜索框不消费/模态打开不抢焦点/`Shift+/` 仍归帮助面板；
  变异测试确认「模态守护」用例可捕获回归。此前仅有手工核对。）
- [x] 三语 i18n 键齐全且对齐（`zh-CN` / `zh-TW` / `en`）。
  （三语各 283 键，新增 `filter.*` 各 11 键；`catalog.test.ts` 强制键对齐与字面量可解析）
- [x] `cd web && pnpm format && pnpm lint && pnpm typecheck && pnpm test && pnpm check:cycles && pnpm build` 全绿。
  （实测：lint 零告警；**621 tests / 78 files**；模块图 277 模块 788 依赖无环；build 成功）
- [x] spec 更新：`.trellis/spec/web/frontend/` 记录任务列表检索契约（维度、AND/OR 语义、计数口径）。
  （`component-guidelines.md` 新增「列表检索与筛选」章节，含 4 条实现中收敛出的契约）

## Out of Scope

- **服务端 `q` 与 `LEFT JOIN cameras`**：见约束节，30 任务规模无收益且引入全表扫描。
- **排序能力与排序稳定性**：本次沿用 `cameraApi.list()` 的既有顺序，不引入排序控件，也不修改后端排序。

  **现状与其代价**：`CameraRepo::list_all`（`crates/db/src/repository/camera.rs:29-44`）按 ① `last_probe_status` 优先级 ② `last_success_at` 倒序 ③ `id` 倒序 排序。其中 **① ② 均为运行时易变字段**（由 `CameraProbeService::spawn_probe_and_broadcast` 写入，`camera.rs:147,230`），因此摄像头探活结果一变，卡片分组与位置随之跳动。`TaskRepo::list_all_with_instances` 的 `order_by_asc(Id)`（`task.rs:85`）在合并时失效——前端将任务摊平为 `Record<cameraId, TaskConfigDto>`，渲染顺序完全由 `cameras` 数组驱动（`TasksPage.tsx:278`）。

  **行业证据（2026-10-02 调研）**：这是公认反模式。`inspector-go` #304「stabilize the live dashboard (cards jump)」根因即「re-sorted by recent traffic every poll」；`OpenLogi` #37 将排序键从 HID 枚举顺序改为稳定硬件标识符；`lobehub` #17296 修复 `lastSeenAt DESC` 导致的设备列表跳动；Frigate 用显式 `ui.order` 字段（`frigate/config/camera/ui.py`）且 Birdseye 默认按名称排序以「ensure a constant view」；UX 规范（`uxpatternsguide.com` Sort controls）指出排序必须用户可控且明示属性，而非由系统按易变字段决定性。

  **为何不在本次修复**：修改排序需触及 `crates/db` + `crates/api`，且 `cameraApi.list()` 被 **三个页面共享消费**（`TasksPage.tsx:43`、`CamerasPage.tsx:61`、`LivePage.tsx:356`），影响面超出本任务边界且需重跑后端门禁。**决定（用户 2026-10-02 确认方案 A）：本次维持现状，列表稳定性另开任务处理。**

  **已记录的后续任务候选**（不在本任务交付）：*摄像头/任务列表稳定性排序* — 方向选项：（a）改用稳定键（`id` / `created_at` / `cameraId`）；（b）新增 `cameras.display_order` 字段 + 拖拽排序（Frigate 模式）；（c）保留动态排序但加过渡动画降低跳变刺眼度。影响面：`crates/db` + `crates/api` + cameras / tasks / live 三个前端页面。
- **时间范围筛选**：配置类列表罕见（事件/告警类才有），且无「找不到」问题。
- **实际运行状态筛选**（`actualStatus` 为 Error/Degraded）：与 `desiredEnabled` 是两个独立开关（`api/backend/api-guidelines.md:39`），语义易混；先做使能，观察真实需求。是否加第四档「异常」见 Open Questions。
- **分页 / 虚拟滚动 / 卡片 `React.memo` / `backdrop-blur` 降级**：性能话题，已被明确搁置。注：`web/frontend/quality-guidelines.md:29` 的虚拟滚动门槛是 **200 条**，30 条不适用。
- **创建任务模态框内的通道搜索**（`CreateTaskModal.tsx:282`）：已存在，本次不改。
- **ANSSVIS 的 `DUPLICATE`（复制任务配置）能力**：调研中发现的对标差异，另开任务。

## Key Decisions

- **D1 纯本地过滤而非服务端查询（已确认）**：依据约束节的 DB 规范与实测数据。
- **D2 药丸而非下拉承载布防状态（已确认）**：对齐 `CamerasPage` 既有形态；带计数可一眼看清分布（`全部 30 / 已布防 15 / 未布防 15`），下拉需展开才知道；且 Frigate 等成熟产品把使能当第一等分组维度。
- **D3 卡片补显示任务名（建议，待评审）**：三种方案——(a) 卡片标题改为任务名、摄像头名降为副标题；(b) 在 `cameraId` 副标题行并置任务名；(c) 不改，接受「搜到但看不到匹配原因」。**建议 (a)**：`LiveRulesStudio.tsx:1087` 的默认任务名就是 `camera.name || camera.cameraId`，因此对未重命名的任务视觉无变化，仅在用户重命名后体现差异，且与页面语义（任务为中心、一通道一任务）一致。
- **D4 算法名映射作为参数注入（已确认）**：不 import `features/algorithms`，由调用方传入 `ReadonlyMap<string, string>`，缺失时回落 `algorithmId`，复用 `LiveRulesStudio` 既有的 `?.name ?? id` 降级策略。
- **D5 算法维度形态 = 独立下拉 + 友好名（B1，用户 2026-10-02 确认）**：`SelectField` 是枚举选择的既定组件（`styling-guidelines.md` 的控件职责表：关键字检索→`SearchInput`、枚举选择→`SelectField`）；算法集合是有限枚举，且用户的真实场景（「哪些任务在用火焰检测」）是精确问题而非探索性搜索。选项从已加载任务派生 distinct `algorithmId`，因此**每个选项必然有结果**，不会出现选中后空列表。标签取友好名以对齐用户配置算法的场所（`AlgorithmRack` / `CreateTaskModal` 均显示 `name`），而非卡片上仅作技术标识的原始 ID。
- **D6 筛选后卡片顺序与排序稳定性（已确认，用户 2026-10-02）**：
  - **筛选不改变排序规则**。`filterTasks` 是对 `entries` 的顺序保持过滤（`Array.prototype.filter` 语义），因此筛选后卡片的相对先后与筛选前一致，仅被滤除的项消失。用户已从「我筛掉了 N 个」的视角理解此行为。
  - **不做稳定排序干预**。本次不在前端重排 `camerasWithTasks`（那会让任务页与 cameras / live 两页的排序口径不一致），也不修改 `CameraRepo::list_all`（影响三个页面，超出边界）。
  - **已知且接受的残留行为**：底层排序键 `last_probe_status` / `last_success_at` 是探活运行时字段，因此一次探活失败就会使该卡片换组跳动。这是既有行为，非本次引入；已从本 PRD 的 Out of Scope 节升级为有证据、有后续任务候选的显式决定。
  - **实现约束**：`visibleEntries` 必须基于「按 `cameras` 数组顺序构建的 `entries`」过滤，**不得先按 `cameraId` 排序**（那会与 `CamerasPage` 的视觉口径分裂）。

## Open Questions

- **Q2 布防状态是否增加第四档「异常」**：`actualStatus === 5`（Error）是真实运维关注点（「哪些任务挂了」），且因撤防会把 `actualStatus` 写回 0，异常态是「已布防」的子集，可作为同维度第四档。**当前默认三档**，本任务按三档实现。
