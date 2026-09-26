# 类型与时间

类型入口：[types/index.ts](../../../web/src/types/index.ts)、[types/system.ts](../../../web/src/types/system.ts)。协议以 [API 规范](../backend/api-guidelines.md) 为准。

## 编译与边界

- 类型基线：`strict`、`noUncheckedIndexedAccess`、`noUnusedLocals`、`noUnusedParameters`、`noFallthroughCasesInSwitch`、`verbatimModuleSyntax`；不因编码方便关闭检查。
- 未知输入用 `unknown`，在网络/配置边界通过共享类型守卫收窄，禁止 `any`、非空断言 `!` 和 UI 内临时 DTO 断言。
- 必要的输入类型断言收敛在 API 边界并注明来源；`as const` 等字面量收窄不替代运行时校验。
- 不使用 `@ts-ignore`；临时 `@ts-expect-error` 必须关联明确缺陷，不作为常规错误处理。

## DTO

| 内容      | 约定                                                    |
| --------- | ------------------------------------------------------- |
| 字段      | 与 Rust serde 的 camelCase 名称逐字对齐                 |
| 绝对时间  | `number`，UTC Unix 毫秒；不声明为 `Date/string`（见 [全局约定](../guides/conventions.md#时间)） |
| 可空字段  | Rust `Option<T>` 对应 `T \| null`，不擅自改为 undefined |
| 几何坐标  | `[0, 1]` 浮点（见 [全局约定](../guides/conventions.md#坐标)），边界校验后交给渲染层 |
| 信封/分页 | 导入共享类型，按具体端点形状解包，不重复定义            |

共享 DTO 放 `types/`，feature 私有类型放本域，Props 放组件附近。
改字段同步 Rust DTO、共享 TS 类型、解码器和全部消费者；编译器不能自动发现后端单侧改名。

**声明即校验，不要写未被消费的断言别名**：类似 `type X = Assert<A, B>` 的写法，若 `X` 从未被当作值或泛型约束消费，它不产生任何约束——是恒为真的空断言，还会让人误以为已有保护。把检查放在声明处（如 `defineFilters` 的实参位置），或让类型成为某个值的类型。

## 联合类型

WS 消息用 `type` 判别联合，异步状态用互斥的 `idle/loading/success/error` 变体；新增变体时必须暴露遗漏分支。

```ts
// switch 已处理所有联合变体后的 default 分支
const exhaustive: never = message;
return exhaustive;
```

类型守卫、归一化、状态 reducer 和元数据投影由数据所有者维护，组件不另建 payload 契约。

### 边界收窄必须走共享工具

原生控件、WS payload 与配置解析拿到的都是宽泛 `string`，还原字面量联合必须做运行时校验，用 [lib/unionNarrowing.ts](../../../web/src/lib/unionNarrowing.ts) 的 `isMemberOf` / `narrowSelectValue`：

- **不要写 `as SomeUnion`**。断言在选项表与类型不同步时会静默放行非法值；校验只会拒绝，差异在编译期和运行期都能暴露。
- `narrowSelectValue` 用于选项数组即唯一真源的场景（接入 [SelectField](../../../web/src/components/ui/SelectField.tsx) 时由组件内部调用）；`isMemberOf` 用于取值域已在别处声明的校验，如 WS 载荷归一化。
- **选项表与联合类型双向锁定**：仅用 `satisfies readonly ['all', ...T[]]` 只能拦住越界值，某个枚举成员从常量中被意外删除时它毫无异议 —— 而这正是下拉会静默丢选项的原因。用 [filters.ts](../../../web/src/features/alarms/filters.ts) 的 `defineFilters<TUnion>()([...])` 声明取值表：它内部用 `Exactly`（两个方向的 `Exclude` 都要求 `never`）把漏档与越界都变成**声明处**的编译错误。
- 同一维度的取值域不得合并共享：语义不同但借住在同一 `string` state 的筛选项（如告警与识别的处理状态）会迫使调用点回写 `as` 断言，应拆成各自类型化的 state。

## 时间显示

统一使用 [lib/time.ts](../../../web/src/lib/time.ts) 的 `Intl.DateTimeFormat` / `Intl.RelativeTimeFormat`。

- 比较和差值按 UTC 毫秒计算，只在展示时构造 Date；不手工切片日期字符串，不为格式化新增日期库。
- 使用用户语言和浏览器/显式时区，统一 24 小时制；关键设置/HUD 标明当前时区。
- 相对时间：小于 1 分钟显示刚刚，1～59 分钟/1～23 小时显示相对值，24 小时起显示绝对日期时间。
- 语言与时区分别处理：当前 formatter 使用浏览器时区，语言切换不会自动变成 Shanghai/Taipei/UTC；不能沿用旧的语言推断时区草案。
- 检测框匹配视频帧时使用原始时间基准，不做本地化换算；CSV/Excel 导出显示时间使用 ISO 8601。

验证空值/非法输入、漏处理联合变体、坐标边界和时间阈值；不以局部断言绕过共享契约。
