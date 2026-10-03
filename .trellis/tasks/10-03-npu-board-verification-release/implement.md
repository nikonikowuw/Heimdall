# NPU 多核分配板端性能基线与发布验收 (Execution Plan)

## 0. 执行前置检查 (Pre-Execution Gate)

- [x] 确认物理开发板网络连通与免密登录正常（RK3568 @ 192.168.17.140, RK3576 @ 192.168.18.28）。
- [x] 确认板端 librknnrt 2.3.2 导出 `rknn_dup_context` 符号，内核 RKNPU 驱动节点正常。
- [x] 确认本地代码库无未提交脏代码，所有前序子任务已归档。

---

## 1. 分阶段实施清单 (Implementation Checklist)

### 阶段 1: 路线 A 线程归属与并发判定 (Criteria C1~C4)
- [x] **1.1** 编写板端轻量级验证程序（C/Rust 原生探针 `rknn_route_a_probe`），包含：
  - 根 Context 初始化（`rknn_init`）；
  - 控制线程通过 `rknn_dup_context` 派生子 context；
  - 将子 context 所有权跨线程传递至专属 Worker 线程；
  - Worker 线程内部调用 `rknn_set_core_mask` 并执行推理；
  - 多线程并发压力测试（RK3576 Core0 + Core1 分核并发）；
  - 先子后根安全析构。
- [x] **1.2** 部署并运行于 RK3576（双核）和 RK3568（单核），收集并记录执行结果。
- [x] **1.3** 依照 C1~C4 标准完成选路判定裁决，消解阻塞项 `B-R14`。

### 阶段 2: 真实板端内存增量与共享证据包采集 (AC02, AC03)
- [x] **2.1** 执行对照组测试：在板端独立启动 1、2、3 个独立的 `rknn_init` 实例，采集每次的 CMA 使用量与进程 RSS。
- [x] **2.2** 执行实验组测试：在板端启动 1 个根权重，通过 `rknn_dup_context` 派生 1、2、3 个子实例，采集每次的 CMA 使用量与进程 RSS。
- [x] **2.3** 形成板端内存增量报告，计算物理权重显存节约比例与单实例私有开销。
- [x] **2.4** 在 RK3576 上监控 `/sys/kernel/debug/rknpu/load`，验证分核调度时 Core0 与 Core1 的同时硬件负载。

### 阶段 3: 循环启停长稳与资源泄漏验证 (AC04)
- [x] **3.1** 编写板端长稳自动化脚本，连续执行 100 轮子会话创建、分核绑定、并发推理与销毁。
- [x] **3.2** 监控每次循环前后 `/proc/$PID/fd` 数量、CMA 显存基线与内核 dmesg。
- [x] **3.3** 验证 100 轮后系统无僵尸 context、无 fd 泄漏、CMA 完全释放回初始值。

### 阶段 4: 前端兼容与全工作区门禁检查 (AC05)
- [x] **4.1** 运行前端静态检查：`cd web && pnpm typecheck && pnpm lint && pnpm test && pnpm build`。
- [x] **4.2** 运行后端各平台算法包门禁：
  - `algo-packages/macos`
  - `algo-packages/rknn/rk3568`
  - `algo-packages/rknn/rk3576`
  - `algo-packages/rknn/rk3588`
- [x] **4.3** 运行根 workspace 完整门禁：`cargo fmt --all -- --check`、`cargo clippy`、`cargo nextest run --workspace`。

### 阶段 5: 规范固化、消除 Blocking Facts 与发布收口 (AC06)
- [x] **5.1** 更新 `.trellis/spec/infer/backend/inference-backends.md` 与 `.trellis/spec/algo-sdk/backend/algo-sdk-guidelines.md`，固化板端实测数据与 Profile 配置。
- [x] **5.2** 清除父任务 `10-01-npu-core-allocation/task.json` 中的全部 `blockingFacts`（B-R14, B-TGT, B-CAP, B-THM）。
- [x] **5.3** 整理验收总结，提交 git commit，归档子任务 5 并完成父任务验收。

---

## 2. 关键验证与排查指令 (Verification Commands)

### 板端调试与监控指令
```bash
# 查看 NPU 驱动版本与当前核负载
cat /sys/kernel/debug/rknpu/version
cat /sys/kernel/debug/rknpu/load

# 查看系统与连续物理显存 (CMA) 分配情况
cat /proc/meminfo | grep -i cma

# 查看目标进程的文件描述符占用
ls -la /proc/<PID>/fd | wc -l

# 查看内核日志是否有 RKNPU 报错或超时
dmesg | grep -i -E 'rknpu|npu|rknn' | tail -n 50
```

### 全量门禁校验命令
```bash
# 1. 根工作区代码质量门禁
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo nextest run --workspace

# 2. 各平台算法包门禁
for manifest in \
  algo-packages/macos/Cargo.toml \
  algo-packages/rknn/rk3568/Cargo.toml \
  algo-packages/rknn/rk3576/Cargo.toml \
  algo-packages/rknn/rk3588/Cargo.toml
do
  cargo fmt --manifest-path "$manifest" --all -- --check
  cargo clippy --manifest-path "$manifest" --workspace --all-targets -- -D warnings
  cargo nextest run --manifest-path "$manifest" --workspace
done

# 3. 前端质量门禁
cd web
pnpm typecheck
pnpm lint
pnpm test
pnpm build
```
