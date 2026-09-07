use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::policy::{RGA_DMA32_HEAP_PATH, RGA_SYSTEM_HEAP_PATH};
use crate::cv::types::PixelFormat;
use crate::error::AlgoError;

/// RGA scheduler/core selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RgaCore {
    /// Leave scheduling to librga/the kernel. Validation uses the portable RGA2 profile.
    #[default]
    Auto,
    /// Apply the RGA2-compatible policy and DMA32 allocation requirement.
    Rga2,
    /// Apply the RGA3-compatible policy profile. Core scheduling remains automatic unless
    /// the platform supplies a separately validated scheduler configuration.
    Rga3Core0,
    /// Apply the RGA3-compatible policy profile. Core scheduling remains automatic unless
    /// the platform supplies a separately validated scheduler configuration.
    Rga3Core1,
}

/// Bounded local RGA output-pool configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RgaPoolConfig {
    /// Number of slots allocated during pool construction.
    pub min_idle: usize,
    /// Hard upper bound for total slots, including checked-out slots.
    pub max_size: usize,
    /// Maximum time an acquire may wait for a slot.
    pub acquire_timeout_ms: u64,
    /// Idle slots above `min_idle` are evicted after this duration.
    pub idle_timeout_sec: u64,
    /// Output dimensions for the pool instance.
    pub width: u32,
    pub height: u32,
    /// Output pixel format. The current hardware engine emits RGB24.
    pub format: PixelFormat,
    /// Preferred Linux DMA-BUF heap.
    pub dma_heap_path: Option<String>,
    /// RGA core/policy selection.
    pub core: RgaCore,
}

/// Candidate Linux DMA heap specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DmaHeapCandidate {
    pub path: String,
    pub dma32: bool,
}

/// Builder for validated RGA pool configuration.
#[derive(Debug, Clone, Default)]
pub struct RgaPoolConfigBuilder {
    config: RgaPoolConfig,
}

impl RgaPoolConfigBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn min_idle(mut self, value: usize) -> Self {
        self.config.min_idle = value;
        self
    }

    pub fn max_size(mut self, value: usize) -> Self {
        self.config.max_size = value;
        self
    }

    pub fn acquire_timeout_ms(mut self, value: u64) -> Self {
        self.config.acquire_timeout_ms = value;
        self
    }

    pub fn idle_timeout_sec(mut self, value: u64) -> Self {
        self.config.idle_timeout_sec = value;
        self
    }

    pub fn dimensions(mut self, width: u32, height: u32) -> Self {
        self.config.width = width;
        self.config.height = height;
        self
    }

    pub fn format(mut self, format: PixelFormat) -> Self {
        self.config.format = format;
        self
    }

    pub fn dma_heap_path(mut self, path: impl Into<String>) -> Self {
        self.config.dma_heap_path = Some(path.into());
        self
    }

    pub fn no_dma_heap(mut self) -> Self {
        self.config.dma_heap_path = None;
        self
    }

    pub fn core(mut self, core: RgaCore) -> Self {
        self.config.core = core;
        self
    }

    pub fn build(self) -> Result<RgaPoolConfig, AlgoError> {
        self.config.validate()?;
        Ok(self.config)
    }
}

/// Built-in default configuration.
impl Default for RgaPoolConfig {
    fn default() -> Self {
        Self {
            min_idle: 2,
            max_size: 4,
            acquire_timeout_ms: 50,
            idle_timeout_sec: 10,
            width: 640,
            height: 640,
            format: PixelFormat::Rgb24,
            dma_heap_path: Some(RGA_DMA32_HEAP_PATH.to_string()),
            core: RgaCore::Auto,
        }
    }
}

impl RgaPoolConfig {
    pub fn builder() -> RgaPoolConfigBuilder {
        RgaPoolConfigBuilder::new()
    }

    pub fn validate(&self) -> Result<(), AlgoError> {
        if self.max_size == 0 || self.min_idle > self.max_size {
            return Err(AlgoError::ConfigParse {
                reason: format!(
                    "RGA pool capacity invalid: min_idle={}, max_size={}",
                    self.min_idle, self.max_size
                ),
            });
        }
        if self.width == 0 || self.height == 0 {
            return Err(AlgoError::ConfigParse {
                reason: "RGA pool output dimensions must be non-zero".to_string(),
            });
        }
        if self.format != PixelFormat::Rgb24 {
            return Err(AlgoError::ConfigParse {
                reason: format!("RGA pool output format is unsupported: {:?}", self.format),
            });
        }
        if let Some(path) = &self.dma_heap_path {
            if path.is_empty() {
                return Err(AlgoError::ConfigParse {
                    reason: "RGA DMA heap path must not be empty".to_string(),
                });
            }
        }
        Ok(())
    }

    pub fn acquire_timeout(&self) -> Duration {
        Duration::from_millis(self.acquire_timeout_ms)
    }

    pub fn idle_timeout(&self) -> Duration {
        Duration::from_secs(self.idle_timeout_sec)
    }

    /// Apply process-level overrides without making environment variables part of the API.
    pub fn apply_env_overrides(&mut self) -> Result<(), AlgoError> {
        if let Some(value) = env_parse("ARGUS_RGA_POOL_MIN_IDLE")? {
            self.min_idle = value;
        }
        if let Some(value) = env_parse("ARGUS_RGA_POOL_MAX_SIZE")? {
            self.max_size = value;
        }
        if let Some(value) = env_parse("ARGUS_RGA_POOL_ACQUIRE_TIMEOUT_MS")? {
            self.acquire_timeout_ms = value;
        }
        if let Some(value) = env_parse("ARGUS_RGA_POOL_IDLE_TIMEOUT_SEC")? {
            self.idle_timeout_sec = value;
        }
        if let Some(value) = std::env::var_os("ARGUS_RGA_DMA_HEAP") {
            let value = value.to_string_lossy().into_owned();
            self.dma_heap_path = (!value.is_empty()).then_some(value);
        }
        self.validate()
    }

    pub fn from_env() -> Result<Self, AlgoError> {
        let mut config = Self::default();
        config.apply_env_overrides()?;
        Ok(config)
    }

    pub fn for_output(&self, width: u32, height: u32, format: PixelFormat) -> Self {
        let mut config = self.clone();
        config.width = width;
        config.height = height;
        config.format = format;
        config
    }

    /// Return the primary heap and the generic fallback in deterministic order.
    pub(crate) fn heap_candidates(&self) -> Vec<DmaHeapCandidate> {
        let mut candidates = Vec::with_capacity(3);
        if let Some(path) = &self.dma_heap_path {
            let dma32 = path.contains("dma32");
            candidates.push(DmaHeapCandidate {
                path: path.clone(),
                dma32,
            });
        }
        for (path, dma32) in [(RGA_DMA32_HEAP_PATH, true), (RGA_SYSTEM_HEAP_PATH, false)] {
            if !candidates.iter().any(|candidate| candidate.path == path) {
                candidates.push(DmaHeapCandidate {
                    path: path.to_string(),
                    dma32,
                });
            }
        }
        candidates
    }
}

fn env_parse<T: std::str::FromStr>(name: &str) -> Result<Option<T>, AlgoError> {
    std::env::var(name)
        .ok()
        .map(|value| {
            value.parse::<T>().map_err(|_| AlgoError::ConfigParse {
                reason: format!("{name} must be an unsigned integer"),
            })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_config_is_bounded_and_valid() {
        let config = RgaPoolConfig::default();
        assert_eq!(config.min_idle, 2);
        assert_eq!(config.max_size, 4);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_rga_pool_config_serde_and_env_override() {
        let _guard = ENV_LOCK.lock().expect("environment test lock");
        let config = RgaPoolConfig::default().for_output(320, 320, PixelFormat::Rgb24);
        let json = serde_json::to_string(&config).expect("serialize config");
        let decoded: RgaPoolConfig = serde_json::from_str(&json).expect("deserialize config");
        assert_eq!(decoded, config);

        let old_min = std::env::var_os("ARGUS_RGA_POOL_MIN_IDLE");
        let old_max = std::env::var_os("ARGUS_RGA_POOL_MAX_SIZE");
        std::env::set_var("ARGUS_RGA_POOL_MIN_IDLE", "3");
        std::env::set_var("ARGUS_RGA_POOL_MAX_SIZE", "5");
        let mut overridden = RgaPoolConfig::default();
        overridden.apply_env_overrides().expect("valid overrides");
        assert_eq!(overridden.min_idle, 3);
        assert_eq!(overridden.max_size, 5);
        restore_env("ARGUS_RGA_POOL_MIN_IDLE", old_min);
        restore_env("ARGUS_RGA_POOL_MAX_SIZE", old_max);
    }

    #[test]
    fn builder_validates_and_sets_pool_fields() {
        let config = RgaPoolConfig::builder()
            .min_idle(1)
            .max_size(2)
            .dimensions(320, 320)
            .acquire_timeout_ms(20)
            .build()
            .expect("valid builder config");
        assert_eq!(config.width, 320);
        assert_eq!(config.acquire_timeout_ms, 20);
    }

    #[test]
    fn test_rga_pool_config_validation() {
        let config = RgaPoolConfig {
            min_idle: 3,
            max_size: 2,
            ..RgaPoolConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(AlgoError::ConfigParse { .. })
        ));
    }

    fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }
}
