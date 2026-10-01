# 数值输入失焦校验契约设计

> 修订草案 / 待用户确认。D1（共享载体）、D2（jsdom + Testing Library）已确认；本文接口为拟议，不是现有 API。需求与验收见 prd.md。

## 1. 分层与归属

三层职责，单向依赖（`features/*` → `components/ui` → `lib`，无环）：

```text
features/cameras、features/system、features/tasks   站点适配：传 bounds / 空值语义 / 保存兜底
        │
components/ui/NumericField.tsx                     行为组件：草稿状态机 + 三段式时序，无业务边界
        │
lib/numericDraft.ts                                纯逻辑：解析 / 收敛 / 格式化，无 React
```

### 迁移映射

| 站点 | 现状 | 目标 |
| --- | --- | --- |
| `features/tasks/numericParam.ts` | 混杂 schema 解析与草稿逻辑（`parseNumericDraft` / `clampNumericParam` / `formatNumericDraft` / `isFiniteNumber` / `getNumericParamConfig`） | 通用逻辑迁至 `lib/numericDraft.ts`；`getNumericParamConfig`（JSON Schema → config）留在原文件，import 通用逻辑。仅 `AlgoParamDrawer` 消费，不新增 re-export 层 |
| `features/tasks/components/AlgoParamDrawer.tsx` | 自有字符串草稿 + `onBlur → commitNumericDraft`（cosine 双域 UI） | **保留自有 UI 与双域换算**，仅换 import 来源；不在本次迁入共享组件（`Out of Scope` 已声明） |
| `features/cameras/components/RecordingConfigCard.tsx` `NumberField` | `value: number` 受控 + `parseInt` guard | 删除本地 `NumberField`，改用 `NumericField`（必填、integer、区间 `RECORDING_LIMITS`）；保存仍走既有 `clampRecordingConfig` |
| `features/system/Gb28181Settings.tsx` ×4 输入 | `value: number` + `Number(e.target.value)` | 改用 `NumericField`（必填、integer）；`handleSave` 增加最终 clamp |
| `features/system/NetworkSettings.tsx` 前缀 / Metric | `Number(v) \|\| null`（`0` 折叠）+ 无收敛 | 前缀改 `NumericField`（可空、integer、1–32）；Metric 改 `NumericField`（可空、integer、无区间）；`executeSave` 增加前缀 clamp |
| `features/system/StorageSettings.tsx` `NumericInput` / `NumberInput` | 本地字符串 + 失焦不 clamp；`NumberInput` 包装层把区间写死为 0–100 | 删除两个本地组件，改用 `NumericField`，按字段传真实区间；新增 `features/system/storageDraft.ts` 的 `clampStorageDraft` 作为保存兜底（对齐 `recordingConfig.ts` 先例） |

## 2. 组件契约（`NumericField`）

```tsx
export type NumericFieldType = 'number' | 'integer'

interface NumericFieldCommonProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>,
    'value' | 'onChange' | 'onBlur' | 'onKeyDown' | 'type' | 'min' | 'max' | 'inputMode' | 'step'> {
  /** 可访问名称，必填（漏写从运行期缺陷变成编译期错误，沿用 SearchInput / SelectField 的做法） */
  label: string
  /** 数值语义：integer 时草稿只接受整数字符串，失焦格式化取整 */
  type?: NumericFieldType
  min?: number
  max?: number
}

/** 必填模式：空/非法失焦回退上次合法值，回调只收 number */
export interface RequiredNumericFieldProps extends NumericFieldCommonProps {
  emptyBehavior?: 'revert'
  value: number
  onChange: (value: number) => void
}

/** 可空模式：空串失焦回调 null，界面保持空白 */
export interface NullableNumericFieldProps extends NumericFieldCommonProps {
  emptyBehavior: 'null'
  value: number | null
  onChange: (value: number | null) => void
}

export type NumericFieldProps = RequiredNumericFieldProps | NullableNumericFieldProps
```

要点：

- **判别联合**让「可空字段忘记处理 null」与「必填字段被迫处理 null」都成为类型错误；不引入 `value: number | null` 的弱签名。
- **不内置 label 元素**：各站点标签形态差异大（卡片内联 label + 单位、表格列头、独立 label 块），组件只接 `label` 落到 `aria-label`，视觉/布局由调用点 `className` 决定。不新增 `.numeric-field` 类族——本次修的是行为契约，统一视觉是另一个议题。
- **`type="text"` + `inputMode="numeric | decimal"` + `autoComplete="off"`**：`type="number"` 各浏览器对 `-`、`1.`、空串的 `value` 归一化不一致，恰好是本契约要保留的键盘中间态；原生 min/max 也不阻挡键入、只影响 `:invalid`，而站点均不在 `<form>` 提交路径上，实际从不触发浏览器校验。切换后失去原生步进箭头（与全局 `appearance` 重置的方向一致）。不设 `role="spinbutton"`：不实现方向键步进却声明 spinbutton 角色会向 AT 谎报能力。
- 组件内部状态：`draft: string`、`isEditing: boolean`、`syncedValue`。外部值同步沿用仓库既有模式（渲染期状态调整而非 effect，见 `StorageSettings` 现状注释）：**非编辑态**外部值变化时同步草稿；编辑中刻意不同步，避免打断输入。

### 状态机（三段式时序）

| 阶段 | 行为 |
| --- | --- |
| `onChange`（编辑期） | 只 `setDraft(e.target.value)`，**不派发 `onChange`**。空串、`-`、`.`、不完整小数全部可停留 |
| `onBlur` / `Enter`（收敛） | `parseNumericDraft(draft, type)`；见下方语义表；可解析则 `clampNumericParam` → 派发 → 草稿原地格式化为提交值显示（不依赖父层回传，避免一帧回跳） |
| 保存/应用（兜底） | 站点各自的 `clamp*` 纯函数对 payload 再收敛一次，不假设 UI 已收敛（R3） |

编辑期不派发意味着 `AlgoParamDrawer` 式的「滑块 ↔ 数字实时联动」不会发生在共享组件上——这是有意的：R4 要求中间态不污染模型，实时预览若需要应由站点组合自有控件实现。

### 失焦语义表（对 R2 的精确化）

| 草稿 | `emptyBehavior: 'revert'`（必填） | `emptyBehavior: 'null'`（可空） |
| --- | --- | --- |
| 空串 / 纯空白 | 回退上次合法值，不派发，显示恢复 | 派发 `null`，显示保持空白 |
| 不可解析非空（`abc`、`--`） | 回退，不派发 | **回退**（笔误 ≠ 清空意图，不降级为 `null`） |
| 可解析、区间内 | 派发原值 | 派发原值 |
| 可解析、超界 | clamp 到边界后派发，显示边界值 | 同左 |
| 可解析、超界为 0（如 prefix=0） | clamp 到 min 派发（修复 `0 → null` 折叠） | 同左 |

## 3. 站点参数表（design 定稿）

| 站点字段 | type | min | max | emptyBehavior | 依据 |
| --- | --- | --- | --- | --- | --- |
| 录像 事件前/后录制秒数 | integer | 5 | 30 | revert | `RECORDING_LIMITS` |
| 录像 单文件上限秒数 | integer | 30 | 3600 | revert | 同上 |
| 录像 保留天数 | integer | 1 | 365 | revert | 同上 |
| GB SIP 端口 | integer | 1 | 65535 | revert | wire 为 `u16` |
| GB RTP 起始 / 结束端口 | integer | 1 | 65535 | revert | wire 为 `u16`（偶数化由后端 `PortPool` 兜底，前端不抢） |
| GB 心跳超时（秒） | integer | 1 | 86400 | revert | wire 为 `u32`；一天为操作意义上界，防误输入（软约束，注明非后端硬约束） |
| 网络 子网前缀 | integer | 1 | 32 | null | `validate_ip_config` |
| 网络 Metric | integer | — | — | null | 语义与现状一致，不新增规则 |
| 存储 各保留天数（4 字段） | integer | 1 | 365 | revert | `StorageConfig::validate` |
| 存储 各配额 MB | integer | 0 | — | revert | `0 = 不限`，无上界 |
| 存储 水位 %（4 字段） | integer | 0 | 100 | revert | 比值换算仅在站点适配层（`v / 100`），组件只见百分数 |
| 存储 单批删除数 | integer | 10 | 500 | revert | `StorageConfig::validate` |

保存兜底（R3）：

- 录像卡片：沿用 `clampRecordingConfig`（已有单测），不重写。
- GB28181 `handleSave`：用 `clampNumericParam` 对 4 个数值字段再收敛。
- NetworkSettings `executeSave`：`prefix` 再 clamp 1–32 后发送。
- StorageSettings `handleSave`：新增 `features/system/storageDraft.ts#clampStorageDraft`（纯函数 + 单测），对天数 / 配额 / 水位 / 批大小收敛后提交。

## 4. 兼容性与影响面

- **无 wire 变更**：不新增/修改任何 HTTP 字段、根信封或 DTO；后端仅作最终兜底。
- **SSR 测试兼容**：组件初始草稿来自 `formatNumericDraft(value)`，无 DOM 访问、无 effect；`CameraDetailDrawer.test.tsx`（间接渲染录像卡片）初始展示与现状一致。
- **`AlgoParamDrawer` 回归面**：仅 import 来源变化；其测试（阈值双域）与 `type="number"` 输入不受影响。
- **依赖新增**：`jsdom`、`@testing-library/react`、`@testing-library/dom` 仅 devDependencies，不进产物；`check:cycles` 不新增跨层边。
- `recordingConfig.ts#clampToRange`（NaN→min、Math.round）与 `clampNumericParam`（纯 clamp）契约不同，本期不合并，避免牵连录像保存链路。

## 5. 测试设计

新增设施按文件 opt-in，不动全局环境（vitest 默认 node；`vite.config.ts` 不新增 `environment`）：

- 依赖：`cd web && pnpm add -D jsdom @testing-library/react @testing-library/dom`（RTL v16 需显式 dom 包）。
- jsdom 测试文件首行 docblock：`// @vitest-environment jsdom`。
- vitest 未开 `globals`，RTL 不会自动清理；每个 jsdom 文件显式 `afterEach(cleanup)`。

| 层 | 文件 | 覆盖 |
| --- | --- | --- |
| 纯逻辑 | `lib/numericDraft.test.ts`（自 `features/tasks/numericParam.test.ts` 迁移 + 扩充） | 中间态解析、clamp、整型格式化、无 max 不误收敛 |
| 组件 | `components/ui/NumericField.test.tsx`（jsdom） | 清空 → 重输不被回写；空串失焦 revert / null 两模式；非法非空回退；超界失焦收敛并显示边界；编辑中外部值变化不打断；非编辑态外部值同步 |
| 站点接线 | `features/cameras/components/RecordingConfigCard.test.tsx`（jsdom） | 清空重输；超界失焦收敛到 `RECORDING_LIMITS` |
| 站点接线 | `features/system/Gb28181Settings.test.tsx`（jsdom） | SIP 端口清空不再变 0；超界失焦收敛；保存 payload 带收敛值（mock `gb28181Api`） |
| 站点接线 | `features/system/NetworkSettings.test.tsx`（jsdom，可行则做） | 前缀空 → 保持空（null）；`0` → 1（不再折叠为 null） |
| 站点接线 | `features/system/StorageSettings.test.tsx`（jsdom） | 水位 150 → 100；批大小 0 → 10 |
| 纯逻辑 | `features/system/storageDraft.test.ts` | `clampStorageDraft` 边界 |

站点测试遵循 `quality-guidelines.md`：「先建立可复现的失败用例」——迁移前先让清空/失焦用例对旧实现失败（或在同一提交内以旧实现跑红、新实现跑绿验证），避免测试只复述新实现。

## 6. 风险与回滚

| 风险 | 缓解 |
| --- | --- |
| `type="number"` → `text` 改变桌面端步进箭头体验 | 设计系统本已重置原生外观；`inputMode` 保留移动端数字键盘 |
| 站点保存路径漏接兜底 | 每站点 checklist 单列；录像卡片复用既有测试证明兜底未回归 |
| jsdom 引入影响既有 30+ 测试 | 按文件 opt-in；全量 `pnpm test` 回归 |
| 依赖升级引起 lockfile 漂移 | 锁定 pnpm 解析版本，随改动一起评审 |

回滚：纯前端改动，无数据迁移；按 commit 粒度 revert 即可。风险文件的隔离点见 implement.md。

## 7. 刻意不做

- 不把 `AlgoParamDrawer` 迁入共享组件（cosine 双域换算语义特殊，见 PRD Out of Scope）。
- 不做跨字段关系校验（水位层级、`rtpStart < rtpEnd`），维持后端既有校验/兜底。
- 不统一各站点的视觉类族（不新增 `.numeric-field`），行为契约先行。
