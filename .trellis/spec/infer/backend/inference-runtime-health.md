# 推理运行时健康

对闭源算法包，宿主只能观测调用边界，无法证明插件内部的模型语义。运行监测的口径固定为**同步调用是否按期正常返回**；结果交付、载荷有效性与检测语义是三条独立信号链，不得互相推断。

## 实现入口

| 入口                                                    | 职责                                                                                  |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| [inflight.rs](../../../../crates/pipeline/src/inflight.rs) | 在途心跳 `InflightMarker`、RAII 守卫 `InflightGuard`、快照与 `evaluate_inflight` 判定 |
| [pump.rs](../../../../crates/pipeline/src/pump.rs)         | 实例指标口径、调用边界归类、Worker 代际栅栏                                           |
| [worker.rs](../../../../crates/infer/src/worker.rs)        | 单帧硬超时、客户端断路与停机隔离                                                      |
| [package.rs](../../../../crates/infer/src/package.rs)      | `AV_OK` 判定与结果载荷解析                                                            |
| [算法 SDK 规范](../../algo-sdk/backend/algo-sdk-guidelines.md#运行状态边界)  | 插件侧 `instance_process` / 结果发射契约                                              |

## 信号分层

| 信号       | 可观测事实                                                   | 不能推出的结论                                                 |
| ---------- | ------------------------------------------------------------ | -------------------------------------------------------------- |
| 调用边界   | `instance_process` 在时限内返回 `AV_OK`，否则超时/返回错误码 | 模型确实推理、检测语义正确                                     |
| 结果交付   | 同步调用期内是否发出结果回调、发出几条                       | 算法是否在运行（空结果与零回调在数据面同样表现为「没有目标」） |
| 载荷有效性 | JSON 结构、坐标/置信度、UTF-8 与数量上限校验是否通过         | 调用是否失败                                                   |
| 检测语义   | ——                                                           | 宿主不可观测，不得作为监测依据                                 |

- 缺少回调、空结果、坏载荷都不得作为「算法停止运行」的证据；插件按自身结果契约发射结果，宿主不替它合成空载荷。
- 某帧没有目标不代表算法停摆，也不代表算法健康；活性只看调用序列是否推进。

## 指标口径

实例指标定义在 [pump.rs](../../../../crates/pipeline/src/pump.rs)：

| 指标                             | 口径                                                           |
| -------------------------------- | -------------------------------------------------------------- |
| `frames_inferred`                | 通过代际栅栏并消费的推理结果帧数                               |
| `inference_errors`               | 推理执行与结果链路的错误累计数（不含被代际栅栏丢弃的迟到错误） |
| `consecutive_execution_failures` | 同步调用未按期正常返回的连续次数，返回 `AV_OK` 即清零          |
| `stale_results`                  | 因 Worker 代际被替换而丢弃的迟到结果或错误数                   |

- 计入连续执行失败的只有调用边界错误：超时、非 `AV_OK`、执行错误与 ABI 错误。
- 结果载荷解析失败（如 `JsonParse`）发生在 `AV_OK` 返回之后，属于结果链路诊断：计入 `inference_errors`，不递增连续失败数，并按调用成功清零。
- 连续执行失败数只说明「调用未按期成功返回」，不能说明模型未执行，也不能说明检测语义错误。
- 同一实例的失败告警按 [日志规范](../../guides/logging-guidelines.md) 节流：首条即时，其后每 10 秒最多一条并附 `suppressed` 计数；指标仍逐帧累计，降噪不改变口径。

## Worker 代际栅栏

- 实例 Worker 热替换先切换句柄再递增代际：宁可丢弃新 Worker 的首批结果，也不能让旧 Worker 的结果被当成新代际产出。
- `Ok` 与 `Err` 两条分支都必须校验代际；旧代际的迟到结果**和**迟到错误（含停机取消、超时）只计入 `stale_results`，不得增减当前代际的连续失败数与错误累计数，也不得进入跟踪与规则链。
- 推理循环在同步阻塞点前取得 `InflightGuard`，由 `Drop` 退出在途：正常返回、返回错误与 await 点被取消/abort 三条路径都不会把实例留在在途状态；每实例同时只允许一个在途调用（循环串行 await），在途状态不阻塞其他实例。

## 在途心跳

`InflightMarker` 由推理循环独占写入、任意线程无锁读取；`entered_total` 单调递增永不归零，是区分「健康空闲」与「从未进入推理」的唯一锚点。

判定优先级：从未进入 > 在途超时 > 有入无出 > 健康，详见 [inflight.rs](../../../../crates/pipeline/src/inflight.rs)。

| 结论                              | 判据                                                                                                                                        |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `NeverEntered`                    | `entered_total == 0` 且超出 `entry_grace_ms`；只说明没有帧进入过推理，源流未交付被放行的帧与插件初始化挂死同判，须结合源帧到达/解码状态判读 |
| `Stuck`                           | 在途且 `in_flight_ms > stall_timeout_ms`                                                                                                    |
| `EnteredButNeverLeaves`           | `entered - completed > max_outstanding`（防御性判定）                                                                                       |
| `HealthyInFlight` / `HealthyIdle` | 其余                                                                                                                                        |

- 全部时间戳使用进程内单调时钟 `monotonic_now_ms`；禁止挂钟——NTP 回拨会把在途时长算成负值或巨值。
- 阈值默认 `entry_grace_ms=120s`、`stall_timeout_ms=30s`、`max_outstanding=2`；接入巡检时必须按平台冷启动实测校准，调小前先确认首帧预热与模型加载耗时不触发误报。
- 快照经 `PipelineManager::get_instance_inflight_snapshots` 暴露。**周期巡检消费方尚未接线**：当前只有采集与判定能力，不能声称已具备自动告警的活性看门狗。

## 验证

- 指标分类：`pump::tests::execution_failure_streak_excludes_output_errors_and_resets_on_success`。
- 代际栅栏：`test_replaced_worker_stale_result_is_discarded`、`test_stale_worker_error_does_not_taint_execution_failure_streak`、`test_generation_fence_keeps_failure_streak_across_worker_replacement`（[pump_incremental_instance_tests.rs](../../../../crates/pipeline/tests/pump_incremental_instance_tests.rs)）。
- 心跳三态、时钟回拨与取消安全：[inflight.rs](../../../../crates/pipeline/src/inflight.rs) 模块测试（含 `dropping_guard_exits_in_flight_even_when_call_is_abandoned`）。
- 失败告警节流：`pump::tests::warn_throttle_limits_frequency`。
- 新增插件/平台时验证：坏载荷不递增连续执行失败数；旧代际的迟到结果与迟到错误都被隔离。
