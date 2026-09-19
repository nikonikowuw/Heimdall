# 前端质量检查

## 门禁

按 [AGENTS.md](../../../AGENTS.md) 执行 Web 门禁：先 `pnpm format`，再 lint、typecheck、test、`check:cycles`、build。
命令以 [package.json](../../../web/package.json) 为准，lint 必须零警告；不能只验证开发服务器。
文档改动检查格式、链接与契约引用，不运行无关产品构建。

- Prettier 配合 `prettier-plugin-tailwindcss` 排序类名；ESLint 保留 hooks/exhaustive-deps、any、非空断言与 console 约束。
- 不提交 `console.log`、`@ts-ignore` 或通过关闭 lint 绕过错误。
- TypeScript 基线见 [类型规范](./type-safety.md#编译与边界)。
- 模块图不得有循环依赖，由 `pnpm check:cycles`（[scripts/check-import-cycles.mjs](../../../web/scripts/check-import-cycles.mjs)）强制；判据与 [目录结构](./directory-structure.md#模块边界) 的层间方向一致。类型层的 `import type` 环同样要清 —— TS 会擦除它，所以 build/lint/test 都发现不了，只有这个检查能抓。

## 测试选择

| 变更                         | 优先验证                                             |
| ---------------------------- | ---------------------------------------------------- |
| 时间、坐标、事件去重等纯逻辑 | 边界值、单位、重复与排序                             |
| 资源 hook / 连接             | 卸载清理、取消、迟到响应、依赖变化和重试             |
| 交互组件                     | 用户可见内容、角色、回调与键盘操作；不依赖内部组件树 |
| 页面                         | 优先测试核心 hook/纯逻辑，补充关键操作验证           |
| Bug                          | 先建立可复现的失败用例                               |

使用 Vitest 与工程已有测试设施；组件/hook 测试按需使用 Testing Library。简单可逆的文案/样式不新增复述实现的测试，生成 UI 不重复上游用例。

## 实时性能

- Profiler 确认无关事件不重渲染/重建播放器，选择器细粒度、对象返回配 `useShallow`。
- 高频数据使用 ref/Canvas，列表项保持稳定；缓冲有界，超过 200 条的长列表虚拟滚动。
- 多路视频下测试 blur/背景效果不会明显掉帧，低端设备可降级静态背景。
- 沿用目标预算：主题切换 300ms 内、自定义光标延迟低于 16ms；若引入 Three.js，验证 60fps 下 GPU 开销目标低于 5%。必须记录测试环境，不能把目标当作性能保证。

## 视觉与可访问性

检查亮/暗主题、校准网格、共享面板材质、光标目标、等宽数字、零 Emoji、三语弹性布局及色彩 token。
控件使用 button/link，图标按钮有 `aria-label`，表单关联 label，焦点可见、对比度可读。
局部媒体视口崩溃不应使整页白屏。

## 构建产物

SPA 由 Rust `rust-embed` 内嵌；检查 chunk 体积，播放器、图表/3D 等大依赖按需加载，不为小功能引入整库。
字体自托管并 `font-display: swap`，翻译按语言/模块懒加载，首屏不打包全部语言。
交付记录实际通过、失败及未运行的检查。

`dist/` 必须留在 [.prettierignore](../../../web/.prettierignore) 里，不得让 `pnpm format` 碰它。

- 格式化压缩产物会把 JS 体积撑到约 1.77 倍，Rust 侧 `static_files.rs` 的 `#[folder = "../../web/dist/"]` 是**编译期**内嵌，于是二进制直接变大，且压缩开关（`rust-embed` 未启用 `compression`）不会救它。
- 更隐蔽的是 `Makefile` 的 `ensure-web-dist` 以 `target/.web-build-stamp` 而非 `dist/index.html` 为判据：以产物文件为判据时，任何外部工具碰一下 `dist/index.html` 就会把它 mtime 推到源码之后，导致 make 误判「已最新」而跳过前端构建 —— 内嵌的要么是未压缩产物，要么干脆缺了最新源码改动，两种都静默无告警。
- 改动 `WEB_SRCS` 覆盖范围（如新增 `vite.config.ts`、`tsconfig*.json`、`pnpm-lock.yaml`）时保持 stamp 判据不变。

### 路由级分包

边界集中在 [lazy-pages.ts](../../../web/src/app/lazy-pages.ts)：

- 8 个工作区页面按需加载；`LoginPage` 保持同步导入 —— 它是未鉴权用户的首次绘制内容，懒加载会在登录表单出现前多加一次 chunk 往返，属懒加载最不该出现的位置。
- 鉴权通过后调用 `preloadDefaultTab()` 预取 `live`，在工作区入场动画（约 500ms）期间完成下载，抵消默认页的懒加载延迟。
- 页面均为具名导出，`lazy()` 需经 `.then` 适配 `default`。
- `Suspense` 占位复用 [RouteFallback.tsx](../../../web/src/components/ui/RouteFallback.tsx) 与 `.route-fallback`：其 200ms 延迟现身避免内网快速加载时的骨架闪烁，进度条样式自带 `prefers-reduced-motion` 降级。
- 新增 chunk 无需改 Rust 侧：`crates/api/src/static_files.rs` 用 `Assets::get(&path)` 按路径取，`assets/` 走 immutable 缓存。
- 跨 feature 共用的大依赖上提到共享层：`LivePlayer`（含 mpegts.js）位于 `components/`，由 `live` 与 `tasks` 共用并作为共享 chunk 输出；若它留在某个 feature 内被另一个 feature 引用，分包归属将不再反映真实依赖关系。

回归基线（首屏 gzip，随实现演进而更新）：登录页约 150KB；已鉴权 `live` 首屏约 231KB（含 mpegts.js）。
