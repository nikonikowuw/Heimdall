# 前端开发规范

> Argus Web UI：Vite + React + TypeScript + Zustand + Tailwind CSS v4 + Three.js + shadcn/ui + react-i18next。
> 双主题：亮色 Clean-Room Minimal Industrial + 暗色 Dark Industrial。
> 语言：简体中文 / 繁体中文 / English。
> 纯客户端 SPA，构建产物由 Rust 后端提供。

> ⚠️ **状态：立项约定（尚未经代码验证）**
> 本目录全部文件基于立项技术栈编写，尚未经真实代码验证。每份文件末尾列有「待验证事项」，
> 首批代码落地后需逐项确认并回填真实文件路径与示例，同时删除各文件顶部的状态提示。

---

## 先读这个

写前端代码前，先读 [../guides/architecture-overview.md](../guides/architecture-overview.md) 了解系统全貌，
以及 [../backend/api-guidelines.md](../backend/api-guidelines.md) 了解接口契约。

---

## 规范索引

| 规范 | 什么时候读 |
|------|-----------|
| [目录结构](./directory-structure.md) | 新建文件，不确定放哪（含 i18n 目录结构） |
| [组件规范](./component-guidelines.md) | 写组件、处理重渲染、用 shadcn/ui、frosted-glass 组件 |
| [样式规范](./styling-guidelines.md) | 写样式、加设计 token、双主题 CSS 变量、Reticle 光标 |
| [Hook 规范](./hook-guidelines.md) | 抽 hook、处理订阅与清理、useTheme、useLocale |
| [状态管理](./state-management.md) | 加 store、接 WebSocket、获取服务端数据 |
| [错误处理与容灾](./error-handling.md) | 处理异常、API 统一拦截、401 登出、ErrorBoundary、媒体流自愈 |
| [类型安全](./type-safety.md) | 定义类型、对接后端 DTO |
| [质量规范](./quality-guidelines.md) | 提交前、写测试、做 code review、视觉回归检查 |

---

## 界面构成

| 页面 / 模块 | 核心功能与挑战 |
|------------|---------------|
| 实时预览监控 | 多路视频并发拉流渲染，Canvas 2D 离屏绘制检测框，告警实时弹窗推送 |
| 告警与通行中心 | 持续增长的告警记录、人脸通行、车牌识别检索，大图特写联动 |
| 算法任务编排 | 摄像头管理、算法参数配置、检测规则交互绘制（ROI 区域 / Mask 遮罩 / Line 分界线） |
| 系统与设备维护 | 设备网络与存储配额配置、系统资源监控 (CPU/NPU/内存/温度)、管理员改密、对外 API Key 管理、重启升级 |

---

## Argus 前端的四条铁律

1. **订阅粒度要细** —— Zustand 选择器返回原始值。订阅整个 store 会让一条事件推送触发全页重渲染。见 [状态管理](./state-management.md)。
2. **播放器要隔离** —— 视频组件必须 `memo` 且 props 稳定。重渲染可能导致播放中断。见 [组件规范](./component-guidelines.md)。
3. **缓冲要有上界** —— 实时事件写入环形缓冲。看板会连开几天，无上界数组必然内存增长。
4. **服务端数据不进 Zustand** —— Zustand 只管客户端状态。见 [状态管理](./state-management.md)。

---

## 设计原则

| 原则 | 亮色主题 | 暗色主题 |
|------|---------|---------|
| 基调 | 高奢极简香槟瓷白（Clean-Room Minimal Industrial） | 深邃深空精密控制台（Dark Industrial） |
| 背景 | #FBFBFC + 32px 校准网格 | #030408 + 32px 校准网格 |
| 强调色 | #0052FF 克莱因蓝 / 玫红微强调 | #3B82F6 提亮蓝 / 靛青微光晕 |
| 材质 | 高透磨砂玻璃（70% 透明） | 深色磨砂玻璃（75% 透明） |
| 文字 | #0B0F19 曜石深黑 | #F8FAFC 浅灰白 |
| 字体体系 | Unbounded (展示) + Space Grotesk (几何) + Inter (正文) + Space Mono (等宽数据) |
| 共用 | 微圆角 12-20px、Reticle 光标、Lucide 图标（stroke-width 1.5）、零 Emoji |

**严禁**：失控杂乱的霓虹乱闪、硬编码颜色值、Emoji 表情符号。

详见 [样式规范](./styling-guidelines.md)。

---

## i18n 约定

| 约定 | 值 |
|------|-----|
| 支持语言 | zh-CN（简体中文）、zh-TW（繁体中文）、en（English） |
| 默认语言 | zh-CN |
| 基线语言 | en（英语是键名来源，翻译缺失时 fallback 到英语） |
| 模块拆分 | common / camera / alarm / task / system，按路由懒加载 |
| 翻译键命名 | 点分隔领域前缀：`camera.status.online`、`alarm.type.motion` |
| 文本容器 | 所有 UI 文本必须包裹在 i18n 函数中，禁止硬编码字符串 |
| 动态插值 | 使用 ICU MessageFormat（复数、性别等） |
| 文本溢出 | 英文可能比中文长 2-3 倍，容器必须支持弹性布局 |
| 加载策略 | common 全量预加载，其它模块按路由懒加载 |

翻译文件结构：`src/i18n/{lang}/{module}.json`，详见 [目录结构](./directory-structure.md)。

---

## 与后端的契约

| 约定 | 值 |
|------|-----|
| API 前缀 | `/api/v1` |
| 字段命名 | camelCase |
| 时间戳 | 全链路 **13 位 UTC Unix 毫秒整数**；信封与绝对时间点统一为 `timestamp`，相对时长保留 `Ms` 后缀（如 `timeoutMs`） |
| 响应格式 | `{ "code": 0, "message": "success", "data": T, "timestamp": ms }` |
| 错误码 | 5 位数字，`0` 成功，非 0 按模块划分 |
| 分页 | 流式大表走游标分页（`before` + `limit`）；配置资源走传统分页（`page` + `pageSize`） |
| 实时数据 | WebSocket 推送，**不轮询** |

契约的权威定义在 [../backend/api-guidelines.md](../backend/api-guidelines.md)。前端类型见 [类型安全](./type-safety.md)。

---

## 时间与时区

### 核心原则

- **后端只存 UTC**：所有时间戳都是 13 位 UTC Unix 毫秒整数，不存储也不传输时区信息。
- **前端按用户时区显示**：根据用户浏览器的 `Intl.DateTimeFormat` 或显式选择的时区进行本地化显示。
- **不使用 `Date` 对象做格式化以外的计算**：时间比较、差值计算全部基于 UTC 毫秒纯数值运算。

### 各语言的时间显示规则

| 语言 | 日期格式 | 12/24 小时制 | 分隔符 | 示例 |
|------|---------|-------------|--------|------|
| zh-CN | YYYY年MM月DD日 HH:mm | 24 小时制 | 月/日用中文，时分用冒号 | 2026年09月03日 14:30 |
| zh-TW | YYYY年MM月DD日 HH:mm | 24 小时制 | 同 zh-CN | 2026年09月03日 14:30 |
| en | MM/DD/YYYY HH:mm | 24 小时制 | 月/日/年用斜杠 | 09/03/2026 14:30 |

### 相对时间显示规则

短时间内显示相对时间，超过阈值切换为绝对时间：

| 时间差 | 显示 | 示例 |
|--------|------|------|
| < 1 分钟 | 「刚刚」/ "just now" | 刚刚 |
| 1-59 分钟 | 「X 分钟前」/ "X min ago" | 3 分钟前 |
| 1-23 小时 | 「X 小时前」/ "X hr ago" | 2 小时前 |
| ≥ 24 小时 | 绝对日期时间 | 09/03/2026 14:30 |

### 时区选择

| 场景 | 行为 |
|------|------|
| 首次访问 | 自动检测 `navigator.language`，推断语言 + 时区 |
| 用户手动切换语言 | 时区跟随切换（zh-CN → Asia/Shanghai, zh-TW → Asia/Taipei, en → UTC 或浏览器时区） |
| 用户手动设置时区 | 覆盖自动检测，持久化到 localStorage |

### 实现约定

```ts
// src/lib/time.ts
/** UTC 毫秒 → 本地化日期时间字符串 */
export function formatTimestamp(ms: number, locale: string): string {
  return new Intl.DateTimeFormat(locale, {
    year: 'numeric', month: '2-digit', day: '2-digit',
    hour: '2-digit', minute: '2-digit',
    hour12: false,  // 统一采用 24 小时制
  }).format(new Date(ms))
}

/** UTC 毫秒 → 相对时间（"3 分钟前"） */
export function formatRelativeTime(ms: number, locale: string): string {
  const diff = Date.now() - ms
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' })
  // 按阈值选择单位，return rtf.format(-value, unit)
  // ...
}
```

规则：

- **使用 `Intl.DateTimeFormat` 和 `Intl.RelativeTimeFormat`**，不引入 date-fns / dayjs 等库。
- **显示时必须标注时区**：在系统设置页、HUD 信息栏等关键位置显示当前时区缩写（如 `CST`、`UTC+8`）。
- **视频 PTS 时间戳的特殊处理**：实时预览页的检测框时间戳直接用 UTC 毫秒渲染，不做本地化转换（保证与视频帧严格对齐）。
- **日志/导出场景**：导出 CSV/Excel 时时间戳转为 ISO 8601 格式（`2026-09-03T14:30:00Z`），便于外部工具处理。

---

## Pre-Development Checklist

写任何前端代码之前，自查以下清单：

- [ ] 状态归属明确：客户端 UI 状态进 Zustand，服务端数据走专用 API hook，不混放
- [ ] 视频播放器组件已用 `React.memo` 隔离，props 保持稳定引用
- [ ] 高频实时检测框（Bounding Box）使用 Canvas 2D 离屏绘制，严禁直接驱动 React DOM 重排
- [ ] 样式走双主题 CSS 变量（`var(--xxx)`），禁止硬编码颜色
- [ ] 所有用户界面文本已包裹进 i18n 国际化函数，无裸字符串

---

## Quality Check

提交前端代码前，必须通过以下检查：

- [ ] **执行代码自动格式化**：`cd web && pnpm format`（Prettier 自动格式化并排序 Tailwind 类名）
- [ ] `pnpm lint`（`eslint --max-warnings=0`）通过
- [ ] `pnpm typecheck`（`tsc --noEmit`）无类型错误
- [ ] `pnpm test` 单元测试通过
- [ ] `pnpm build` 生产构建成功出包
- [ ] 实时事件推送时，视频播放器与非相关面板未发生级联重渲染（通过 DevTools Profiler 校验）
