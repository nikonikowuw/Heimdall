# 技术设计：系统概述模块升级

## 1. 架构设计

### 1.1 分层架构

```
┌─────────────────────────────────────────────────────────────┐
│                    Frontend (React)                         │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐          │
│  │ CpuOverview │ │ NpuOverview │ │NetworkOverview│         │
│  └─────────────┘ └─────────────┘ └─────────────┘          │
└─────────────────────────────────────────────────────────────┘
                           │ HTTP/WebSocket
                           ▼
┌─────────────────────────────────────────────────────────────┐
│                    API Layer (Axum)                         │
│  GET /api/v1/system/overview (enhanced)                    │
│  GET /api/v1/system/metrics/realtime (new)                 │
└─────────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│                 Metrics Collector Layer                     │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐          │
│  │ CpuCollector│ │NpuCollector │ │NetworkCollector│        │
│  └─────────────┘ └─────────────┘ └─────────────┘          │
└─────────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│              Platform Abstraction Layer                     │
│  ┌─────────────────────┐ ┌─────────────────────┐          │
│  │  LinuxCollector     │ │  MacosCollector      │          │
│  │  (RKNP/Ascend)      │ │  (No NPU)            │          │
│  └─────────────────────┘ └─────────────────────┘          │
└─────────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│                    OS / Hardware                            │
│  /proc/stat  /sys/class/devfreq  /sys/class/thermal        │
└─────────────────────────────────────────────────────────────┘
```

### 1.2 目录结构

```
crates/api/src/
├── system_info.rs          # 现有，保持兼容
├── metrics/
│   ├── mod.rs              # MetricsCollector 统一入口
│   ├── cpu.rs              # CPU 多核心采集
│   ├── memory.rs           # 内存详细统计
│   ├── disk.rs             # 磁盘 + Inode 统计
│   ├── thermal.rs          # 温度传感器采集
│   └── network.rs          # 网络流量统计
│
crates/infer/src/
├── npu/
│   ├── mod.rs              # NpuDevice trait 定义
│   ├── monitor.rs          # NpuMonitor 多核心管理
│   ├── rknn.rs             # RKNN 平台实现
│   └── ascend.rs           # Ascend 平台实现

web/src/features/system/
├── SystemOverview.tsx      # 现有，重构
├── components/
│   ├── CpuHeatmap.tsx      # CPU 核心热力图
│   ├── NpuOverview.tsx     # NPU 多核心概览
│   ├── NetworkChart.tsx    # 网络流量曲线
│   └── ThermalStatus.tsx   # 温度状态显示
```

## 2. 数据模型设计

### 2.1 后端数据结构

```rust
// crates/types/src/system.rs (扩展)

/// 系统概述（增强版）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemOverview {
    // === 保留现有字段 ===
    pub software_version: String,
    pub device_model: String,
    pub os_info: String,
    pub kernel_version: String,
    pub uptime_seconds: u64,
    pub active_cameras: u32,
    pub total_cameras: u32,
    pub active_tasks: u32,
    pub today_alarms: u32,
    pub today_captures: u32,
    
    // === 新增：CPU 细粒度 ===
    pub cpu: CpuMetrics,
    
    // === 新增：内存详细 ===
    pub memory: MemoryMetrics,
    
    // === 新增：NPU 多核心 ===
    pub npu: Option<NpuMetrics>,
    
    // === 新增：网络接口 ===
    pub network: Vec<NetworkInterfaceMetrics>,
    
    // === 新增：温度状态 ===
    pub thermal: ThermalMetrics,
    
    // === 新增：磁盘详细 ===
    pub disk: DiskMetrics,
}

/// CPU 指标
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuMetrics {
    pub overall_percent: f64,
    pub per_core: Vec<CoreMetrics>,
    pub temperature: Option<f32>,
    pub frequency_mhz: Option<u32>,
    pub top_processes: Vec<ProcessMetrics>,
}

/// 单个 CPU 核心
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreMetrics {
    pub core_id: u32,
    pub usage_percent: f64,
    pub frequency_mhz: Option<u32>,
    pub temperature: Option<f32>,
}

/// NPU 指标（多核心）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NpuMetrics {
    pub device_type: String,  // "rk3588-npu" | "ascend-310b"
    pub cores: Vec<NpuCoreMetrics>,
    pub total_memory_mb: u64,
    pub used_memory_mb: u64,
    pub temperature: Option<f32>,
    pub active_sessions: u32,
}

/// 单个 NPU 核心
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NpuCoreMetrics {
    pub core_id: u32,
    pub utilization_percent: f64,
    pub frequency_mhz: u32,
    pub power_watts: Option<f32>,
}

/// 内存详细指标
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMetrics {
    pub total_mb: u64,
    pub used_mb: u64,
    pub available_mb: u64,
    pub cached_mb: u64,
    pub buffer_mb: u64,
    pub swap_total_mb: u64,
    pub swap_used_mb: u64,
}

/// 网络接口指标
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterfaceMetrics {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
    pub rx_dropped: u64,
    pub tx_dropped: u64,
    pub speed_mbps: Option<u32>,
    pub link_up: bool,
}

/// 温度指标
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThermalMetrics {
    pub zones: Vec<ThermalZone>,
    pub throttle_active: bool,
}

/// 单个温度区域
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThermalZone {
    pub name: String,
    pub temperature: f32,
    pub type_label: String,  // "cpu" | "npu" | "ddr" | "board"
}

/// 磁盘详细指标
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskMetrics {
    pub total_gb: f64,
    pub used_gb: f64,
    pub available_gb: f64,
    pub inode_total: u64,
    pub inode_used: u64,
    pub inode_available: u64,
}

/// 进程指标
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessMetrics {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f64,
    pub memory_mb: u64,
}
```

### 2.2 前端类型定义

```typescript
// web/src/types/system.ts (扩展)

export interface SystemOverview {
  // 保留现有字段...
  
  cpu: CpuMetrics;
  memory: MemoryMetrics;
  npu: NpuMetrics | null;
  network: NetworkInterfaceMetrics[];
  thermal: ThermalMetrics;
  disk: DiskMetrics;
}

export interface CpuMetrics {
  overallPercent: number;
  perCore: CoreMetrics[];
  temperature: number | null;
  frequencyMhz: number | null;
  topProcesses: ProcessMetrics[];
}

export interface CoreMetrics {
  coreId: number;
  usagePercent: number;
  frequencyMhz: number | null;
  temperature: number | null;
}

export interface NpuMetrics {
  deviceType: string;
  cores: NpuCoreMetrics[];
  totalMemoryMb: number;
  usedMemoryMb: number;
  temperature: number | null;
  activeSessions: number;
}

export interface NpuCoreMetrics {
  coreId: number;
  utilizationPercent: number;
  frequencyMhz: number;
  powerWatts: number | null;
}

// ... 其他类型
```

## 3. API 设计

### 3.1 现有端点增强

```
GET /api/v1/system/overview
```

**响应（增强版）**：
```json
{
  "code": 0,
  "message": "success",
  "data": {
    "softwareVersion": "0.1.0",
    "deviceModel": "RK3588 EVB Board",
    "cpu": {
      "overallPercent": 45.2,
      "perCore": [
        { "coreId": 0, "usagePercent": 62.3, "frequencyMhz": 1800 },
        { "coreId": 1, "usagePercent": 23.1, "frequencyMhz": 1200 },
        { "coreId": 2, "usagePercent": 78.5, "frequencyMhz": 2000 },
        { "coreId": 3, "usagePercent": 17.0, "frequencyMhz": 1000 }
      ],
      "temperature": 62.5,
      "topProcesses": [
        { "pid": 1234, "name": "rknn_infer", "cpuPercent": 45.2, "memoryMb": 256 }
      ]
    },
    "npu": {
      "deviceType": "rk3588-npu",
      "cores": [
        { "coreId": 0, "utilizationPercent": 82.3, "frequencyMhz": 1000 },
        { "coreId": 1, "utilizationPercent": 45.6, "frequencyMhz": 800 },
        { "coreId": 2, "utilizationPercent": 0.0, "frequencyMhz": 500 }
      ],
      "totalMemoryMb": 2048,
      "usedMemoryMb": 1024,
      "temperature": 72.1,
      "activeSessions": 2
    },
    "network": [
      {
        "name": "eth0",
        "rxBytes": 1234567890,
        "txBytes": 987654321,
        "speedMbps": 1000,
        "linkUp": true
      }
    ],
    "thermal": {
      "zones": [
        { "name": "cpu-thermal", "temperature": 62.5, "typeLabel": "cpu" },
        { "name": "npu-thermal", "temperature": 72.1, "typeLabel": "npu" }
      ],
      "throttleActive": false
    },
    "disk": {
      "totalGb": 58.5,
      "usedGb": 23.4,
      "availableGb": 35.1,
      "inodeTotal": 3840000,
      "inodeUsed": 156000,
      "inodeAvailable": 3684000
    }
  },
  "timestamp": 1694169600000
}
```

### 3.2 新增端点（可选，用于实时推送）

```
WebSocket /ws/system/metrics
```

**推送数据**：
```json
{
  "type": "metrics_update",
  "data": {
    "cpu": { "overallPercent": 45.2 },
    "npu": { "cores": [...] },
    "network": [{ "name": "eth0", "rxBytes": ... }]
  },
  "timestamp": 1694169600000
}
```

## 4. 关键实现细节

### 4.1 NPU 多核心采集

```rust
// crates/infer/src/npu/rknn.rs

pub struct RknnNpuDevice {
    cores: Vec<RknnCore>,
}

struct RknnCore {
    index: u32,
    devfreq_path: PathBuf,
}

impl RknnNpuDevice {
    pub fn detect() -> Option<Self> {
        let mut cores = Vec::new();
        
        // 探测 RK3588 的 3 个 NPU 核心
        for (idx, path) in [
            (0, "/sys/class/devfreq/ff100000.npu"),
            (1, "/sys/class/devfreq/ff110000.npu"),
            (2, "/sys/class/devfreq/ff120000.npu"),
        ].iter() {
            if Path::new(path).exists() {
                cores.push(RknnCore {
                    index: *idx,
                    devfreq_path: PathBuf::from(path),
                });
            }
        }
        
        if cores.is_empty() {
            None
        } else {
            Some(Self { cores })
        }
    }
    
    pub async fn collect_metrics(&self) -> NpuMetrics {
        let mut core_metrics = Vec::new();
        
        for core in &self.cores {
            let utilization = self.read_utilization(&core.devfreq_path).await;
            let frequency = self.read_frequency(&core.devfreq_path).await;
            
            core_metrics.push(NpuCoreMetrics {
                core_id: core.index,
                utilization_percent: utilization,
                frequency_mhz: frequency,
                power_watts: None,  // RK3588 不直接暴露功耗
            });
        }
        
        NpuMetrics {
            device_type: "rk3588-npu".to_string(),
            cores: core_metrics,
            // ... 其他字段
        }
    }
}
```

### 4.2 CPU 多核心采集

```rust
// crates/api/src/metrics/cpu.rs

pub struct CpuCollector;

impl CpuCollector {
    pub async fn collect() -> CpuMetrics {
        // 读取 /proc/stat
        let content = tokio::fs::read_to_string("/proc/stat").await
            .unwrap_or_default();
        
        let mut per_core = Vec::new();
        let mut total_usage = 0.0;
        
        for line in content.lines() {
            if line.starts_with("cpu") && !line.starts_with("cpu ") {
                // 单个核心：cpu0, cpu1, cpu2...
                if let Some(metrics) = Self::parse_core_line(line) {
                    total_usage += metrics.usage_percent;
                    per_core.push(metrics);
                }
            }
        }
        
        let overall = if !per_core.is_empty() {
            total_usage / per_core.len() as f64
        } else {
            0.0
        };
        
        CpuMetrics {
            overall_percent: overall,
            per_core,
            temperature: Self::read_temperature().await,
            frequency_mhz: Self::read_frequency().await,
            top_processes: Self::get_top_processes(5).await,
        }
    }
    
    fn parse_core_line(line: &str) -> Option<CoreMetrics> {
        // 解析 "cpu0 12345 678 9012 ..."
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 {
            return None;
        }
        
        let core_id: u32 = parts[0].trim_start_matches("cpu").parse().ok()?;
        let user: u64 = parts[1].parse().ok()?;
        let nice: u64 = parts[2].parse().ok()?;
        let system: u64 = parts[3].parse().ok()?;
        let idle: u64 = parts[4].parse().ok()?;
        
        let total = user + nice + system + idle;
        let busy = user + nice + system;
        let usage = if total > 0 {
            (busy as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        
        Some(CoreMetrics {
            core_id,
            usage_percent: usage,
            frequency_mhz: None,  // 需要单独读取
            temperature: None,
        })
    }
}
```

## 5. 兼容性与迁移

### 5.1 向后兼容

- 现有字段保持不变，新增字段为 `Option` 类型
- 前端对 `null` 值进行优雅降级
- 无 NPU 设备时返回 `npu: null`

### 5.2 平台降级策略

| 平台 | CPU | NPU | 网络 | 温度 |
|------|-----|-----|------|------|
| Linux (RK3588) | ✅ 完整 | ✅ 3 核心 | ✅ 完整 | ✅ 完整 |
| Linux (Ascend) | ✅ 完整 | ✅ ACL API | ✅ 完整 | ✅ 完整 |
| Linux (通用) | ✅ 完整 | ❌ 无 | ✅ 完整 | ⚠️ 部分 |
| macOS | ⚠️ 部分 | ❌ 无 | ⚠️ 部分 | ❌ 无 |

## 6. 性能考虑

### 6.1 采样策略

```rust
// 自适应采样间隔
impl MetricsCollector {
    fn get_sample_interval(&self) -> Duration {
        match self.system_load {
            load if load > 80.0 => Duration::from_millis(500),   // 高负载
            load if load < 20.0 => Duration::from_secs(10),      // 低负载
            _ => Duration::from_secs(2),                         // 默认
        }
    }
}
```

### 6.2 缓存策略

- 静态信息（设备型号、核心数）：启动时缓存
- 半静态信息（频率上限）：60s 缓存
- 动态信息（使用率、温度）：实时采集

## 7. 风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| NPU 驱动路径不一致 | 采集失败 | 自动探测 + 降级 |
| 采样频率过高 | CPU 开销 | 自适应采样 |
| 权限不足 | 部分指标 N/A | 降级显示 |
| ACL 版本不兼容 | Ascend 采集失败 | 版本检测 + 降级 |
