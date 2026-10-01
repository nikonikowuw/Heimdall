# 实施与验证计划

> planning / 待确认。本计划不是启动授权。需求见 prd.md，技术设计见 design.md。纯前端任务（无 wire 变更），按 design 第 4 节跳过 `api.md` 并在下方记录理由。

## 0. 开工门禁与变更边界

- [ ] 用户确认最终规划摘要（含 design 第 3 节站点参数表：GB 端口 1–65535、心跳 1–86400 软上界、水位 0–100、配额 min 0 无上界；前缀 1–32）。
- [ ] 该摘要之后的明确实施批准，再执行 `task.py start`；当前保持 planning。
- [ ] 保留工作区已有修改；不触碰 `.trellis/.template-hashes.json`、`crates/algo-sdk/**`、`09-30-edge-torch-algo-ecosystem` 删除项等无关改动。
- [ ] 跳过 `api.md` 的理由：不新增/修改任何 HTTP 字段、DTO、信封或错误码；前端收敛边界与既有后端校验同源，后端仅作最终兜底。
- [ ] 按 web 包 spec 读取规范（`web/frontend/index.md` 的 Pre-Development Checklist）；上下文清单见 implement.jsonl / check.jsonl。

变更边界（不做什么）：

- 不动 `AlgoParamDrawer` 的 UI 与 cosine 双域换算，仅换 import 来源。
- 不做跨字段校验（水位层级、`rtpStart < rtpEnd`）、不改后端、不改 DTO、不新增 `.numeric-field` 视觉类族。
- 不合并 `recordingConfig.ts#clampToRange` 与 `clampNumericParam` 的不同 NaN 语义。

## 1. 交付拆分与风险隔离

| 单元 | 顺序 | 可独立验证 / 回滚点 |
| --- | --- | --- |
| U1 测试设施 | 首先 | `pnpm test` 既有 30+ 测试全绿；无环境影响 |
| U2 纯逻辑 `lib/numericDraft.ts` | 依赖 U1 | 单测全绿；不影响任何站点 |
| U3 `numericParam.ts` 收缩 + AlgoParamDrawer import | 依赖 U2 | 算法抽屉既有测试全绿 |
| U4 `NumericField` 组件 + jsdom 测试 | 依赖 U1/U2 | 组件测试先红后绿；SSR 测试不受影响 |
| U5 RecordingConfigCard 迁移 | 依赖 U4 | 录像卡片测试；保存链路 `clampRecordingConfig` 不回归 |
| U6 Gb28181Settings 迁移 | 依赖 U4 | 站点测试；保存 payload 断言 |
| U7 NetworkSettings 迁移 | 依赖 U4 | 站点测试（不可行则记录原因，转手工清单） |
| U8 StorageSettings 迁移 + `storageDraft.ts` | 依赖 U4 | 站点测试 + 纯逻辑测试 |
| U9 spec 固化 | 依赖 U4（参考实现存在后） | 文档自检 |
| U10 全量门禁 + 手工核对 | 最后 | 见第 3 节命令 |

U5–U8 相互独立，可逐个合入；每单元自带测试与回滚点。若中途需要缩小范围，U9 可最后并入。

## 2. 实施清单

### U1 测试设施

- [ ] `cd web && pnpm add -D jsdom @testing-library/react @testing-library/dom`（RTL v16 的 dom 为显式 peer）。
- [ ] 不改 `vite.config.ts` 全局 `test.environment`；jsdom 测试文件首行加 `// @vitest-environment jsdom` docblock。
- [ ] jsdom 文件显式 `afterEach(cleanup)`（仓库未开 vitest `globals`，RTL 不会自动清理）。
- [ ] 不引入 `@testing-library/user-event`：`fireEvent.change` + `fireEvent.blur` 足够覆盖清空/失焦语义。

### U2 纯逻辑层

- [ ] 新建 `web/src/lib/numericDraft.ts`：迁入 `parseNumericDraft`、`clampNumericParam`、`formatNumericDraft`、`isFiniteNumber`（自 `features/tasks/numericParam.ts` 平移，逻辑不变；补注释说明中间态契约）。
- [ ] 新建 `web/src/lib/numericDraft.test.ts`：迁移 `features/tasks/numericParam.test.ts` 中解析/收敛/格式化用例，并扩充「可解析超界 → 边界」「整型草稿 `12.5` 不提交」「空串保留」。
- [ ] `features/tasks/numericParam.ts` 收缩为 `NumericParamType` / `NumericParamConfig` / `getNumericParamConfig` / `isFiniteNumber` 再导出（仅保留 schema 语义），其余 import 改为 re-export 或直接引用 `@/lib/numericDraft`；由 `check:cycles` 与 typecheck 验证无环。

### U3 AlgoParamDrawer 接线

- [ ] `features/tasks/components/AlgoParamDrawer.tsx` 更新 import 来源；行为零变化。
- [ ] 运行 `AlgoParamDrawer.test.tsx` 与 `numericParam.test.ts` 全绿。

### U4 `NumericField` 组件

- [ ] 新建 `web/src/components/ui/NumericField.tsx`：按 design 第 2 节实现判别联合 props、`type="text"` + `inputMode`、三段式时序、失焦语义表。
- [ ] `label` 必填并落到 `aria-label`；`className` 透传到 `<input>`；不新增类族、不内置 label 元素。
- [ ] 渲染期同步外部值（沿用 `StorageSettings` 既有注释同款模式），非编辑态生效、编辑中不打断。
- [ ] 新建 `web/src/components/ui/NumericField.test.tsx`（jsdom）：
  - [ ] 清空 → 重输完整数值，中途 `value` 不被回写（核心回归，先对旧站点实现验证失败语义）。
  - [ ] 必填模式空串失焦 → 回退且不派发；可空模式空串失焦 → 派发 `null`。
  - [ ] 非法非空（`abc`）失焦 → 回退（可空模式也不降级为 `null`）。
  - [ ] 超界失焦 → clamp 派发且显示边界；`0` 于 min=1 → 1。
  - [ ] 编辑中外部值变化不打断；非编辑态外部值变化同步显示。

### U5 RecordingConfigCard

- [ ] 删除本地 `NumberField`，改用 `NumericField`（integer、`RECORDING_LIMITS`、revert）。
- [ ] 保留 `clampRecordingConfig` 保存兜底与现有纯逻辑测试。
- [ ] 新建 `features/cameras/components/RecordingConfigCard.test.tsx`（jsdom）：清空重输；超界失焦收敛；保存按钮 payload 含收敛值（mock `cameraApi`）。
- [ ] `CameraDetailDrawer.test.tsx`（SSR）保持全绿。

### U6 Gb28181Settings

- [ ] 4 个输入改 `NumericField`（integer、1–65535 / 1–86400、revert）。
- [ ] `handleSave` 用 `clampNumericParam` 对 4 字段最终收敛后再发请求。
- [ ] 新建 `features/system/Gb28181Settings.test.tsx`（jsdom，mock `gb28181Api`）：SIP 端口清空不再变 0；超界失焦收敛；保存 payload 断言。

### U7 NetworkSettings

- [ ] 前缀改 `NumericField`（nullable、integer、1–32）：空 → `null` 保持空；`0` → clamp 1（不再折叠）。
- [ ] Metric 改 `NumericField`（nullable、integer、无区间）。
- [ ] `executeSave` 增加 `prefix` 最终 clamp 1–32。
- [ ] 新建 `features/system/NetworkSettings.test.tsx`（jsdom，mock `systemApi`）；若表单依赖较重导致测试成本失衡，改为记录原因 + 手工核对清单项。

### U8 StorageSettings

- [ ] 新建 `features/system/storageDraft.ts`：`clampStorageDraft`（天数 1–365、配额 min 0、水位 0–100、批大小 10–500）+ 单测（对齐 `recordingConfig.ts` 先例）。
- [ ] 删除本地 `NumericInput` / `NumberInput`；`RetentionRow`、高级设置全部改 `NumericField`，按字段传真实区间（修复包装层写死 0–100 的问题）。
- [ ] `handleSave` 走 `clampStorageDraft` 兜底。
- [ ] 新建 `features/system/StorageSettings.test.tsx`（jsdom）：水位 150 → 100；批大小 0 → 10；保存 payload 断言；`0=不限` 配额保持 0。

### U9 spec 固化

- [ ] `.trellis/spec/web/frontend/component-guidelines.md` 新增「数值输入与校验时机」小节：三段式契约、失焦语义表（必填/可空）、提交兜底要求，引用 `NumericField.tsx` 与 `lib/numericDraft.ts` 为参考实现。
- [ ] 若新小节与 `.trellis/spec/guides/conventions.md` 有交叉，只引用不重复展开。
- [ ] 文档改动随任务提交，不另行运行无关构建。

### U10 全量门禁与手工核对

见第 3 节。

## 3. 验证命令

```bash
cd web
pnpm format
pnpm lint          # eslint --max-warnings=0
pnpm typecheck     # tsc --noEmit
pnpm test          # vitest run（含新 jsdom 用例）
pnpm check:cycles  # 模块图不得有循环依赖（含 import type）
pnpm build
```

手工核对清单（自动化覆盖不到时）：

- [ ] 三语（zh-CN / zh-TW / en）下新组件 `aria-label` 正确。
- [ ] 录像卡片：清空 → 重输 → 保存，落库值正确。
- [ ] GB28181：端口改 70000 → 失焦显示 65535；保存后回显一致。
- [ ] 网络设置：静态前缀清空 → 保持空；输入 0 → 失焦显示 1。
- [ ] 存储设置：水位/批大小超界失焦收敛；`0=不限` 配额保存为 0。
- [ ] 键盘操作可见焦点；无 `console` 报错。

## 4. 风险与回滚

- 风险文件（改动集中、需重点 review）：`StorageSettings.tsx`（多字段 + 复合行组件）、`Gb28181Settings.tsx`（mock API 边界）、`NumericField.tsx`（新组件契约）。
- 回滚粒度：U2–U9 各自独立可 revert；新组件与纯逻辑层为纯增量，删除即回滚。
- 无数据迁移、无后端变更、无特性开关需求。
