# 样式规范

主题与共享样式唯一来源：[globals.css](../../../web/src/styles/globals.css)。不在 spec 复制整套 token 值。

## 主题与材质

- 亮色为 Clean-Room Minimal Industrial，暗色为 Dark Industrial；保留 32px 校准网格与低噪工业界面。
- 色彩使用 CSS 变量或语义 Tailwind 类，不写硬编码色值、`bg-blue-500` 等具名色；不在组件用 `dark:` 重复配色。
- `.dark` 根类切换变量，初始主题及持久化以 [use-theme.ts](../../../web/src/hooks/use-theme.ts) 为准（当前默认 dark）。
- 面板复用 `.frosted-glass` 等共享材质，卡片使用 16～24px 圆角，避免直角；不随意使用 `!important`。
- 生产图标统一 Lucide，零 Emoji；避免霓虹闪烁，图标沿用统一 stroke 风格。
- `cn()` 组合类名，调用方 `className` 放末尾；工具类由格式化工具排序。

## 排印

| token          | 字体与用途                                                |
| -------------- | --------------------------------------------------------- |
| `font-display` | Unbounded，展示标题                                       |
| `font-tech`    | Space Grotesk，技术标题/控件                              |
| `font-sans`    | Inter，正文/表单                                          |
| `font-data`    | Space Mono，FPS/延迟/置信度；配 `tabular-nums` 防宽度抖动 |

字体自托管，使用 `font-display: swap`。文本通过 [i18n](./directory-structure.md#国际化)，布局容纳英文扩展，表单和关键错误不因截断丢失含义。

## 视频与叠加

- 视频容器预留 `aspect-video`（16:9），加载前后不跳动。
- 展示型透明 Canvas 绝对定位贴合视频内容区，设置 `pointer-events: none`；规则编辑层单独处理交互。
- 归一化坐标绘制时乘实际内容区尺寸，考虑留黑边与缩放，不能使用固定屏幕像素定位。
- 规则编辑在动态子码流上完成；数据与播放帧按时间戳对齐，详见 [组件规范](./component-guidelines.md)。
- 自定义光标的交互目标沿用 `.reticle-target`，保留原生键盘焦点和可访问性。

验证双主题、32px 网格、三语溢出、等宽数字、视频多路负载和主题切换无残留硬编码颜色。
