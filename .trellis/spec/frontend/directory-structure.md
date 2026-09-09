# 前端目录与 i18n

按 feature 组织业务；共享设施只承载真实跨 feature 需求。

## 文件归属

| 路径（相对 `web/src/`） | 内容                                                      |
| ----------------------- | --------------------------------------------------------- |
| `app/`、`main.tsx`      | 应用入口、路由与全局布局                                  |
| `features/<domain>/`    | 页面、私有 `components/`、`hooks/`、类型及必要 UI store   |
| `components/`           | 跨 feature UI；若引入 shadcn，生成组件放 `components/ui/` |
| `lib/`                  | API、媒体协议、时间与纯工具                               |
| `hooks/`                | 共享基础 hook，如 `use-theme.ts`                          |
| `stores/`               | 客户端全局状态，不存普通服务端资源                        |
| `types/`                | 共享 DTO/领域类型；当前公共入口为 `index.ts`              |
| `i18n/<locale>/`        | 按业务模块拆分的翻译 JSON                                 |
| `styles/globals.css`    | 主题 token、字体与共享样式                                |

当前请求入口是 [lib/api.ts](../../../web/src/lib/api.ts)，共享类型见 [types/index.ts](../../../web/src/types/index.ts)；不照搬旧 `lib/api/`、`types/api.ts` 示例新建目录。

## 模块边界

- feature 对外通过受控出口（如 `index.ts`）暴露页面/共享类型，不深层导入其他 feature 私有组件。
- 单 feature 组件和 hook 留在本域；两个以上 feature 实际共用时再上提。
- 组件文件与具名导出采用 PascalCase，hook 使用 `use` 前缀，普通工具用 camelCase；已有文件命名不为统一风格重命名。
- 跨目录使用工程已配置的 `@/` 别名，同域使用 `./`；避免新增多级 `../../../` 引用。
- shadcn 生成源码通过包装定制，不直接手改。

## 国际化

配置以 [i18n/index.ts](../../../web/src/i18n/index.ts) 为准：

- 支持 `zh-CN` / `zh-TW` / `en`，默认 `zh-CN`，缺失翻译 fallback 到 `en`。
- 当前 namespace：`common`、`camera`、`alarm`、`task`、`oplog`、`auth`、`algo`、`system`；按语言/模块懒加载，避免巨型翻译文件。
- 所有可见文本接入 `t()`，键按领域点分隔，例如 `camera.status.online`；新增键同步三语。
- 动态文本使用现有 i18next 插值/复数机制，不拼接翻译句子；当前未安装 ICU 插件，不假定支持 ICU MessageFormat。
- 容器能容纳英文较中文长 2～3 倍的内容；时间格式化见 [类型与时间](./type-safety.md#时间显示)。
