# 统一抽屉实施计划

## 执行顺序

1. [x] 实施前读取 web 规范入口及组件、样式、质量检查指南；复查 7 个目标抽屉及现有组件/hook 测试。用户批准规划后运行 `task.py start`，再进入代码实现。
2. [x] 新增 `components/ui/Drawer.tsx`，实现实体材质抽屉壳、small/compact/medium/wide 尺寸、统一生成的 header、可选 toolbar/footer 和唯一主体滚动区。新增抽屉结构 CSS 类和共享组件测试，验证 solid surface、各尺寸映射、关闭标签/禁用态、固定头尾与可滚动主体。
3. [x] 迁移 AccountPanelDrawer、VersionsDrawer、AlgoSandboxDrawer 和 LogDetailDrawer，分别覆盖无 footer、动态列表/嵌套卸载确认、小型元数据/完成操作、wide 详情/复制操作等组合。
4. [x] 迁移 CameraDetailDrawer、PersonnelDetailDrawer 和 AlgoParamDrawer；保留摄像头固定操作工具栏及退出清理、人员复制/状态内容及优先级更高的图片/确认浮层、算法参数本地草稿与固定应用栏。
5. [x] 检查全部 7 个调用点：统一传入语义尺寸；移除抽屉外壳上的玻璃材质、重复 padding 和各自拼写的结构布局；保留业务卡片的语义化 token、翻译键、请求逻辑和按钮行为。
6. [x] 更新 `.trellis/spec/web/frontend/component-guidelines.md` 与 `styling-guidelines.md`，记录 Drawer 组合契约、实体材质、尺寸和业务内容边界；不新增 `api.md`（纯前端变更）。
7. [x] 执行 `pnpm format`、lint、typecheck、全量测试、循环依赖检查和生产构建；476 项测试通过，循环依赖检查与 build 成功。build 仅报告 chunk 大小提示。
8. [ ] 进行亮/暗主题、窄/宽视口、各宽度变体、长内容滚动、头尾固定和嵌套浮层的浏览器视觉检查。当前环境未安装 Playwright/Chromium；本地 Vite 开发服务已启动于 `http://127.0.0.1:5174/`，可继续人工检查。
9. [x] 两轴 code review（Standards / Spec）后的修复批次：`Drawer` 移除死参数 `ariaLabel`、`ModalOverlay` 名称契约改为类型联合、新增 `titleTooltip` 并回填摄像头抽屉、`.drawer-footer > :only-child` 自动靠右并去除调用点包裹容器、清除与实体面板重复的 `backdrop-blur`/`.frosted-glass`、补 CSS 规则级结构测试。(`.trellis/.template-hashes.json` 与 `.trellis/workspace/niko/` 属 Trellis 工具链产物，不随本任务提交。)

## 验证命令

先在 `web/` 执行格式化，再按顺序运行：

```bash
cd web
pnpm format
pnpm lint
pnpm typecheck
pnpm test
pnpm check:cycles
pnpm build
```

针对实现新增的共享 Drawer 测试及受影响的 AccountPanelDrawer、CameraDetailDrawer、AlgoParamDrawer 测试先做快速迭代；最终仍运行完整 Web 门禁。手动视觉检查至少覆盖亮/暗主题、窄/宽视口、各宽度变体和内容高度超出视口场景。所有可见文本与关闭名称继续通过 i18n，不新增硬编码文案。

## 风险文件与回归点

- `web/src/components/ui/ModalOverlay.tsx`：原则上不改默认 surface 或浮层栈，只由 Drawer 明确传入 solid；如确需调整必须证明不会影响非 drawer 调用方。
- `web/src/components/ui/Drawer.tsx` 与 `web/src/styles/globals.css`：检查尺寸 CSS 层叠、`p-0`、主体 flex/overflow、主题 token 和 reduced-motion 行为。
- 7 个目标 feature 文件：保持状态初始化、网络请求、确认流程、回调次序、国际化和 `onExitComplete` 清理不变。
- 人员详情嵌套预览/确认、算法版本卸载确认、账号改密弹窗：回归浮层 priority、ESC 栈顺序、背景点击与焦点归还。
- 摄像头详情、算法参数、日志详情：回归固定工具栏/底栏不随 body 滚动，主体内容可独立滚动。

## 回退点

- 共享 UI 阶段：若实体材质或尺寸映射有问题，限定修复 `Drawer` 与抽屉 CSS，不改 `ModalOverlay` 的全局默认值。
- feature 迁移阶段：出现单域功能回归时，先回退该抽屉调用点，其他抽屉保留共享组件迁移；恢复该域后再继续。
- 全量验收阶段：若浮层栈、焦点或主题回归无法在共享组件中修复，回退所有 7 个调用点和新增 Drawer 样式/测试；普通 modal 保持原样。

## 工作流门禁

- 用户审阅规划后任务已启动为 `in_progress`；实现阶段使用 `trellis-before-dev`，按 Codex inline 模式跳过 implement/check JSONL 配置，不派发 implement/check 子代理。
- 组件和样式规范已更新，自动化 Web 门禁及差异审查完成。浏览器视觉检查仍待执行，提交未创建；仓库 `AGENTS.md` 要求仅在用户明确要求时创建提交。
