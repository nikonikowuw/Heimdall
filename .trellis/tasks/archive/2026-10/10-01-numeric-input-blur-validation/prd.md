# 规范并修复前端数值输入框的失焦校验契约

## Goal

统一 Heimdall Web 端数值输入的校验时机契约：**编辑期只接受键盘中间态、失焦时收敛、提交时兜底**；修复当前"输入期即校验/回写"导致的输入框无法清空、超界值静默、`0` 被折叠为 `null` 等缺陷。契约固化到 `.trellis/spec/web/frontend/`，避免后续新表单再次漂移。

## Background

### 现状

- `.trellis/spec/web/frontend/` 现有规范没有覆盖数值输入的校验时机；`styling-guidelines.md` 只定义了 `.search-field` / `.select-field` / `.modal-form-field` 的视觉类族。
- 仓库中已有事实范例（未固化进 spec，本次作为契约样本）：
  - `web/src/features/tasks/numericParam.ts`：`parseNumericDraft` 保留空串 / `-` / `.` 等键盘中间态；`clampNumericParam` 收敛区间；`formatNumericDraft` 格式化回写；已有单测 `numericParam.test.ts`。
  - `web/src/features/tasks/components/AlgoParamDrawer.tsx`：字符串草稿 `numberDrafts` + `onBlur → commitNumericDraft` + `getFinalParams()` 提交兜底；其 `:65` 注释明确声明「校验与取整推迟到失焦或应用时执行」。
  - `web/src/components/DateTimeRangePicker.tsx:810`：时/分/秒 `onBlur → clampPad` 收敛。

### 缺陷证据

见 Requirements R5–R8 的 owning 条目（严重度与 file:line 锚点已内联）。

### 后端约束（前端收敛边界的同源依据）

- `crates/types/src/system.rs` `StorageConfig::validate()`：retention 1-365；`batch_delete_size` 10-500；水位层级关系 `target > min > emergency > critical`。
- `crates/api/src/network_service/mod.rs` `validate_ip_config()`：静态 IP `prefix` 1-32。
- `crates/pipeline/src/recording/config.rs` `RecordingConfig::normalized()`：与前端 `RECORDING_LIMITS`（`web/src/features/cameras/recordingConfig.ts`）区间一致，已有单测。
- `crates/types/src/gb28181.rs`：端口字段为 `Option<u16>`；`crates/media/src/gb28181/port_pool.rs` 对 start 偶数化、end 最小跨度做静默兜底。

## Requirements

- **R1 编辑期只维护草稿**：`onChange` 只更新本地字符串草稿并接受键盘中间态（空串、`-`、`.`、不完整小数）；显示值永远来自草稿。共享字段组件编辑期不派发数值回调（R4 的直接推论）；确需实时联动的站点自行组合自有控件（如 `AlgoParamDrawer` 的滑块）。
- **R2 失焦收敛**：`onBlur` 解析草稿 → 按字段语义处理：必填字段空或不可解析均回退上次合法值；可空字段空串落 `null`，不可解析非空串同样回退（不把笔误当清空意图）→ 可解析则 clamp 到 `[min, max]` → 格式化回写显示。
- **R3 提交兜底**：保存/应用路径必须对最终值再做一次 clamp，不假设 UI 已收敛。
- **R4 中间态不污染模型**：草稿可暂时非法，但未收敛的中间态不得写入提交模型。
- **R5（高）** `web/src/features/cameras/components/RecordingConfigCard.tsx:41` `NumberField`（事件前后录制、单文件上限、保留天数 4 字段）：`value` 直绑 `number` + `parseInt` guard 拦截中间态，**清空后值原样弹回、无法重输**；超界仅原生 `min/max` 提示，clamp 只在保存时执行。修复为清空重输 + 失焦收敛到 `RECORDING_LIMITS`。
- **R6（高）** `web/src/features/system/Gb28181Settings.tsx:265,313,340,351`（SIP 端口、心跳超时、RTP 起止端口 4 字段）：`value` 直绑 `number` + `Number(e.target.value)`，**清空即变 `0`**；无范围约束与失焦收敛。修复为清空重输 + 失焦收敛（边界见 design 第 3 节参数表）。
- **R7（中）** `web/src/features/system/NetworkSettings.tsx:552`（子网前缀）：`Number(v) || null` 把 `0` 折叠为 `null`；失焦无收敛。修复为 `0 → clamp(1, 32)` 且空串保持 `null`。
- **R8（中）** `web/src/features/system/StorageSettings.tsx:752` `NumericInput`：失焦不做 clamp；输入期即把未收敛值 live commit 进草稿。修复为失焦 clamp，并保持「编辑中不同步外部值」行为。
- **R9** 将契约固化进 `.trellis/spec/web/frontend/`（新增「数值输入与校验时机」小节），引用共享实现。
- **R10** 补测试：清空→重输、失焦收敛超界、中间态不写模型、空值回退语义。

## Acceptance Criteria

- [ ] R5-R8 涉及的所有数值输入框：编辑中可清空并重新输入完整数值，中途不被强制回写。
- [ ] 失焦行为：可解析且超界 → 收敛到边界；空且必填 → 回退上次合法值；不可解析 → 回退；可空字段空 → `null`。均有组件测试覆盖。
- [ ] 保存路径仍执行最终 clamp（复用既有测试并新增用例）。
- [ ] `pnpm format && pnpm lint && pnpm typecheck && pnpm test && pnpm check:cycles && pnpm build` 全绿。
- [ ] spec 新小节包含三段式契约、空值语义与参考实现链接。

## Out of Scope

- `AlgoParamDrawer` 的 cosine 阈值双域显示（score ↔ percent）改造（除非确认迁移到共享组件）。
- StorageSettings 水位层级关系的即时前端跨字段校验（仍由保存时 `StorageConfig::validate` 报错）。
- GB28181 RTP 起止端口跨字段关系（start < end）的前端校验（后端 `PortPool` 已有静默兜底）。
- 非数值输入框（文本、搜索）与 `select`、`range` 滑块控件。

## Key Decisions

- **D1 修复载体 = 全共享（用户 2026-10-01 确认）**：提取 `lib/` 纯逻辑 + `components/ui/` 共享字段组件，4 处缺陷站点迁移；`AlgoParamDrawer` 保留自有 UI（cosine 双域）但改吃共享纯逻辑。理由：根因是契约未固化 + 逻辑未共享，就地修复会留 4 份重复实现并让同类缺陷再长出来。
- **D2 测试设施 = 引入 jsdom + Testing Library（用户 2026-10-01 确认）**：新增 devDependencies `jsdom` / `@testing-library/react` / `@testing-library/dom`，按文件 `// @vitest-environment jsdom` opt-in，不改全局 node 环境与既有 SSR 测试；先写清空→重输、失焦收敛的失败交互用例再修。理由：本次 bug 恰是受控组件交互层问题，纯函数测不到；仅按文件 opt-in，对既有 30+ 测试零影响。

## Open Questions

（无）
