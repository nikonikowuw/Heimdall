# RKNN rknn_dup_context 权重复用与跨线程/跨核亲和性独立探针 (T42)

## 目的

验证在 Rockchip RKNN 平台（RK3588 / RK3576 / RK3568）上使用 `rknn_dup_context` 进行多实例同模型权重复用时的核心假设：
1. **路线 A (首选)**: 控制线程调用 `rknn_dup_context(&root, &child)`，将 child context 所有权独占移交目标实例 Worker 线程；实例 Worker 在其专用线程内设置核心掩码（`rknn_set_core_mask`）、运行推理（`rknn_run`）并在完成时销毁（`rknn_destroy`）。
2. **路线 B (后备)**: 若跨线程使用 child context 出现 TLS/驱动限制，验证实例 Worker 线程加锁调用 `rknn_dup_context` 的可行性。
3. **隔离与物理共享**:
   - 兄弟 child 独立销毁不影响其他并发推理实例与 root context；
   - 验证 VmRSS / RKNN_QUERY_MEM_SIZE 内存共享证据。

## 板端运行指南

```bash
# 1. 编译探针 (板端直接编译，无需外部依赖)
cd tools/probe_rknn_dup_context
make

# 2. 静态自检 (检查 librknnrt.so 是否导出 rknn_dup_context 与 rknn_set_core_mask)
./probe_rknn_dup_context

# 3. 运行实测 (传入任意板端已有 rknn 模型文件，例如 yolov6n 或 face 模型)
./probe_rknn_dup_context -m /path/to/model.rknn -c 3 -n 20 --route A
```
