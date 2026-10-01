# 统一前端抽屉技术设计

## 目标与边界

所有业务抽屉共用一个实体面板外壳和一致的结构槽位。业务 feature 继续负责数据加载、领域状态、按钮行为和内容渲染；共享 UI 不接触 API 或业务类型。

继续由 `ModalOverlay` 唯一承担 backdrop、动画、焦点陷阱、`useDismissStack`、ESC/Enter 路由及 `aria-modal`。新增 `Drawer` 组合层固定 `variant="drawer"`、`surface="solid"` 和零内边距，避免改动 `ModalOverlay` 默认玻璃材质而影响普通浮层。

## 组件结构

在 `web/src/components/ui/Drawer.tsx` 增加具名导出 `Drawer`：

- `Drawer` 接收 `isOpen`、`onClose`、必填 `closeLabel`、`closeDisabled`、`priority`、`layer`、`onExitComplete`、`size`、标题与可选 `titleTooltip`/图标/说明/metadata/headerActions/toolbar/footer、主体 `children` 和有限的主体 `className`。（实现复核后移除 `ariaLabel`：`aria-labelledby` 恒存在，该参数不会渲染到 DOM，属死参数。）
- 组件内部生成标题与说明 ID，渲染统一 header、可选固定 toolbar、唯一滚动主体、可选固定 footer；关闭按钮在共享 header 内使用 `CloseIconButton`，因此关闭回调和禁用状态不需要由调用点重复传递。
- 标题、说明、metadata、headerActions、toolbar 和 footer 使用 `ReactNode` 插槽，以支持复制 ID、状态徽标、日志复制操作和摄像头工具栏等现有业务内容。
- 主体拥有 `min-height: 0`、`flex: 1`、`overflow-y: auto` 和共享 padding；`className` 只补充业务内容间距，不覆盖外壳材质、尺寸或滚动职责。

`Drawer` 的 `size` 使用 `small | compact | medium | wide`，映射现有 24 / 28 / 36 / 42 rem 宽度类，迁移时将页面上的 `panelClassName` 选择改为语义尺寸，不改变当前各抽屉的宽度。

## 样式与主题

抽屉外壳固定使用 `ModalOverlay surface="solid"`，底色继承 `--bg-surface-solid`。新增抽屉结构 CSS 类使用现有 border、background、text 和 spacing token；标题栏、辅助工具栏与底栏不再自行叠加毛玻璃或不同的不透明表面。主体中的业务卡片可继续使用语义化 `--bg-secondary`、状态色等样式，不做无差别去样式化。

不修改 `ModalOverlay` 的默认 surface，不影响确认框、表单弹窗或媒体灯箱。实体面板在亮暗主题中都依赖根主题 token，不增加逐调用点的 `dark:` 颜色分支。

## 迁移映射

| 抽屉 | 尺寸 | 特殊结构需保留 |
| --- | --- | --- |
| `AccountPanelDrawer` | medium | 用户资料与改密入口；改密弹窗的浮层栈行为 |
| `PersonnelDetailDrawer` | medium | 可复制人员 ID、样本状态、嵌套确认/重提取弹窗、图片预览优先级与焦点行为 |
| `VersionsDrawer` | compact | 占用与加载提示、列表滚动、卸载确认的 `priority` 与禁用态 |
| `CameraDetailDrawer` | medium | 复制 ID 元数据、固定工具栏、退出动画后的快照清理 |
| `LogDetailDrawer` | wide | 复制全部操作、日志滚动区和现有底栏 |
| `AlgoParamDrawer` | compact | 参数草稿状态、固定保存/重置操作栏、取消和应用行为 |
| `AlgoSandboxDrawer` | small | 算法元数据、导航入口；将当前整面板滚动改为共享主体滚动，底部完成操作放入 footer |

迁移后以上业务组件不再直接使用 `ModalOverlay` 实现 drawer shell，也不再各自拼接外层 header/body/footer 类。人员详情中的全屏照片预览和业务确认弹窗不是该 drawer shell 的组成部分，保持现有独立浮层实现。

## 可访问性与生命周期

`Drawer` 内部把 `closeDisabled` 同步应用于 `ModalOverlay` 和 `CloseIconButton`；`priority`、`layer`、`onExitComplete` 与说明标签原样传给 `ModalOverlay`。组件不创建第二套键盘监听或焦点逻辑。关闭按钮名称由 feature 提供本地化 `closeLabel`。保留人员抽屉、算法版本抽屉内部更高优先级浮层，以及摄像头详情数据快照的退场清理。`ModalOverlay` 的 `ariaLabel` / `ariaLabelledBy` 在类型上约束为「至少提供其一」（同时提供时以 `ariaLabelledBy` 为准），`Drawer` 只传后者。

## 风险与回退

复查结论（两轴 code review 后的修复批次）：① `Drawer` 删去从未渲染的 `ariaLabel`，改由 `ModalOverlay` 的类型联合约束「至少提供其一」；② 新增 `titleTooltip` 并回填摄像头抽屉的长设备名悬浮提示；③ `.drawer-footer > :only-child` 接管单一操作项靠右，调用点去掉 `flex justify-end` 包裹；④ 抽屉主体内与实体面板重复的 `backdrop-blur` / `.frosted-glass` 改为语义背景（仅保留压在照片上的角标，其模糊用于未知底图的对比度）；⑤ 固定头尾与唯一滚动区的契约新增 CSS 规则级测试守护。

主要风险是迁移 `p-0` 后遗漏某个内容区 padding、将原固定工具栏并入可滚动主体，或破坏嵌套弹层关闭顺序。共享组件测试验证实体材质、尺寸和插槽结构；既有业务测试及 hook 测试保护数据与浮层行为。若单一 feature 出现回归，可先回退该 feature 的 `Drawer` 迁移；由于 `ModalOverlay` 默认值保持不变，普通 modal 不受影响。

## 规范与接口

这是纯前端 UI 改动，无后端/API/DTO 变更，不需要 `api.md`。实现后更新 web 组件与样式规范，明确业务抽屉统一使用 `Drawer`、实体材质及共享结构。Codex 当前为 inline dispatch 模式，context JSONL 配置步骤按工作流跳过，执行阶段使用 `trellis-before-dev` 加载规范。
