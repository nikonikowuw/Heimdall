//! 系统级应用配置加载器
//!
//! 优先级架构：
//! 环境变量 (`ARGUS_*`) > 本地 `.env` 文件 > `config.toml` > 代码内置默认值

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 全局应用统一配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub pipeline: PipelineConfig,
    #[serde(default)]
    pub media: MediaConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

/// HTTP/WebSocket 服务配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

/// 数据库连接配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_path")]
    pub path: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: default_db_path(),
            max_connections: default_max_connections(),
        }
    }
}

/// 本地持久化存储配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_evidence_dir")]
    pub evidence_dir: PathBuf,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            evidence_dir: default_evidence_dir(),
        }
    }
}

/// 视频分析管线与 VPU 算力调度配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// 全局最大并发硬件抓拍解码器通道数 (RK3588 推荐 4~6，RK3568 推荐 1~2，昇腾 310B 推荐 6~8)
    #[serde(default = "default_max_concurrent_decoders")]
    pub max_concurrent_decoders: usize,

    /// 抓拍通道配额等待超时 (毫秒，超时自动降级复用子码流，杜绝告警延迟)
    #[serde(default = "default_permit_timeout_ms")]
    pub permit_timeout_ms: u64,

    /// 极速单帧硬解 vs 前向追帧硬解的时间戳相位差阈值 (毫秒，默认 500ms)
    #[serde(default = "default_phase_diff_threshold_ms")]
    pub phase_diff_threshold_ms: i64,

    /// 前向追帧允许硬解的最大压缩包数量 (默认 30 包，防大 GOP 连续解码)
    #[serde(default = "default_max_burst_packets")]
    pub max_burst_packets: usize,

    /// 前向追帧硬实时最大延时预算 (毫秒，超过此时间立即自适应熔断，杜绝卡死)
    #[serde(default = "default_max_burst_timeout_ms")]
    pub max_burst_timeout_ms: u64,

    /// 抓拍策略模式
    #[serde(default)]
    pub capture_mode: pipeline::SnapshotCaptureMode,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            max_concurrent_decoders: default_max_concurrent_decoders(),
            permit_timeout_ms: default_permit_timeout_ms(),
            phase_diff_threshold_ms: default_phase_diff_threshold_ms(),
            max_burst_packets: default_max_burst_packets(),
            max_burst_timeout_ms: default_max_burst_timeout_ms(),
            capture_mode: pipeline::SnapshotCaptureMode::default(),
        }
    }
}

/// 流媒体接入配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    /// RTSP 握手单步超时 (毫秒)
    #[serde(default = "default_handshake_timeout_ms")]
    pub handshake_timeout_ms: u64,

    /// 码流静默无报文看门狗超时 (毫秒，判定网络半开或摄像头假死)
    #[serde(default = "default_inactivity_timeout_ms")]
    pub inactivity_timeout_ms: u64,

    /// 主流环形缓冲区容量 (毫秒，用于抓拍索引历史关键帧)
    #[serde(default = "default_ring_buffer_duration_ms")]
    pub ring_buffer_duration_ms: u64,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            handshake_timeout_ms: default_handshake_timeout_ms(),
            inactivity_timeout_ms: default_inactivity_timeout_ms(),
            ring_buffer_duration_ms: default_ring_buffer_duration_ms(),
        }
    }
}

/// 结构化日志配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_filter")]
    pub filter: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            filter: default_log_filter(),
        }
    }
}

// ============================================================================
// 默认值生成函数
// ============================================================================

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8000
}

fn default_db_path() -> String {
    "argus.db".to_string()
}

fn default_max_connections() -> u32 {
    4
}

fn default_evidence_dir() -> PathBuf {
    PathBuf::from(pipeline::DEFAULT_EVIDENCE_DIR)
}

fn default_max_concurrent_decoders() -> usize {
    pipeline::DEFAULT_MAX_CONCURRENT_SNAPSHOT_DECODERS
}

fn default_permit_timeout_ms() -> u64 {
    pipeline::DEFAULT_SNAPSHOT_PERMIT_TIMEOUT_MS
}

fn default_phase_diff_threshold_ms() -> i64 {
    500
}

fn default_max_burst_packets() -> usize {
    30
}

fn default_max_burst_timeout_ms() -> u64 {
    80
}

fn default_handshake_timeout_ms() -> u64 {
    5000
}

fn default_inactivity_timeout_ms() -> u64 {
    6000
}

fn default_ring_buffer_duration_ms() -> u64 {
    4000
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_filter() -> String {
    "info,api=debug,media=debug,pipeline=debug".to_string()
}

// ============================================================================
// 配置加载逻辑
// ============================================================================

/// 加载系统配置
///
/// 1. 加载本地 `.env` 文件到系统环境变量（若存在）；
/// 2. 加载指定的 `config.toml` 或同目录 `config` 配置文件（若存在）；
/// 3. 读取 `ARGUS_` 前缀的环境变量并根据双下划线 `__` 映射覆盖配置字段；
/// 4. 未指定的配置自动填充工业级默认值。
pub fn load_config(custom_path: Option<&str>) -> Result<AppConfig, config::ConfigError> {
    // 1. 加载本地 .env 到系统环境变量
    let _ = dotenvy::dotenv();

    // 2. 构造 config::Config 构建器
    let mut builder = config::Config::builder();

    if let Some(path) = custom_path {
        builder = builder.add_source(config::File::with_name(path).required(true));
    } else {
        // 自动尝试加载 config.toml 或 config.json
        builder = builder.add_source(config::File::with_name("config").required(false));
    }

    // 3. 环境变量覆盖 (ARGUS_前缀，双下划线__层级映射，如 ARGUS_PIPELINE__MAX_CONCURRENT_DECODERS=2)
    builder = builder.add_source(
        config::Environment::with_prefix("ARGUS")
            .prefix_separator("_")
            .separator("__"),
    );

    let cfg = builder.build()?;
    cfg.try_deserialize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.server.host, "0.0.0.0");
        assert_eq!(cfg.server.port, 8000);
        assert_eq!(cfg.database.path, "argus.db");
        assert_eq!(
            cfg.pipeline.max_concurrent_decoders,
            pipeline::DEFAULT_MAX_CONCURRENT_SNAPSHOT_DECODERS
        );
        assert_eq!(
            cfg.pipeline.permit_timeout_ms,
            pipeline::DEFAULT_SNAPSHOT_PERMIT_TIMEOUT_MS
        );
        assert_eq!(cfg.pipeline.max_burst_timeout_ms, 80);
    }

    #[test]
    fn test_load_from_toml_string() {
        let toml_content = r#"
[server]
host = "127.0.0.1"
port = 9000

[pipeline]
max_concurrent_decoders = 8
permit_timeout_ms = 250
max_burst_timeout_ms = 120
capture_mode = "sub_stream_on_large_gap"
"#;

        let config = config::Config::builder()
            .add_source(config::File::from_str(
                toml_content,
                config::FileFormat::Toml,
            ))
            .build()
            .expect("配置构建应成功");

        let cfg: AppConfig = config.try_deserialize().expect("配置反序列化应成功");
        assert_eq!(cfg.server.host, "127.0.0.1");
        assert_eq!(cfg.server.port, 9000);
        assert_eq!(cfg.pipeline.max_concurrent_decoders, 8);
        assert_eq!(cfg.pipeline.permit_timeout_ms, 250);
        assert_eq!(cfg.pipeline.max_burst_timeout_ms, 120);
        assert_eq!(
            cfg.pipeline.capture_mode,
            pipeline::SnapshotCaptureMode::SubStreamOnLargeGap
        );
        // 验证其余未配置项使用默认值
        assert_eq!(cfg.database.path, "argus.db");
        assert_eq!(cfg.media.handshake_timeout_ms, 5000);
    }

    #[test]
    fn test_environment_variable_override() {
        // 设置测试环境变量
        std::env::set_var("ARGUS_SERVER__PORT", "9999");
        std::env::set_var("ARGUS_PIPELINE__MAX_CONCURRENT_DECODERS", "12");
        std::env::set_var("ARGUS_PIPELINE__MAX_BURST_TIMEOUT_MS", "65");

        let cfg = load_config(None).expect("带环境变量的配置加载应成功");
        assert_eq!(cfg.server.port, 9999);
        assert_eq!(cfg.pipeline.max_concurrent_decoders, 12);
        assert_eq!(cfg.pipeline.max_burst_timeout_ms, 65);

        // 清理环境变量防影响其他测试
        std::env::remove_var("ARGUS_SERVER__PORT");
        std::env::remove_var("ARGUS_PIPELINE__MAX_CONCURRENT_DECODERS");
        std::env::remove_var("ARGUS_PIPELINE__MAX_BURST_TIMEOUT_MS");
    }
}
