# 执行计划：系统概述模块升级

## 实现顺序

### 阶段 1：后端基础架构（预计 2-3 小时）

#### Step 1.1：创建 NPU 抽象层
- [ ] 创建 `crates/infer/src/npu/mod.rs` - 定义 `NpuDevice` trait
- [ ] 创建 `crates/infer/src/npu/monitor.rs` - `NpuMonitor` 多核心管理器
- [ ] 创建 `crates/infer/src/npu/rknn.rs` - RKNN 平台实现
- [ ] 创建 `crates/infer/src/npu/ascend.rs` - Ascend 平台实现
- [ ] 编写单元测试

**验证**：
```bash
cargo test -p infer --lib npu
```

#### Step 1.2：创建指标采集器
- [ ] 创建 `crates/api/src/metrics/mod.rs` - 统一入口
- [ ] 创建 `crates/api/src/metrics/cpu.rs` - CPU 多核心采集
- [ ] 创建 `crates/api/src/metrics/memory.rs` - 内存详细统计
- [ ] 创建 `crates/api/src/metrics/disk.rs` - 磁盘 + Inode 统计
- [ ] 创建 `crates/api/src/metrics/thermal.rs` - 温度传感器采集
- [ ] 创建 `crates/api/src/metrics/network.rs` - 网络流量统计
- [ ] 编写单元测试

**验证**：
```bash
cargo test -p api --lib metrics
```

#### Step 1.3：扩展数据模型
- [ ] 更新 `crates/types/src/system.rs` - 添加新的数据结构
- [ ] 确保向后兼容（新字段为 Option 类型）

**验证**：
```bash
cargo build -p types
```

### 阶段 2：后端 API 集成（预计 1-2 小时）

#### Step 2.1：更新系统概述端点
- [ ] 修改 `crates/api/src/routes/system.rs` - `get_overview` 函数
- [ ] 集成新的指标采集器
- [ ] 处理权限降级逻辑

**验证**：
```bash
cargo build -p api
curl http://localhost:8080/api/v1/system/overview | jq .
```

#### Step 2.2：添加实时指标端点（可选）
- [ ] 创建 WebSocket 端点 `GET /ws/system/metrics`
- [ ] 实现定时推送逻辑

**验证**：
```bash
# 使用 wscat 测试
wscat -c ws://localhost:8080/ws/system/metrics
```

### 阶段 3：前端组件开发（预计 3-4 小时）

#### Step 3.1：更新类型定义
- [ ] 更新 `web/src/types/system.ts` - 添加新的 TypeScript 接口

**验证**：
```bash
cd web && pnpm typecheck
```

#### Step 3.2：创建 CPU 核心热力图
- [ ] 创建 `web/src/features/system/components/CpuHeatmap.tsx`
- [ ] 实现每核心独立显示
- [ ] 添加颜色编码（绿/黄/红）

**验证**：
```bash
cd web && pnpm typecheck
cd web && pnpm lint
```

#### Step 3.3：创建 NPU 多核心概览
- [ ] 创建 `web/src/features/system/components/NpuOverview.tsx`
- [ ] 实现多核心并排显示
- [ ] 添加负载均衡度指标

**验证**：
```bash
cd web && pnpm typecheck
cd web && pnpm lint
```

#### Step 3.4：创建网络流量曲线
- [ ] 创建 `web/src/features/system/components/NetworkChart.tsx`
- [ ] 实现实时流量曲线（使用 Canvas 或 SVG）
- [ ] 支持多接口切换

**验证**：
```bash
cd web && pnpm typecheck
cd web && pnpm lint
```

#### Step 3.5：创建温度状态显示
- [ ] 创建 `web/src/features/system/components/ThermalStatus.tsx`
- [ ] 实现多传感器温度显示
- [ ] 添加降频预警状态

**验证**：
```bash
cd web && pnpm typecheck
cd web && pnpm lint
```

#### Step 3.6：重构系统概述主页面
- [ ] 重构 `web/src/features/system/SystemOverview.tsx`
- [ ] 集成新的子组件
- [ ] 优化布局和响应式设计

**验证**：
```bash
cd web && pnpm build
```

### 阶段 4：集成测试（预计 1-2 小时）

#### Step 4.1：后端集成测试
- [ ] 运行完整的 API 测试套件
- [ ] 测试不同平台的降级逻辑

**验证**：
```bash
cargo test -p api
```

#### Step 4.2：前端集成测试
- [ ] 运行前端测试套件
- [ ] 手动测试 UI 交互

**验证**：
```bash
cd web && pnpm test
```

#### Step 4.3：端到端测试
- [ ] 启动后端服务
- [ ] 启动前端开发服务器
- [ ] 手动验证所有功能

**验证**：
```bash
# 终端 1：启动后端
cargo run -p api

# 终端 2：启动前端
cd web && pnpm dev

# 浏览器访问 http://localhost:5173/settings
```

## 关键文件清单

### 后端新增文件
```
crates/infer/src/npu/mod.rs
crates/infer/src/npu/monitor.rs
crates/infer/src/npu/rknn.rs
crates/infer/src/npu/ascend.rs
crates/api/src/metrics/mod.rs
crates/api/src/metrics/cpu.rs
crates/api/src/metrics/memory.rs
crates/api/src/metrics/disk.rs
crates/api/src/metrics/thermal.rs
crates/api/src/metrics/network.rs
```

### 后端修改文件
```
crates/types/src/system.rs
crates/api/src/routes/system.rs
crates/infer/src/lib.rs
crates/api/src/lib.rs
```

### 前端新增文件
```
web/src/features/system/components/CpuHeatmap.tsx
web/src/features/system/components/NpuOverview.tsx
web/src/features/system/components/NetworkChart.tsx
web/src/features/system/components/ThermalStatus.tsx
```

### 前端修改文件
```
web/src/types/system.ts
web/src/features/system/SystemOverview.tsx
```

## 验证门禁

### 代码质量
```bash
# 后端
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --workspace

# 前端
cd web
pnpm format
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

### 功能验收清单
- [ ] CPU 多核心热力图正常显示
- [ ] NPU 多核心状态正常显示（RK3588 测试）
- [ ] 网络流量实时更新
- [ ] 温度传感器数据正确
- [ ] 无 NPU 设备时优雅降级
- [ ] 无权限时显示 N/A
- [ ] 移动端布局正常

## 回滚点

### 回滚点 1：后端基础架构完成后
如果 NPU 采集实现有问题，可以：
1. 保留数据模型扩展
2. 回滚 NPU 采集器
3. 使用 mock 数据

### 回滚点 2：前端组件开发完成后
如果 UI 设计有问题，可以：
1. 保留后端 API
2. 回滚前端组件
3. 使用简化布局

## 后续优化（本 Task 范围外）

1. 历史数据存储和趋势分析
2. 告警阈值配置
3. WebSocket 实时推送
4. eMMC/SSD 健康度监控
5. 更多 NPU 平台支持（TensorRT、OpenVINO）
