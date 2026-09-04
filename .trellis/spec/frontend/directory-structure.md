# 前端目录结构

> Vite + React + TypeScript。按**功能域**组织，不按文件类型组织。
> 支持双主题（亮色/暗色）与 i18n 三语（zh-CN / zh-TW / en）。

> ⚠️ **状态：立项约定（尚未经代码验证）**
> Argus 仓库当前无前端代码。首批页面落地后需回填真实路径与示例，并删除本提示。

---

## 布局

```
web/
├── index.html
├── vite.config.ts
├── tsconfig.json
├── tailwind.config.ts
├── components.json          # shadcn/ui 配置
└── src/
    ├── main.tsx             # 入口，挂载 + 路由
    ├── app/
    │   ├── router.tsx       # 路由表
    │   └── layout.tsx       # 全局布局（含主题/语言初始化）
    ├── i18n/                # 国际化翻译文件（按语言+模块拆分）
    │   ├── index.ts         # i18n 初始化配置
    │   ├── en/              # 英语（基线语言）
    │   │   ├── common.json
    │   │   ├── camera.json
    │   │   ├── alarm.json
    │   │   ├── task.json
    │   │   └── system.json
    │   ├── zh-CN/           # 简体中文
    │   │   └── ...（同 en/）
    │   └── zh-TW/           # 繁体中文
    │       └── ...（同 en/）
    ├── features/            # 按业务功能域划分
    │   ├── auth/            # 登录认证与首次开箱初始化向导 (SetupWizard)
    │   ├── live/            # 实时视频多路预览（Canvas 2D 识别框叠加）
    │   ├── alarms/          # 告警事件检索与证据详情
    │   ├── observations/    # 人脸与车牌通行记录
    │   ├── tasks/           # 摄像头与任务配置（ROI/Mask/Line 交互绘制）
    │   └── system/          # 设备网络、存储配额、系统资源(CPU/NPU/内存)、API Key与重启
    ├── components/
    │   ├── ui/              # shadcn/ui 生成的组件，不手改
    │   ├── background/      # Three.js 拓扑背景等全局装饰
    │   └── cursor/          # Reticle 光标组件
    ├── lib/
    │   ├── api/             # 后端接口调用
    │   ├── ws/              # WebSocket 客户端
    │   ├── time.ts          # 时间格式化（Intl API，统一出口）
    │   └── utils.ts         # cn() 等工具函数
    ├── hooks/               # 全局共享 hooks
    │   ├── use-theme.ts     # 主题切换（light/dark）
    │   ├── use-locale.ts    # 语言切换（zh-CN/zh-TW/en）
    │   └── use-hotkeys.ts   # 键盘快捷键（ESC/Enter/组合键）
    ├── stores/              # 全局 Zustand store
    ├── types/               # 与后端对齐的类型定义
    └── styles/
        └── globals.css      # Tailwind 指令 + CSS 变量 + .frosted-glass 等全局类
```

---

## feature 目录的形状

每个 feature 自包含，内部结构统一：

```
src/features/alarms/
├── index.ts              # 对外出口：只导出页面组件和被别处复用的东西
├── AlarmsPage.tsx        # 页面级组件
├── components/
│   ├── AlarmList.tsx
│   ├── AlarmCard.tsx
│   └── AlarmFilters.tsx
├── hooks/
│   └── useAlarmStream.ts
├── locales/              # feature 级翻译（可选，超过 500 行时拆分）
│   ├── en.json
│   ├── zh-CN.json
│   └── zh-TW.json
├── store.ts              # 该 feature 的局部 Zustand store（如果需要）
└── types.ts              # 只在该 feature 内用的类型
```

规则：

- **feature 之间不互相 import 内部文件**。需要共享就上提到 `components/`、`lib/`、`stores/`，或通过 `index.ts` 显式导出。
- 一个组件只被一个 feature 用 → 放该 feature 的 `components/`。被两个以上 feature 用 → 上提到 `src/components/`。
- **不要预先上提**。先放 feature 里，出现第二个使用方时再移。

---

## 为什么不按类型分目录

```
❌ src/components/  src/hooks/  src/utils/     # 改一个功能要跳四个目录
✅ src/features/alarms/                        # 改告警相关，全在这里
```

`src/components/` 只放**真正跨 feature 复用**的东西和 shadcn/ui 生成物。它不是所有组件的收容所。

---

## 命名约定

| 对象 | 规则 | 示例 |
|------|------|------|
| 组件文件 | 大驼峰，与主具名导出组件同名 | `AlarmCard.tsx` |
| hook 文件 | `use` 前缀小驼峰 | `useAlarmStream.ts` |
| 非组件模块 | 小驼峰 | `formatTimestamp.ts` |
| feature 目录 | 小写领域名词 | `live/`、`alarms/`、`tasks/` |
| 类型文件 | `types.ts` | — |
| 翻译文件 | 模块名小写 | `common.json`、`camera.json` |
| shadcn/ui 组件 | 保持 shadcn 生成的 kebab-case | `components/ui/button.tsx` |

**shadcn/ui 生成的文件保持它自己的命名风格**，不要改成项目风格 —— 那会让后续 `shadcn add` 更新产生冲突。

---

## 路径别名

`tsconfig.json` + `vite.config.ts` 里配 `@/` 指向 `src/`：

```ts
import { Button } from "@/components/ui/button"
import { useAlarmStream } from "@/features/alarms/hooks/useAlarmStream"
```

**规则**：跨目录一律用 `@/`，同目录内用 `./`。禁止 `../../../` 这种相对路径爬升。

---

## i18n 目录结构

翻译文件按语言 + 模块拆分，扁平目录，不嵌套语言文件夹：

```
src/i18n/
├── index.ts              # 初始化：检测语言、加载资源、导出 i18n 实例
├── en/                   # 英语（基线语言，键名来源）
│   ├── common.json       # 通用：按钮、状态、错误信息、导航
│   ├── camera.json       # 摄像头管理、实时预览
│   ├── alarm.json        # 告警事件、通行记录
│   ├── task.json         # 算法任务、ROI/Mask/Line 规则
│   └── system.json       # 系统设置、网络、存储、API Key
├── zh-CN/                # 简体中文
│   ├── common.json
│   ├── camera.json
│   ├── alarm.json
│   ├── task.json
│   └── system.json
└── zh-TW/                # 繁体中文
    ├── common.json
    ├── camera.json
    ├── alarm.json
    ├── task.json
    └── system.json
```

规则：

- **英语（en）是基线语言**：翻译缺失时 fallback 到英语，键名以英语语义命名。
- **按模块拆分**：每个业务域一个 JSON 文件，避免单文件膨胀（超过 500 行难以review）。
- **模块名与 feature 目录名对齐**：`camera.json` ↔ `features/live/` + `features/tasks/`，`alarm.json` ↔ `features/alarms/` + `features/observations/`。
- **加载策略**：`common.json` 全量预加载（启动时），其它模块按路由懒加载（进入对应页面时加载）。
- feature 级翻译放 `src/features/<name>/locales/`（仅当该 feature 有独立于全局的私有翻译键时使用）。
- 翻译键用点分隔领域前缀：`camera.status.online`、`alarm.type.motion`。

i18n 初始化示例（采用 Vite 原生动态 import，支持单二进制静态编译与按需加载）：

```ts
// src/i18n/index.ts
import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'
import resourcesToBackend from 'i18next-resources-to-backend'

i18n
  .use(
    resourcesToBackend((language: string, namespace: string) =>
      import(`./${language}/${namespace}.json`)
    )
  )
  .use(initReactI18next)
  .init({
    fallbackLng: 'en',
    defaultNS: 'common',
    interpolation: { escapeValue: false },
  })

export default i18n
```

组件中使用：

```tsx
// 加载特定模块的翻译
const { t } = useTranslation('camera')
t('status.online')  // → "在线"
```

---

## 禁止事项

- ❌ 将业务私有逻辑随意塞入顶层目录（如将特定 feature 的私有 hook 塞入 `src/hooks/`；顶层 `src/hooks/` 仅允许存放真正的全局共享 hook，如 `useTheme`）
- ❌ feature 之间深层 import 对方内部文件
- ❌ 手改 `components/ui/` 下 shadcn 生成的组件（需要定制就在外面包一层）
- ❌ `../../../` 相对路径
- ❌ 一个组件文件超过 300 行还不拆
- ❌ 组件内硬编码 UI 文本字符串（必须走 i18n 翻译键）

---

## 待验证事项

- [ ] 路由库选型（`react-router` vs `@tanstack/router`）
- [ ] 是否需要 `src/features/playback` 与 `live` 共享播放器组件（可能要上提到 `components/player/`）
- [ ] i18n 库选型（`react-i18next` vs `react-intl`）
- [ ] feature 级翻译是否需要独立于全局翻译懒加载
