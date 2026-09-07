# 前端质量规范

> 提交前必须通过的检查、测试要求、性能与可访问性底线。

> ⚠️ **状态：立项约定（尚未经代码验证）**
> 工具链配置需在首批代码落地后实测并回填，删除本提示。

---

## 提交前必跑

**硬性规则：每次提交代码前，必须先使用格式化工具自动格式化代码！**

```bash
cd web
# 1. 自动代码格式化（每次提交必跑）
pnpm format        # prettier --write .

# 2. 静态检查、类型检查、单元测试与构建产物验证
pnpm lint          # eslint --max-warnings=0
pnpm typecheck     # tsc --noEmit
pnpm test          # vitest run
pnpm build         # tsc + vite build，验证产物能出
```

**`--max-warnings=0` 是硬要求**。允许积累警告等于没有 lint。

`pnpm build` 也要跑：Vite 开发服务器的宽松度和生产构建不同，只在 dev 下测过的代码可能构建失败。

---

## 工具链

| 工具 | 用途 | 配置要点 |
|------|------|---------|
| Prettier | 自动格式化 | 必须配置 `package.json` 的 `"format": "prettier --write ."`，加 `prettier-plugin-tailwindcss` 自动排序工具类 |
| ESLint | 静态检查 | flat config，必开 `react-hooks/exhaustive-deps` 和 `react-hooks/rules-of-hooks` |
| TypeScript | 类型检查 | `strict: true`，见 [type-safety.md](./type-safety.md) |
| Vitest | 单元/组件测试 | 配 `@testing-library/react` |

**`prettier-plugin-tailwindcss` 是必装的** —— 它自动排序 className，消除"同样的样式两种写法"造成的 diff 噪音。

---

## 必开的 ESLint 规则

```js
// eslint.config.js（要点，非完整配置）
rules: {
  "react-hooks/rules-of-hooks": "error",
  "react-hooks/exhaustive-deps": "error",     // 不是 warn
  "@typescript-eslint/no-explicit-any": "error",
  "@typescript-eslint/no-non-null-assertion": "error",
  "no-console": ["error", { allow: ["warn", "error"] }],
}
```

`exhaustive-deps` 设为 `error` 而非默认的 `warn`：依赖数组不完整是 React 里最高频的 bug 来源，见 [hook-guidelines.md](./hook-guidelines.md)。

---

## 测试要求

| 变更类型 | 测试要求 |
|---------|---------|
| 纯函数（时间格式化、坐标换算、事件去重） | **必须有单元测试** |
| 自定义 hook | 用 `renderHook` 测，重点测清理逻辑和依赖变化 |
| 展示组件 | 测渲染结果和交互回调，用 `@testing-library/react` |
| 容器组件 / 页面 | 可选，成本高收益低，优先测 hook 和纯函数 |
| Bug 修复 | **先写能复现的失败测试** |

**测行为不测实现**：

```tsx
// ❌ 测实现细节，重构就挂
expect(wrapper.find("EventCard")).toHaveLength(3)

// ✅ 测用户能观察到的
expect(screen.getAllByRole("button", { name: /事件/ })).toHaveLength(3)
```

不测：`components/ui/` 下 shadcn 生成的组件（上游已测过）。

---

## 性能检查项（Heimdall 特有）

实时预览页面同时跑 8 路视频 + 持续事件推送，性能问题是功能问题。

提交涉及实时页面的改动前自查：

- [ ] 用 React DevTools Profiler 确认：**推送一条事件时，视频播放器组件没有重渲染**
- [ ] Zustand 选择器返回原始值，或用了 `useShallow`
- [ ] 列表项是 `memo` 的，且传入的回调是稳定引用
- [ ] 事件缓冲有上界
- [ ] 长列表（>200 条）用了虚拟滚动
- [ ] Three.js Canvas 在 60fps 下 GPU 占用 < 5%（低端设备降级到静态帧）
- [ ] Reticle 光标跟随延迟 < 16ms（一帧以内），无明显抖动
- [ ] `backdrop-filter` blur 在 8 路视频同时渲染时无帧率下降
- [ ] 主题切换在 300ms 内完成，无闪烁

见 [component-guidelines.md](./component-guidelines.md#重渲染控制heimdall-特有) 和 [state-management.md](./state-management.md)。

---

## 构建产物约束

前端产物由 Rust 后端提供，可能内嵌进二进制，见 [../backend/api-guidelines.md](../backend/api-guidelines.md)。

- **关注产物体积**。内嵌方案下，前端每大 1 MB 二进制就大 1 MB。
- 构建后检查 chunk 分布，大依赖（Three.js、播放器库、图表库）走动态 import 按需加载。
- **不要为一个函数引入一个大库**（日期格式化尤其常见）。原生 `Intl` 能解决的不引 date 库。
- **字体文件**自托管（Unbounded + Space Grotesk + Inter + Space Mono），走 `font-display: swap` 避免 FOIT，确保首屏文字立即可见。
- **Three.js** ~150KB gzip，考虑动态 import（仅在有拓扑背景的页面加载）。
- **i18n 翻译文件**按语言懒加载，初始包只含当前语言，切换语言时动态加载其它语言。

---

## 视觉回归检查（新增）

提交涉及样式改动的 PR 前自查：

- [ ] 亮色/暗色双主题下校准网格均正常显示（32px 间距，发丝线透明度适配）
- [ ] 所有卡片/面板使用 `frosted-glass` 类，无遗漏的硬编码背景色
- [ ] Reticle 光标在所有可交互元素上正确膨胀（`reticle-target` 类已添加）
- [ ] 等宽数据（延迟、FPS、置信度）使用 `font-data` 类，数字无抖动
- [ ] 无 Emoji 出现在界面中
- [ ] i18n 文本无溢出截断（英文可能比中文长 2-3 倍，容器需弹性布局）
- [ ] 主题切换后无残留的硬编码颜色（检查 DevTools 计算样式）

---

## 可访问性底线

- 可点击元素用 `<button>` / `<a>`，不用 `<div onClick>`
- 图标按钮有 `aria-label`
- 表单控件关联 `<label>`
- 焦点可见（不允许裸 `outline: none`）
- 深色模式下对比度达标（监控界面长时间注视，对比度不足很快就累）

优先用 shadcn/ui 组件 —— 它基于 Radix，a11y 已处理好。

---

## 禁止事项

- ❌ 提交前未执行代码格式化（必须跑 `pnpm format`）
- ❌ 带 lint 警告提交
- ❌ 提交 `console.log`
- ❌ 关闭 `exhaustive-deps`
- ❌ 用 `@ts-ignore` 绕过类型错误
- ❌ 只在 dev 模式下验证就提交（必须跑一次 `build`）
- ❌ 为一个工具函数引入整个库
- ❌ 测试断言组件内部结构

---

## 待验证事项

- [ ] 包管理器确认（本文件假设 pnpm）
- [ ] 是否引入 E2E 测试（Playwright），以及是否需要真实设备环境
- [ ] 产物体积预算的具体数值（取决于静态资源交付方式）
- [ ] 视觉回归测试方案（Chromatic / Percy / 手动截图对比）
