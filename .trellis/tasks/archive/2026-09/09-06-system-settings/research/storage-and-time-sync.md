# 存储保留策略与对时服务调研

状态：已更新为工业级方案，可作为 design.md 输入。

## 一、存储与保留策略（工业级 NVR/VMS 方案）

### 1.1 设计目标

对齐海康/大华 NVR 存储模型，防止抓拍图、告警图和识别图打爆边缘设备空间导致系统崩溃。

### 1.2 现状分析

| 组件 | 当前状态 | 工业级需要 |
|------|---------|-----------|
| `StorageCleanerConfig` | 全部硬编码，运行时不可变 | 即时生效 + 按类型配额 |
| `clean_if_needed()` | 仅按磁盘水位淘汰 | + 按类型配额 + 按天数保留 + 覆盖模式 |
| `StorageCleaner.config` | 普通字段，构造时确定 | `Arc<RwLock<>>` 支持热更新 |
| 证据类型区分 | 有（captures/alarms 表分离） | + 按类型独立配额和保留天数 |

### 1.3 工业级存储策略模型

对齐海康 NVR 配额模式 + 大华 Disk Quota：

```
┌─────────────────────────────────────────┐
│           存储策略引擎                    │
├─────────────────────────────────────────┤
│                                          │
│  第一层：按类型配额（Quota）              │
│    告警图: 最大 XX MB，写满从最老删       │
│    识别图: 最大 XX MB，写满从最老删       │
│    抓拍图: 剩余空间，写满从最老删         │
│                                          │
│  第二层：按类型保留天数（Retention）       │
│    告警图: 保留 30 天，过期自动删         │
│    识别图: 保留 14 天，过期自动删         │
│    抓拍图: 保留 7 天，过期自动删          │
│                                          │
│  第三层：磁盘水位安全网（Watermark）       │
│    < 15% → 开始清理最老的抓拍图          │
│    < 8%  → 冻结普通抓拍，只保留告警      │
│    < 5%  → 全域写熔断                    │
│                                          │
│  第四层：循环覆盖（Overwrite）            │
│    配额写满时自动覆盖最老记录             │
│    或写满停止（stop 模式）               │
│                                          │
└─────────────────────────────────────────┘
```

### 1.4 可编辑配置

#### 只读运行状态

| 字段 | 来源 | 说明 |
|------|------|------|
| 证据目录路径 | `AppConfig.storage.evidence_dir` | 只读 |
| 总磁盘容量 | `statvfs` | 实时 |
| 已用 / 可用空间 | `statvfs` | 实时 |
| 磁盘使用率 | `statvfs` | 实时 |
| Inode 使用率 | `statvfs` | 实时 |
| 当前健康等级 | `StorageHealthLevel` | Normal/Evicting/Emergency/Critical |
| 各类型已用空间 | 按 evidence_type 分组统计 | 实时 |
| 各类型记录数 | 按 evidence_type 分组 count | 实时 |

#### 可编辑配置（按类型保留 + 配额）

| 字段 | 类型 | 范围 | 默认值 | 说明 |
|------|------|------|--------|------|
| `alarmRetentionDays` | u32 | 1–365 | 30 | 告警图保留天数 |
| `alarmQuotaMb` | u64 | 0–∞ | 0 (不限) | 告警图最大空间 (MB)，0=不限 |
| `recognitionRetentionDays` | u32 | 1–365 | 14 | 识别图保留天数 |
| `recognitionQuotaMb` | u64 | 0–∞ | 0 (不限) | 识别图最大空间 (MB) |
| `captureRetentionDays` | u32 | 1–365 | 7 | 抓拍图保留天数 |
| `captureQuotaMb` | u64 | 0–∞ | 0 (不限) | 抓拍图最大空间 (MB) |
| `overwriteMode` | enum | overwrite/stop | overwrite | 写满时覆盖最老 or 停止写入 |
| `autoCleanupEnabled` | bool | - | true | 自动清理总开关 |
| `minFreeRatio` | f64 | 0.05–0.50 | 0.15 | 触发清理水位 |
| `targetFreeRatio` | f64 | 0.10–0.60 | 0.25 | 清理目标水位 |
| `emergencyFreeRatio` | f64 | 0.03–0.20 | 0.08 | 紧急水位 |
| `criticalFreeRatio` | f64 | 0.01–0.10 | 0.05 | 熔断水位 |
| `batchDeleteSize` | u32 | 10–500 | 100 | 单批删除数 |

#### 字段约束

- `targetFreeRatio` > `minFreeRatio`（回滞防抖）
- `emergencyFreeRatio` < `minFreeRatio`
- `criticalFreeRatio` < `emergencyFreeRatio`
- 配额为 0 表示不限，不限的类型不参与配额淘汰

### 1.5 清理算法

```
clean_if_needed():
  if !autoCleanupEnabled → 仅执行水位安全网

  // 第一层：按类型配额淘汰
  for type in [alarm, recognition, capture]:
    if type.quota > 0 && type.used > type.quota:
      delete oldest records of this type until used <= quota

  // 第二层：按类型保留天数淘汰
  for type in [alarm, recognition, capture]:
    delete records where created_at < now - type.retention_days

  // 第三层：磁盘水位安全网
  if free_ratio < minFreeRatio:
    // 从最低重要级开始删
    delete oldest captures until free_ratio >= targetFreeRatio
    if still insufficient:
      delete oldest recognitions
    if still insufficient:
      delete oldest alarms  // 最后手段

  // 第四层：紧急/熔断
  if free_ratio < emergencyFreeRatio:
    freeze normal captures (禁止新抓拍写入)
  if free_ratio < criticalFreeRatio:
    circuit_breaker: 全域写熔断

  // 覆盖模式处理
  if overwriteMode == "stop" && free_ratio < criticalFreeRatio:
    reject new writes  // 写满停止
```

### 1.6 配置热更新实现

```rust
// StorageCleaner.config 改为 Arc<RwLock<>>
pub struct StorageCleaner {
    config: Arc<RwLock<StorageCleanerConfig>>,
    metrics: Arc<EvictionMetrics>,
    dispatcher: UnlinkDispatcher,
    _worker_handle: tokio::task::JoinHandle<()>,
}

impl StorageCleaner {
    /// 运行时更新配置（用户保存设置后调用）
    pub async fn update_config(&self, new_config: StorageCleanerConfig) {
        *self.config.write().await = new_config;
    }

    /// clean_if_needed 每次读取最新配置
    pub async fn clean_if_needed<S: EvictionStore>(&self, store: &S) -> Result<Option<EvictionReport>> {
        let config = self.config.read().await.clone();
        // 用 config 执行清理...
    }
}
```

### 1.7 生效策略

所有配置项**即时生效**，无需重启：

1. 用户保存 → `PUT /api/v1/system/storage/config`
2. 后端写入 `system_configs` 表（JSON）
3. 调用 `cleaner.update_config(new_config)`
4. 下次清理周期自动使用新配置

### 1.8 API 设计

```
GET  /api/v1/system/storage/status    → 只读运行状态（磁盘、健康、各类型统计）
GET  /api/v1/system/storage/config    → 可编辑配置
PUT  /api/v1/system/storage/config    → 更新配置
POST /api/v1/system/storage/cleanup   → 手动触发清理
```

### 1.9 数据库 Schema

```sql
-- system_configs 表新增 key
-- 存储配置 (JSON, 工业级)
INSERT INTO system_configs (key, value) VALUES (
    'storage_config',
    '{
        "alarmRetentionDays": 30,
        "alarmQuotaMb": 0,
        "recognitionRetentionDays": 14,
        "recognitionQuotaMb": 0,
        "captureRetentionDays": 7,
        "captureQuotaMb": 0,
        "overwriteMode": "overwrite",
        "autoCleanupEnabled": true,
        "minFreeRatio": 0.15,
        "targetFreeRatio": 0.25,
        "emergencyFreeRatio": 0.08,
        "criticalFreeRatio": 0.05,
        "batchDeleteSize": 100
    }'
);
```

### 1.10 与海康/大华的对齐

| 海康/大华概念 | Heimdall 对齐 |
|--------------|--------------|
| 配额模式 (Quota) | `alarmQuotaMb` / `recognitionQuotaMb` / `captureQuotaMb` |
| 循环覆盖 | `overwriteMode: "overwrite"` |
| 写满停止 | `overwriteMode: "stop"` |
| 低空间告警 | `minFreeRatio` 水位触发 |
| 低配额告警 | 各类型配额超限时触发 |
| 录像类型（定时/移动/报警/智能） | 抓拍/识别/告警（按 AI 业务分） |
| 硬盘健康检测 | 已有 `detect_emmc_health()` |

---

## 二、对时服务

### 2.1 现状分析

| 组件 | 当前状态 |
|------|---------|
| NTP 客户端 | **不存在**，代码中无任何 NTP/chrony/timesyncd 集成 |
| 系统时钟读取 | 依赖 OS 系统时钟（`chrono`, `std::time::SystemTime`） |
| 时间跳变防护 | `ring_buffer.rs` 有 NTP backward-step 检测，但只是消费者 |
| 时区处理 | 前端用 `Intl.DateTimeFormat`，后端用 UTC 毫秒 |

**关键发现**：Heimdall 完全依赖宿主 OS 的时间同步能力。系统设置的"对时服务"分组应定位为**系统时间状态展示和 NTP 服务管理**，而非自建 NTP 客户端。

### 2.2 检测与适配策略

不自建 NTP 客户端，封装宿主 OS 已有的 NTP 服务：

```
检测顺序：
1. timedatectl → 通用入口（systemd 系统标配）
2. chrony → 检查 chrony.service 是否 active
3. systemd-timesyncd → 检查 systemd-timesyncd.service 是否 active
4. ntpd → 检查 ntp.service（传统 NTP，少见）
```

### 2.3 能力矩阵

| 操作 | timedatectl | chrony | timesyncd | 说明 |
|------|------------|--------|-----------|------|
| 读取系统时间 | ✅ | ✅ | ✅ | `timedatectl show -p TimeUSec` |
| 读取时区 | ✅ | - | - | `timedatectl show -p Timezone` |
| 修改时区 | ✅ | - | - | `timedatectl set-timezone Asia/Shanghai` |
| 启用/禁用 NTP | ✅ | - | - | `timedatectl set-ntp true/false` |
| 读取同步状态 | ✅ | ✅ | ✅ | `timedatectl show -p NTPSynchronized` |
| 读取 NTP 服务器 | - | ✅ | ✅ | chrony: `chronyc sources`；timesyncd: `timedatectl show -p Server` |
| 修改 NTP 服务器 | - | ✅ | ✅ | 写配置文件 + 重启服务 |
| 读取时间偏移 | - | ✅ | ⚠️ | `chronyc tracking` 有详细偏移；timesyncd 信息有限 |
| 手动设置时间 | ✅ | - | - | `timedatectl set-time` (高风险) |

### 2.4 只读运行状态

| 字段 | 来源 | 说明 |
|------|------|------|
| 系统当前时间 | `chrono::Utc::now()` | UTC 毫秒，前端格式化显示 |
| 本地时间 | `chrono::Local::now()` | 含时区偏移 |
| 时区 | `timedatectl show -p Timezone` | 系统时区 |
| NTP 同步状态 | `timedatectl show -p NTPSynchronized` | 是否已同步 |
| NTP 服务 | 检测 chrony/ntpd/timesyncd | 当前运行的 NTP 服务 |
| NTP 服务器 | chrony: `chronyc sources`；timesyncd: `timedatectl show -p Server` | 当前同步源 |
| 时间偏移 | `chronyc tracking` 或 `timedatectl show -p TimeUSec` | 与 NTP 源的偏移量 |

### 2.5 可编辑配置

| 字段 | 类型 | 范围 | 默认值 | 说明 |
|------|------|------|--------|------|
| `ntpEnabled` | bool | - | true | 是否启用 NTP 自动同步 |
| `ntpServer` | string | URL/hostname | 系统默认 | NTP 服务器地址 |
| `timezone` | string | IANA tz | 系统当前 | 系统时区 |

### 2.6 手动设置时间（高风险操作）

#### 安全流程

```
管理员点击"立即设置"
  ↓
弹出确认对话框：
  "此操作将修改系统时钟，可能导致：
   • 视频时间戳跳变，影响录像回放连续性
   • 检测日志时间不连续
   • 已生成的告警时间戳与实际不符
   建议优先使用 NTP 自动同步。"
  ↓
后端执行：
  1. 记录操作日志（旧时间 → 新时间）
  2. timedatectl set-time "2026-09-06 16:00:00"
  3. 验证新时间已生效
  4. 返回结果
```

#### 安全约束

| 约束 | 说明 |
|------|------|
| 时间差不超过一年 | 防止误操作设置到 2099 或 1970 |
| NTP 启用时警告 | 提示"NTP 启用中，手动设置后可能被自动校正" |
| 操作日志审计 | 记录旧时间、新时间、操作者、时间戳 |
| 需二次确认 | 弹窗说明影响后才执行 |

#### 后端实现

```rust
pub async fn set_system_time(time_str: &str) -> Result<()> {
    // 1. 校验时间格式
    let parsed = NaiveDateTime::parse_from_str(time_str, "%Y-%m-%d %H:%M:%S")?;
    
    // 2. 安全边界校验：不能设置到太远的过去或未来
    let now = Utc::now();
    let target = parsed.and_utc();
    let diff = (target - now).num_days().abs();
    if diff > 365 {
        return Err("不能设置超过一年的时间差".into());
    }
    
    // 3. 记录操作日志
    tracing::warn!(
        old_time = %now.format("%Y-%m-%d %H:%M:%S"),
        new_time = time_str,
        "手动设置系统时间"
    );
    
    // 4. 执行设置
    let status = Command::new("timedatectl")
        .args(["set-time", time_str])
        .status()
        .await?;
    
    if !status.success() {
        return Err("设置系统时间失败".into());
    }
    
    // 5. 验证新时间
    // ...
    
    Ok(())
}
```

### 2.7 风险评估

| 操作 | 风险等级 | 影响 | 确认要求 |
|------|---------|------|---------|
| 查看当前时间 | 无 | 只读 | 无 |
| 修改时区 | 低 | 影响日志时间显示，不影响硬件时间 | 二次确认 |
| 修改 NTP 服务器 | 中 | 可能短暂时间跳变 | 二次确认 |
| 启用/禁用 NTP | 中 | 禁用后时间可能漂移 | 二次确认 |
| 手动设置系统时间 | **高** | 时间跳变影响 PTS、检测时间戳、日志一致性 | 强确认 + 警告 |

### 2.8 后端实现架构

```
crates/app/src/ntp/
├── mod.rs          // TimeService trait + factory
├── timedatectl.rs  // systemd timedatectl 适配
├── chrony.rs       // chrony 适配
├── timesyncd.rs    // systemd-timesyncd 适配
└── types.rs        // TimeStatus, NtpConfig, NtpService
```

**TimeService trait**：
```rust
#[async_trait]
pub trait TimeService: Send + Sync {
    /// 获取当前系统时间状态
    async fn get_status(&self) -> Result<TimeStatus>;
    /// 获取当前 NTP 配置
    async fn get_config(&self) -> Result<NtpConfig>;
    /// 设置时区
    async fn set_timezone(&self, tz: &str) -> Result<()>;
    /// 启用/禁用 NTP
    async fn set_ntp_enabled(&self, enabled: bool) -> Result<()>;
    /// 设置 NTP 服务器
    async fn set_ntp_server(&self, server: &str) -> Result<()>;
    /// 手动触发 NTP 同步
    async fn force_sync(&self) -> Result<()>;
    /// 手动设置系统时间 (高风险)
    async fn set_system_time(&self, time: &str) -> Result<()>;
}
```

### 2.9 API 设计

```
GET  /api/v1/system/time/status   → 只读时间状态
GET  /api/v1/system/time/config   → 可编辑配置
PUT  /api/v1/system/time/config   → 更新配置（时区/NTP/服务器）
POST /api/v1/system/time/sync     → 手动触发 NTP 同步
POST /api/v1/system/time/set      → 手动设置系统时间（高风险，需确认）
```

### 2.10 首版范围

| 功能 | 首版 | 后续 |
|------|------|------|
| 显示系统时间和时区 | ✅ | - |
| 显示 NTP 同步状态 | ✅ | - |
| 修改时区 | ✅ | - |
| 启用/禁用 NTP | ✅ | - |
| 修改 NTP 服务器 | ✅ | 多源配置 |
| 手动触发 NTP 同步 | ✅ | - |
| 手动设置系统时间 | ✅ | 安全约束优化 |
| chrony 适配 | ✅ (若已安装) | 深度集成 |
| timesyncd 适配 | ✅ (systemd 默认) | - |
| 时间偏移量显示 | ✅ (若 NTP 服务支持) | - |

---

## 三、与其他分组的关系

### 3.1 系统概览（只读信息源）

系统概览分组聚合以下只读信息：

| 信息 | 来源 | 说明 |
|------|------|------|
| 软件版本 | 编译时嵌入或 `CARGO_PKG_VERSION` | |
| 设备型号 | `/sys/devices/virtual/dmi/id/board_name` 或 `/proc/cpuinfo` | |
| 操作系统 | `uname -r/v` 或 `/etc/os-release` | |
| CPU 使用率 | `/proc/stat` 或 `top` | |
| 内存使用率 | `/proc/meminfo` 或 `free` | |
| NPU/GPU 状态 | 平台 SDK 查询 | 不可用则显示"不可用" |
| 运行时间 | `uptime` 或 `/proc/uptime` | |
| 磁盘状态 | `statvfs` + `StorageHealthLevel` | 复用存储模块 |
| 活跃摄像头数 | `cameras` 表 count | |
| 活跃任务数 | `tasks` 表 count | |
| 最近告警数 | `alarms` 表近 24h count | |

系统概览不做任何编辑操作，所有字段只读。

### 3.2 账号与安全（复用现有）

- 直接调用 `GET /api/v1/auth/me` 获取账号信息
- 修改密码复用现有 `PUT /api/v1/auth/password`
- 前端复用 `ChangePasswordModal` 组件
- 无需新增后端代码

---

## 四、数据库 Schema 扩展

### system_configs 表新增 Key

```sql
-- 存储保留策略配置 (JSON)
INSERT INTO system_configs (key, value) VALUES (
    'storage_config',
    '{"retentionDays":30,"minFreeRatio":0.15,"targetFreeRatio":0.25,"emergencyFreeRatio":0.08,"criticalFreeRatio":0.05,"batchDeleteSize":100,"autoCleanupEnabled":true}'
);

-- 对时服务配置 (JSON)
INSERT INTO system_configs (key, value) VALUES (
    'time_config',
    '{"ntpEnabled":true,"ntpServer":"pool.ntp.org","timezone":"Asia/Shanghai"}'
);
```

**注意**：`jwt_secret` 已占用一个 key；新增 `storage_config` 和 `time_config` 两个 JSON key。网络配置因涉及运行时状态和操作流程，不存入 `system_configs`，由网络适配层直接查询系统状态。
