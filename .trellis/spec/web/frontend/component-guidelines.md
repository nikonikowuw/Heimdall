# 组件规范

## 接口与归属

- 使用具名函数导出，不用 `export default` / `React.FC`；`<ComponentName>Props` 放在组件附近，默认值在参数解构中设置。
- 展示组件接收数据和回调，请求交给资源 hook/页面容器与统一 API client。
- 动态列表 key 使用稳定业务 ID；高频事件列表项用 `memo`，回调保持稳定。
- 复用现有 UI 与主题样式；所有可见文本遵循 [i18n](directory-structure.md#国际化)。

## 实时页面

- 播放器用 `React.memo` 隔离，camera/source/回调等 props 保持稳定，无关事件不重建播放器。
- 15～30fps 检测元数据放 `useRef`/Worker，通过 `requestAnimationFrame` 绘制透明 Canvas；不使用 React state 驱动高频 DOM。
- Zustand 按字段订阅，见 [状态规范](state-management.md)；事件缓冲有界。
- 常态监控保持低噪，违规时通过稀疏事件显示轻量告警卡片与声音，不持续展示无违规噪点框。
- ROI / Mask / Line 编辑叠加在动态子码流上，坐标规范见 [全局约定](../../guides/conventions.md#坐标)。

## 播放器生命周期

实现入口：[LivePlayer](../../../../web/src/components/LivePlayer.tsx)、[WebCodecs 能力与协议](../../../../web/src/lib/webcodecs.ts)。

- **音视频解耦与伴生音频架构**：
  - **主视频通道纯净度**：主视频播放器（WebCodecs Canvas 或 mpegts.js `<video>`）必须严格保持纯视频模式（`hasAudio: false, hasVideo: true`，视频元素 `muted = true`），彻底杜绝安防摄像头无音频或非 AAC 格式造成浏览器 MSE SourceBuffer 饥饿而永久死锁挂起；
  - **伴生音频按需启停**：音频由独立的伴生 `<audio>` 播放器通过 `cameraApi.getLiveStreamUrl(cameraId, streamType, true, false)` 独立拉取纯 AAC-only FLV 流；
  - **静音/开启切换零闪烁**：用户在界面开关音频仅在后台启动或销毁伴生音频播放器，严禁触发主视频播放器重启或更新 `retryKey`；
  - **故障隔离原则**：伴生音频播放器的任何错误（如摄像头无音频、自动播放受阻或解析失败）仅记录警告，严禁向上传播为全局连接失败（`setConnectionStatus('failed')`）或触发主画面重试。
- HTTP-FLV/mpegts.js 支持 H.264 与 Enhanced FLV H.265（`hvc1`）；实际硬解能力通过浏览器检测，不承诺所有浏览器都支持。
- **WebCodecs 优先策略**：浏览器支持 WebCodecs 时，优先使用 WebCodecs + Canvas 零拷贝超低延迟渲染，伴生音频在开启时由伴生 `<audio>` 播放器同步接管，不受编码和音轨状态牵制。
- 辅轨拉子码流，主屏默认主码流并支持切流；预览切流不决定后端分析码流，分析侧遵循 [媒体管线](../../media/backend/media-pipeline.md) 的 `main` / `sub` / `auto` 契约，不能假定仅对子码流常驻推理。
- 卸载、换摄像头或换协议时销毁旧播放器、SourceBuffer、Worker、订阅、定时器与 RAF；MSE 实例必须调用 `destroy()`。
- 断流展示可恢复状态，重试规则见 [错误处理](error-handling.md#连接恢复)。

## 交互

- 控件使用语义化 button/link，图标按钮有翻译后的 `aria-label`，表单关联 label，保持可见焦点。
- 普通玻璃面板沿用 `.frosted-glass`；业务抽屉通过共享 `Drawer` 使用实体材质；自定义光标目标加 `.reticle-target`，生成 UI 通过封装定制。
- 浮层按栈响应 ESC，避免穿透关闭外层；表单/确认弹窗支持 Enter。两者均由 [use-dismiss-stack](../../../../web/src/hooks/use-dismiss-stack.ts) 的同一浮层栈分发（LIFO + `priority`），组件不各自挂 `window` 键盘监听；确认类弹窗传 `onConfirm` 才接管 Enter，焦点在 `INPUT` / `TEXTAREA` 或可编辑区时让行。
- **浮层外壳必须复用组件，不得在业务文件里自建**：遮罩、层级、进出场动效、焦点陷阱与 `aria-modal` 语义统一由 [ModalOverlay](../../../../web/src/components/ui/ModalOverlay.tsx) 承担；业务抽屉必须经 [Drawer](../../../../web/src/components/ui/Drawer.tsx) 组合层接入，普通浮层可按需直接使用 `ModalOverlay`。禁止再写 `<div className="modal-backdrop">` + `<motion.div className="modal-surface">` 这类外壳：CSS 类族只保证视觉一致，无法保证焦点约束与键盘语义，手写外壳会静默产生「声明了 `aria-modal` 却没有焦点陷阱」的可访问性缺口。
  - `Drawer` 固定 `variant="drawer"` 与 `surface="solid"`，统一标题/关闭区、可选工具栏、唯一滚动主体和可选固定底栏；调用方通过 `title`、`description`、`icon`、`metadata`、`headerActions`、`toolbar`、`footer` 提供业务内容，通过 `small` / `compact` / `medium` / `wide` 选择尺寸。业务数据和操作留在 feature，不在调用点重拼外壳类或另建滚动容器。
  - `Drawer` 自动将标题和说明关联至 `aria-labelledby` / `aria-describedby`，因此调用方**不再传 `ariaLabel`**（`aria-labelledby` 存在时它不会渲染到 DOM，重复传属死参数）；长标题需要原生悬浮提示时传 `titleTooltip`。关闭名称使用本地化 `closeLabel`；`closeDisabled` 同步约束遮罩/ESC 与关闭按钮，`priority`、`layer`、`onExitComplete` 原样交给 `ModalOverlay`。
  - 底栏只有一个操作项时无需再包 `flex justify-end`（`.drawer-footer > :only-child` 已靠右）；多项时按「说明在左、操作在右」的顺序传入。
  - `ModalOverlay` 的 `ariaLabel` 与 `ariaLabelledBy` 由类型约束为「至少提供其一」：有可见标题节点时一律用 `ariaLabelledBy` 并省略 `ariaLabel`。
  - 直接使用 `ModalOverlay` 时通过 props 表达变体：`variant="drawer"` 侧滑、`surface="solid"` 实体面、`layer` 取 `base`/`raised`/`top`/`highest`（对应 `--layer-*`，不在组件内写 `z-[70]` 等魔法值）、`priority` 调浮层栈顺序、`onConfirm` 接管 Enter、`onExitComplete` 清理为退场动画保留的数据快照。
  - 沉浸式媒体查看器（灯箱）的遮罩即内容面，无法套用 panel 结构时，可保留自定义根元素，但必须自行接入 [useFocusTrap](../../../../web/src/hooks/use-focus-trap.ts) 与 `useDismissStack`，并补齐本地化 `aria-label`。
- 二次确认弹窗（危险/警告操作）复用 [ConfirmDialog](../../../../web/src/components/ui/ConfirmDialog.tsx)：标题、说明、图标、页脚操作与提交态由该原语提供，需人工核对的对象信息与预览清单通过 `children` 传入，失败原因走 `errorMessage`（Toast 与内联错误只由一处负责）。
- 关闭按钮复用 [CloseIconButton](../../../../web/src/components/ui/CloseIconButton.tsx)，内联错误提示复用 [FormErrorAlert](../../../../web/src/components/ui/FormErrorAlert.tsx)，不在业务文件重复拼这组类名。
- 创建与编辑表单弹窗复用 `ModalFormHeader`、`.modal-surface--form`、`.modal-form-content` 与 `.modal-form-footer`；保留业务字段分组，但标题、滚动区、按钮区不另起一套视觉结构。标题与描述必须通过 `aria-labelledby` / `aria-describedby` 关联，关闭按钮使用本地化 `aria-label`。
  - **弹窗内的工具条必须走共享类族，不得手拼**：视图切换 / 过滤 / 次要动作放进 `.modal-form-toolbar`（详见[样式规范](styling-guidelines.md#表单弹窗)）。头部、工具条、页脚三者固定，唯一滚动区是 `.modal-form-content`；把滚动加在外层 `div` 或用 `mt-*` 凑分区会产生双内衬与头部不齐。
  - **尺寸用 `modal-surface--*` token，不传裸 `max-w-*`**：裸 utility 读不出「这是哪一档」，与 `Drawer` 的 `small` / `compact` / `medium` / `wide` 词表脱节；即使当前宽度数值恰好相同（如 `max-w-2xl` 与 `--medium` 均为 42rem、`max-w-xl` 与 `--xl` 均为 36rem），也必须走 token，否则后续调整只能逐个调用点改。
  - **尺寸档位由调用方显式声明，`ModalOverlay` 不注入默认档位**：档位值域为 `--small` 24rem / `--compact` 28rem / `--narrow` 32rem / `--xl` 36rem / `--medium` 42rem / `--wide` 48rem / `--extra-wide` 64rem；未声明时回落 `.modal-surface` 基类的 36rem。同一元素带上两个档位时，同层同权重的规则只能靠源序决出胜负，未声明的档位会被静默改宽（历史上 `ModalOverlay` 曾无条件注入 `--wide`，导致 `--small` / `--compact` / `--narrow` 调用点实际渲染为 48rem）；该契约由 `ModalOverlay.test.tsx` 守护，不要恢复注入。
  - **只有带滚动/页脚的查看器是否收敛到 form 族由内容形态决定**：无工具条、无固定页脚且内容自然撑高的只读查看器可直接用 `ModalOverlay` + `panelClassName`；一旦出现固定页脚或独立滚动区，就必须收敛到 form 三区结构，避免与同域表单弹窗出现两套内衬与标题视觉。
  - **不要用片段标识符做 id**：标题/描述的 `id` 一律 `useId()` 生成。`CreateTaskModal` 里的 `id="create-task-title"` 是硬编码字面量，同一页面渲染多个实例时会重复 id 并使 `aria-labelledby` 指向首个节点；新代码不再照抄该写法。
- 高风险媒体/图形视口局部错误隔离，不牵连导航和其他监控路。

验证无关事件下播放器稳定、动态 key、键盘行为及反复挂载/切流后的资源释放。

## 数值输入与校验时机

人工录入数值的字段（端口、秒数、天数、配额、百分比、Metric 等）统一遵循**三段式契约：编辑期只维护草稿、失焦时收敛、提交时兜底**。共享参考实现为 [NumericField](../../../../web/src/components/ui/NumericField.tsx)（行为组件）与 [numericDraft](../../../../web/src/lib/numericDraft.ts)（纯逻辑，无 React）；站点只负责提供边界与空值语义，并在保存路径保留兜底。

- **编辑期只维护草稿**：`onChange` 只更新本地字符串草稿并接受键盘中间态（空串、`-`、`.`、不完整小数），显示值永远来自草稿；共享字段组件编辑期**不派发数值回调**，未收敛的中间态不得写入提交模型。确需滑块 ↔ 数值实时联动的站点自行组合自有控件，共享组件刻意不做实时派发。
- **失焦/Enter 收敛**：解析草稿 → 按下方语义表处理 → 可解析则 clamp 到 `[min, max]` → 派发一次数值回调，并把草稿原地格式化为提交值显示（不依赖父层回传，避免一帧回跳）。外部值同步沿用仓库既有模式：非编辑态外部值变化时同步草稿，编辑中刻意不同步。
- **提交兜底**：保存/应用路径必须对最终值再做一次 clamp，不假设 UI 已收敛（先例：[recordingConfig](../../../../web/src/features/cameras/recordingConfig.ts) 的 `clampRecordingConfig`）。

### 失焦语义表

| 失焦时的草稿             | 必填（`emptyBehavior` 缺省为 `revert`）   | 可空（`emptyBehavior="null"`）                |
| ------------------------ | ----------------------------------------- | --------------------------------------------- |
| 空串 / 纯空白            | 回退上次合法值，不派发                    | 派发 `null`，界面保持空白                     |
| 不可解析非空（如 `abc`） | 回退，不派发                              | **回退**（笔误不是清空意图，不降级为 `null`） |
| 可解析、区间内           | 原样派发                                  | 同左                                          |
| 可解析、超界             | clamp 到边界后派发，显示边界值            | 同左                                          |
| 可解析、`0` 且 `min > 0` | clamp 到 `min` 派发（`0` 不得折算成空值） | 同左                                          |

实现入口与约束：

- **判别联合 props**：`emptyBehavior: 'revert'` 配 `value: number` / `onChange: (value: number) => void`；`emptyBehavior: 'null'` 配 `value: number | null` / `onChange: (value: number | null) => void`。让「可空字段忘记处理 null」与「必填字段被迫处理 null」都成为类型错误，不用 `value: number | null` 的弱签名。
- **`label` 必填**并落到 `aria-label`（漏写从运行期缺陷变成编译期错误，沿用 [SearchInput](../../../../web/src/components/ui/SearchInput.tsx) / [SelectField](../../../../web/src/components/ui/SelectField.tsx) 的做法）；组件不内置 label 元素，标签排布与视觉由调用点 `className` 决定，不在本次新增 `.numeric-field` 类族（视觉类族见[样式规范](styling-guidelines.md#工具栏输入控件)）。
- **`type?: 'number' | 'integer'`** 只决定草稿解析与格式化：`integer` 拒绝小数草稿并在收敛时取整；`min` / `max` 是语义边界，只参与收敛、不落到 DOM —— 原生 `min`/`max` 不阻挡键入、只影响 `:invalid`，站点又不在 `<form>` 提交路径上。
- **外部值同步必须能处理非有限数**：草稿同步用 `Object.is` 比较外部值（而非 `!==`）—— `NaN` 与自身不相等，用 `!==` 会让「外部值已变化」永为真，在渲染期同步的写法下直接造成 Too many re-renders 渲染循环；非有限外部值（`NaN` / `Infinity`）一律显示为空串，不渲染字符串 `'NaN'`。
- 渲染为 **`type="text"` + `inputMode`**（`integer` 为 `numeric`，否则 `decimal`）：`type="number"` 各浏览器对 `-`、`1.`、空串的 `value` 归一化不一致，恰好会破坏要保留的键盘中间态。不宣告 `role="spinbutton"`：未实现方向键步进却声明该角色会向辅助技术谎报能力。
- **新表单不得自建受控 number 输入**：禁止 `value` 直绑 `number` + `onChange` 里 `Number(...)` / `parseInt` 的组合 —— 它会导致清空弹回、`0` 被折叠为空值、超界静默。数值字段一律用 `NumericField`；站点内只保留保存侧兜底纯函数。

测试约定：

- 解析、clamp、格式化等纯逻辑下沉 `lib/numericDraft.ts`，在默认 node 环境用单测覆盖中间态与边界，见 [numericDraft.test.ts](../../../../web/src/lib/numericDraft.test.ts)。
- 交互行为（清空 → 重输、失焦收敛、编辑期不派发、外部值同步）用 RTL + jsdom：文件首行 `// @vitest-environment jsdom` 按文件 opt-in，不修改全局 `test.environment`；仓库未开 vitest `globals`，每个 jsdom 文件显式 `afterEach(cleanup)`；不依赖 jest-dom 断言或 `user-event`，`fireEvent.change` + `fireEvent.blur` 足以表达失焦语义。见 [NumericField.test.tsx](../../../../web/src/components/ui/NumericField.test.tsx) 与[测试选择](quality-guidelines.md#测试选择)。

验证清空 → 重输不被回写、失焦按语义表收敛、编辑期不派发、保存路径仍有最终 clamp。
