# 前端目录结构规范 (Directory Structure)

> Vite + React 18/19 + TypeScript。
> 核心原则：**按业务功能域 (Feature-First) 组织，严禁按文件类型扁平堆砌**。

---

## 1. 顶层工程布局

```text
web/
├── index.html
├── vite.config.ts
├── tsconfig.json
├── tailwind.config.ts
├── components.json          # shadcn/ui 配置
└── src/
    ├── main.tsx             # 入口：挂载全局 ErrorBoundary 与路由
    ├── app/                 # 路由表 (router.tsx) 与全局布局骨架 (layout.tsx)
    ├── i18n/                # 国际化语言包（zh-CN, zh-TW, en）
    ├── features/            # 核心业务功能域（自治分形结构）
    │   ├── auth/            # 登录与首次开箱初始化向导 (SetupWizard)
    │   ├── live/            # 实时视频多路预览（Canvas 2D 识别框叠加）
    │   ├── alarms/          # 告警事件检索与证据详情抽屉
    │   ├── observations/    # 人脸比对与车牌通行记录
    │   ├── tasks/           # 分析任务与 ROI/Mask 规则配置
    │   └── system/          # 设备网络、存储状态、API Key、算力概览
    ├── components/          # 真正跨 feature 复用的纯 UI 控件
    │   ├── ui/              # shadcn/ui CLI 生成组件（严禁手改源码）
    │   └── ErrorBoundary.tsx # 全局与局部渲染崩溃隔离面板
    ├── lib/                 # 基础设施客户端与纯工具函数
    │   ├── api/             # apiClient 统一请求拦截、信封解包、401 登出
    │   ├── ws/              # WebSocket 客户端（指数退避自动重连）
    │   └── time.ts          # 时间格式化统一出口 (Intl API)
    ├── hooks/               # 全局共享 hooks（useTheme, useLocale, useHotkeys）
    ├── stores/              # 全局轻量 Zustand store（auth, ui）
    ├── types/               # 全局领域类型与后端 DTO (api.ts)
    └── styles/              # globals.css（CSS 变量、字体声明、.frosted-glass）
```

---

## 2. Feature 目录自治规范

每个业务域自包含，内部结构统一，外部仅通过 `index.ts` 暴露受控出口：

```text
src/features/alarms/
├── index.ts              # 统一对外出口：仅导出页面组件与被外部复用的类型
├── AlarmsPage.tsx        # 页面级容器组件
├── components/           # 本 feature 专属展示组件 (AlarmList, AlarmCard)
├── hooks/                # 本 feature 专属数据请求 hooks (useAlarmStream)
├── store.ts              # 本 feature 局部 Zustand store（按需）
└── types.ts              # 仅在本 feature 内部流转的专属类型
```

- **Feature 边界隔离铁律**：严禁在 `features/A` 中直接深层 import `features/B/components/...` 的内部未导出文件；
- **上提规则**：一个组件仅单 feature 使用时严格收敛在自身 `components/`，仅当被两个及以上 feature 真实消费时方可上提至顶层 `src/components/`。

---

## 3. 命名与别名约定

| 目标 | 命名风格 | 示例 | 约束 |
|------|---------|------|------|
| **组件文件** | 大驼峰 PascalCase | `AlarmCard.tsx` | 与组件主具名导出同名 |
| **Hook 文件** | 小驼峰 camelCase | `useAlarmStream.ts` | 必须以 `use` 为前缀 |
| **纯工具/非组件** | 小驼峰 camelCase | `formatTime.ts` | 纯函数模块 |
| **Feature 目录** | 小写复数领域名词 | `alarms/`, `tasks/` | 保持语义对齐 |
| **shadcn 组件** | 短横线 kebab-case | `components/ui/button.tsx` | 保持 CLI 原样，严禁手工修改 |

- **路径别名**：跨目录引用一律使用 `@/` 别名（指向 `src/`），同 feature 内部子文件使用 `./`，**严禁出现 `../../../` 相对路径深层爬升**。

---

## 4. 国际化 (i18n) 目录规范

- **三语结构**：基线英文 `en/`、简体中文 `zh-CN/`、繁体中文 `zh-TW/`；
- **按模块拆分**：按领域拆分为 `common.json`, `camera.json`, `alarm.json`, `task.json`, `system.json`，禁止将数千行翻译塞入单一巨型 JSON；
- **回退机制**：英语为基线兜底，缺失键自动 fallback 至英文，翻译键采用点号分段（`alarm.type.motion`）。

---

## 5. 禁止事项 (Iron Rules)

- ❌ 将业务私有 Hook 塞入顶层 `src/hooks/`（顶层只允许通用基础 hooks）
- ❌ 跨 Feature 互相穿透私有目录深层 import
- ❌ 手改 `src/components/ui/` 中 shadcn 生成的源码
- ❌ 出现 `../../../` 形式的深层相对路径爬升
- ❌ 在组件中直接硬编码中文文本（必须接入 i18n 翻译键）
