//! 算法配置定义与三级优先级宏抽象 (`algo_config!`)
//!
//! 提供 `algo_config!` 声明式宏，自动实现：
//! 1. 结构体定义与 `explicit_fields` 显式字段跟踪；
//! 2. `serde::Deserialize` 自动标记显式传参；
//! 3. `Default` 默认值填充；
//! 4. `apply_env` 三级优先级覆盖逻辑（宿主下发 > 包私有 `.env` > 代码默认值）。

use crate::env::PackageEnv;

/// 支持从 `PackageEnv` 环境变量解析的值类型
pub trait FromEnvValue: Sized {
    fn from_env(env: &PackageEnv, key: &str) -> Option<Self>;
}

impl FromEnvValue for f32 {
    fn from_env(env: &PackageEnv, key: &str) -> Option<Self> {
        env.get_f32(key)
    }
}

impl FromEnvValue for f64 {
    fn from_env(env: &PackageEnv, key: &str) -> Option<Self> {
        env.get_f64(key)
    }
}

impl FromEnvValue for u32 {
    fn from_env(env: &PackageEnv, key: &str) -> Option<Self> {
        env.get_u32(key)
    }
}

impl FromEnvValue for u64 {
    fn from_env(env: &PackageEnv, key: &str) -> Option<Self> {
        env.get_u64(key)
    }
}

impl FromEnvValue for usize {
    fn from_env(env: &PackageEnv, key: &str) -> Option<Self> {
        env.get_usize(key)
    }
}

impl FromEnvValue for i32 {
    fn from_env(env: &PackageEnv, key: &str) -> Option<Self> {
        env.get_i32(key)
    }
}

impl FromEnvValue for i64 {
    fn from_env(env: &PackageEnv, key: &str) -> Option<Self> {
        env.get(key).and_then(|s| s.parse::<i64>().ok())
    }
}

impl FromEnvValue for bool {
    fn from_env(env: &PackageEnv, key: &str) -> Option<Self> {
        env.get_bool(key)
    }
}

impl FromEnvValue for String {
    fn from_env(env: &PackageEnv, key: &str) -> Option<Self> {
        env.get_str(key)
    }
}

impl<T: FromEnvValue> FromEnvValue for Option<T> {
    fn from_env(env: &PackageEnv, key: &str) -> Option<Self> {
        T::from_env(env, key).map(Some)
    }
}

impl FromEnvValue for Vec<String> {
    fn from_env(env: &PackageEnv, key: &str) -> Option<Self> {
        env.get_str(key).map(|s| {
            s.split(',')
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect()
        })
    }
}

/// 声明算法实例配置结构体并自动实现三级优先级隔离
#[macro_export]
macro_rules! algo_config {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $(
                $(#[$field_meta:meta])*
                $f_vis:vis $field:ident : $f_type:ty = $default:expr
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis struct $name {
            $(
                $(#[$field_meta])*
                $f_vis $field: $f_type,
            )*
            /// 记录宿主任务配置显式下发的参数名（用于执行三级优先级隔离）
            pub explicit_fields: std::collections::HashSet<String>,
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                #[derive(serde::Deserialize, Default)]
                struct __RawConfig {
                    $(
                        #[serde(default)]
                        $field: Option<$f_type>,
                    )*
                }

                let raw = __RawConfig::deserialize(deserializer)?;
                let mut explicit_fields = std::collections::HashSet::new();

                $(
                    let $field = match raw.$field {
                        Some(v) => {
                            explicit_fields.insert(stringify!($field).to_string());
                            v
                        }
                        None => $default,
                    };
                )*

                Ok(Self {
                    $(
                        $field,
                    )*
                    explicit_fields,
                })
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self {
                    $(
                        $field: $default,
                    )*
                    explicit_fields: std::collections::HashSet::new(),
                }
            }
        }

        impl $name {
            /// 注入当前算法包私有 `.env` 的参数覆盖。
            ///
            /// 【三级优先级阶梯原则】：
            /// 1. 宿主显式下发的任务配置最高级：若宿主已传递该字段，严格保护，不被 `.env` 覆盖；
            /// 2. 宿主未传递该字段时：优先使用 `.env` 局部配置；
            /// 3. 若 `.env` 也未设置：维持代码硬编码默认值。
            #[allow(dead_code)]
            pub fn apply_env(&mut self, env: &$crate::env::PackageEnv) {
                $(
                    if !self.explicit_fields.contains(stringify!($field)) {
                        if let Some(val) = <$f_type as $crate::config::FromEnvValue>::from_env(env, stringify!($field)) {
                            self.$field = val;
                        }
                    }
                )*
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    algo_config! {
        #[derive(Debug, Clone, PartialEq)]
        pub struct TestConfig {
            pub confidence: f32 = 0.5,
            pub iou: f32 = 0.45,
            pub label: Option<String> = None,
            pub tags: Vec<String> = vec!["default".to_string()],
            pub count: usize = 3,
            pub enabled: bool = true,
        }
    }

    #[test]
    fn test_algo_config_defaults() {
        let cfg = TestConfig::default();
        assert_eq!(cfg.confidence, 0.5);
        assert_eq!(cfg.iou, 0.45);
        assert!(cfg.label.is_none());
        assert_eq!(cfg.tags, vec!["default"]);
        assert_eq!(cfg.count, 3);
        assert!(cfg.enabled);
        assert!(cfg.explicit_fields.is_empty());
    }

    #[test]
    fn test_algo_config_deserialization_tracks_explicit() {
        let json = r#"{"confidence": 0.8, "count": 10}"#;
        let cfg: TestConfig = serde_json::from_str(json).expect("deserialization");
        assert_eq!(cfg.confidence, 0.8);
        assert_eq!(cfg.count, 10);
        assert_eq!(cfg.iou, 0.45); // default
        assert!(cfg.explicit_fields.contains("confidence"));
        assert!(cfg.explicit_fields.contains("count"));
        assert!(!cfg.explicit_fields.contains("iou"));
    }

    #[test]
    fn test_algo_config_apply_env_three_tier_precedence() {
        // 宿主显式传递 confidence=0.9
        let json = r#"{"confidence": 0.9}"#;
        let mut cfg: TestConfig = serde_json::from_str(json).expect("deserialization");

        let env = PackageEnv::parse_str(
            r#"
            CONFIDENCE = 0.1
            IOU = 0.65
            LABEL = "my_alarm"
            TAGS = "fire, smoke"
            COUNT = 99
            ENABLED = false
            "#,
        );
        cfg.apply_env(&env);

        // 1. 宿主显式传递的置信度严格保持 0.9，不被 .env 覆盖
        assert_eq!(cfg.confidence, 0.9);
        // 2. 宿主未传递的从 .env 获取
        assert_eq!(cfg.iou, 0.65);
        assert_eq!(cfg.label.as_deref(), Some("my_alarm"));
        assert_eq!(cfg.tags, vec!["fire", "smoke"]);
        assert_eq!(cfg.count, 99);
        assert!(!cfg.enabled);
    }

    algo_config! {
        #[derive(Debug, Clone, PartialEq)]
        pub struct AnotherConfigInSameModule {
            pub ratio: f32 = 1.0,
        }
    }

    #[test]
    fn test_multiple_algo_configs_in_same_module_no_collision() {
        let cfg = AnotherConfigInSameModule::default();
        assert_eq!(cfg.ratio, 1.0);
    }
}
