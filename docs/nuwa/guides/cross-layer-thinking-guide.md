# 跨层数据流检查

跨 crate、FFI、数据库、HTTP/WS 或 TypeScript 的改动，先写清：`来源 → 转换 → 存储 → 消费`。
复杂或曾出错的数据流记录在任务 `design.md`。

## 边界契约

| 边界                    | 明确内容                                   | 详细规范                                                                           |
| ----------------------- | ------------------------------------------ | ---------------------------------------------------------------------------------- |
| Rust ↔ C/C++ / 插件     | 布局、版本、所有权、线程、错误码           | [FFI](../backend/ffi-guidelines.md)、[算法 SDK](../backend/algo-sdk-guidelines.md) |
| media ↔ infer           | 帧载体、stride、时间基准、释放责任         | [媒体管线](../backend/media-pipeline.md)                                           |
| infer ↔ pipeline ↔ 告警 | 检测载荷、帧关联、坐标、事件 ID 与证据状态 | [检测与告警契约](../backend/detection-alarm-contract.md)                           |
| pipeline ↔ db           | 领域/entity 映射、幂等、事务与证据路径     | [数据库](../backend/database-guidelines.md)                                        |
| Rust ↔ TypeScript       | 字段、空值、单位、错误信封                 | [API](../backend/api-guidelines.md)、[类型安全](../frontend/type-safety.md)        |

每个边界注明输入、输出、失败行为和校验责任。HTTP、配置、外部消息与 FFI 各自完成入口校验；内部消费者复用已验证类型，不重复猜测 payload。

## 事件与 DTO

- 新增事件时同步共享枚举、entity/migration、DTO 转换、WS 联合类型与前端消费者。
- 共享解码器唯一负责 `unknown` 收窄；渲染层只格式化，不重定义契约。
- 事件 ID 只分配一次；历史分页与实时增量按同一 ID 去重，派生状态沿用源 `id` / `seq`。
- 检查 `Option<T>` / `null`、camelCase、时间戳与枚举穷尽分支（见 [全局约定](./conventions.md)）；新迁移只增不改。

## 坐标

```text
原图 → 缩放/letterbox → 模型输出 → 去 padding / 逆缩放 → 归一化 → 视频内容区
```

- 明确模型输出为像素还是归一化坐标，正逆变换共用布局参数。
- 覆盖非等比分辨率（1920×1080 → 640×640）、贴边框、微小目标和无效输入。
- 三平台输出格式一致；量化误差不能变成坐标协议差异。
- 坐标与时间规范见 [全局约定](./conventions.md)。

## 配置与验证

- 配置字段同步 Rust 默认值、入口校验、运行时应用、API 与表单；旧配置缺少新字段仍可加载。
- 支持热重载的字段必须覆盖重载路径，前后端校验不得互相矛盾。
- 验证空值、非法值、边界值、错误转换和数据往返；搜索全部消费者确认没有旧字段或局部断言。
