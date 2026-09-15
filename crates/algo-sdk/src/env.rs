//! 算法包局部环境变量隔离模块 (`PackageEnv`)
//!
//! 提供针对单个算法包根目录（`package_root/.env`）的轻量私有键值对解析与参数覆盖支持。
//!
//! # 核心约束与设计原则
//! 1. **零全局污染（Zero Global Leak）**：严格禁止调用 `std::env::set_var`，严禁在多线程环境中修改宿主进程的全局 `environ` 指针。
//! 2. **包级作用域（Package-Scoped Isolation）**：仅在当前算法包实例与初始化生命周期内有效，各算法包互不干扰、互不踩踏。
//! 3. **免编译调参（Zero-Recompile Tuning）**：现场工程师或算法开发者可在 `package_root/.env` 中修改任意阈值或模型路径，即改即生效。
//! 4. **三级优先级阶梯（Precedence Hierarchy）**：
//!    - 第一级（最高）：宿主显式下发的任务配置（`task.parameters`）
//!    - 第二级：算法包私有 `.env`（`package_root/.env`）
//!    - 第三级（保底）：代码内部硬编码固定默认值

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::error::AlgoError;

/// 算法包私有的局部环境变量集合
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageEnv {
    vars: HashMap<String, String>,
}

impl PackageEnv {
    /// 从算法包根目录（`package_root/.env`）加载私有环境变量。
    /// 若文件不存在或读取失败，静默返回空的 `PackageEnv`。
    pub fn load(package_root: &Path) -> Self {
        let env_path = package_root.join(".env");
        Self::load_from_file(&env_path)
    }

    /// 从指定文件路径加载 `.env` 格式文本
    pub fn load_from_file(file_path: &Path) -> Self {
        let Ok(content) = std::fs::read_to_string(file_path) else {
            return Self::default();
        };
        Self::parse_str(&content)
    }

    /// 从字符串解析 `.env` 键值对内容
    pub fn parse_str(content: &str) -> Self {
        let mut vars = HashMap::new();
        for line in content.lines() {
            let trimmed = line.trim();
            // 忽略空行与以 '#' 或 ';' 开头的注释行
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                let key = k.trim().to_string();
                if key.is_empty() {
                    continue;
                }
                let mut val = v.trim();
                // 剔除行尾可能存在的行内注释（如 `KEY=VAL # comment`，引号内的除外）
                if !val.starts_with('"') && !val.starts_with('\'') {
                    if let Some((clean_val, _)) = val.split_once('#') {
                        val = clean_val.trim();
                    }
                }
                // 去除成对的外层引号
                if (val.starts_with('"') && val.ends_with('"') && val.len() >= 2)
                    || (val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2)
                {
                    val = &val[1..val.len() - 1];
                }
                vars.insert(key, val.to_string());
            }
        }
        Self { vars }
    }

    /// 查询字符串值（支持大小写不敏感查找：精确 -> 全大写 -> 全小写，≤64 字节键名 0 堆分配）
    pub fn get(&self, key: &str) -> Option<&str> {
        if let Some(value) = self.vars.get(key) {
            return Some(value.as_str());
        }

        // 短键名（≤64 字节）走栈缓冲区大小写映射，保持旧优先级并消除堆分配。
        let bytes = key.as_bytes();
        if bytes.len() <= 64 {
            let mut buf = [0u8; 64];

            // 保持兼容：全大写键优先于全小写键。
            for (index, &byte) in bytes.iter().enumerate() {
                buf[index] = byte.to_ascii_uppercase();
            }
            if let Ok(mapped) = std::str::from_utf8(&buf[..bytes.len()]) {
                if let Some(value) = self.vars.get(mapped) {
                    return Some(value.as_str());
                }
            }

            for (index, &byte) in bytes.iter().enumerate() {
                buf[index] = byte.to_ascii_lowercase();
            }
            if let Ok(mapped) = std::str::from_utf8(&buf[..bytes.len()]) {
                if let Some(value) = self.vars.get(mapped) {
                    return Some(value.as_str());
                }
            }
        } else {
            if let Some(value) = self.vars.get(&key.to_ascii_uppercase()) {
                return Some(value.as_str());
            }
            if let Some(value) = self.vars.get(&key.to_ascii_lowercase()) {
                return Some(value.as_str());
            }
        }

        None
    }

    /// 获取 `String` 配置值
    pub fn get_str(&self, key: &str) -> Option<String> {
        self.get(key).map(|s| s.to_string())
    }

    /// 获取 `f32` 浮点型配置值
    pub fn get_f32(&self, key: &str) -> Option<f32> {
        self.get(key).and_then(|s| s.parse::<f32>().ok())
    }

    /// 获取 `f64` 浮点型配置值
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(|s| s.parse::<f64>().ok())
    }

    /// 获取 `u32` 无符号整数配置值
    pub fn get_u32(&self, key: &str) -> Option<u32> {
        self.get(key).and_then(|s| s.parse::<u32>().ok())
    }

    /// 获取 `u64` 无符号整数配置值
    pub fn get_u64(&self, key: &str) -> Option<u64> {
        self.get(key).and_then(|s| s.parse::<u64>().ok())
    }

    /// 获取 `usize` 配置值
    pub fn get_usize(&self, key: &str) -> Option<usize> {
        self.get(key).and_then(|s| s.parse::<usize>().ok())
    }

    /// 获取 `i32` 有符号整数配置值
    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.get(key).and_then(|s| s.parse::<i32>().ok())
    }

    /// 获取 `bool` 布尔型配置值（支持 `1/0`, `true/false`, `yes/no`, `on/off`，零堆分配比对）
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        let s = self.get(key)?;
        if s == "1"
            || s.eq_ignore_ascii_case("true")
            || s.eq_ignore_ascii_case("yes")
            || s.eq_ignore_ascii_case("on")
        {
            Some(true)
        } else if s == "0"
            || s.eq_ignore_ascii_case("false")
            || s.eq_ignore_ascii_case("no")
            || s.eq_ignore_ascii_case("off")
        {
            Some(false)
        } else {
            None
        }
    }

    /// 安全解析模型或资源文件路径：
    /// 1. 优先使用当前包 `.env` 中指定的键（`env_key`）；
    /// 2. 未指定时回退到默认相对路径 `default_rel_path`；
    /// 3. 支持绝对路径或相对于 `package_root` 的相对路径；
    /// 4. 严格防路径穿越校验（相对路径不得包含 `..`）；
    /// 5. 确保目标在文件系统上存在且为常规文件。
    pub fn resolve_model_path(
        &self,
        package_root: &Path,
        env_key: &str,
        default_rel_path: &str,
    ) -> Result<PathBuf, AlgoError> {
        let raw_path = self.get(env_key).unwrap_or(default_rel_path);
        let target_path = resolve_candidate_target_path(package_root, raw_path)?;

        let canonical = target_path
            .canonicalize()
            .map_err(|err| AlgoError::ModelLoad {
                reason: format!("无法规范化模型路径 ({target_path:?}): {err}"),
            })?;

        if !canonical.is_file() {
            return Err(AlgoError::ModelLoad {
                reason: format!("模型路径不是有效物理文件: {canonical:?}"),
            });
        }

        Ok(canonical)
    }

    /// 解析可选模型路径。
    ///
    /// 默认路径不存在时返回未规范化的候选路径，由调用方通过 `is_file()` 判断是否启用；
    /// `.env` 显式指定的路径仍必须存在且为普通文件，避免拼写错误被静默吞掉。
    pub fn resolve_optional_model_path(
        &self,
        package_root: &Path,
        env_key: &str,
        default_rel_path: &str,
    ) -> Result<PathBuf, AlgoError> {
        let explicit = self.get(env_key);
        let target_path =
            resolve_candidate_target_path(package_root, explicit.unwrap_or(default_rel_path))?;

        match target_path.canonicalize() {
            Ok(canonical) if canonical.is_file() => Ok(canonical),
            Ok(canonical) => Err(AlgoError::ModelLoad {
                reason: format!("模型路径不是有效物理文件: {canonical:?}"),
            }),
            Err(error) if explicit.is_none() && error.kind() == std::io::ErrorKind::NotFound => {
                Ok(target_path)
            }
            Err(error) => Err(AlgoError::ModelLoad {
                reason: format!("无法规范化模型路径 ({target_path:?}): {error}"),
            }),
        }
    }

    /// 当前环境变量集合是否为空
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    /// 当前包含的配置项数量
    pub fn len(&self) -> usize {
        self.vars.len()
    }
}

fn resolve_candidate_target_path(
    package_root: &Path,
    raw_path: &str,
) -> Result<PathBuf, AlgoError> {
    let candidate = Path::new(raw_path);
    if candidate.is_absolute() {
        Ok(candidate.to_path_buf())
    } else {
        // 安全防穿透检查：相对路径不允许包含 ParentDir (..)
        if candidate
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(AlgoError::ModelLoad {
                reason: format!("模型相对路径非法，包含父级遍历组件 '..': {raw_path}"),
            });
        }
        Ok(package_root.join(candidate))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_parse_str_basic_and_types() {
        let content = r#"
            # 基础配置注释
            DETECTION_CONFIDENCE_THRESHOLD = 0.45
            min_face_size=32
            ENABLE_DEBUG = true
            custom_label="custom_alarm"
            quality_max_yaw='35.5'
            ; 分号注释
            EMPTY_VAL=
        "#;

        let env = PackageEnv::parse_str(content);
        assert_eq!(env.get_f32("detection_confidence_threshold"), Some(0.45));
        assert_eq!(env.get_f32("DETECTION_CONFIDENCE_THRESHOLD"), Some(0.45));
        assert_eq!(env.get_u32("min_face_size"), Some(32));
        assert_eq!(env.get_u32("MIN_FACE_SIZE"), Some(32));
        assert_eq!(env.get_bool("enable_debug"), Some(true));
        assert_eq!(
            env.get_str("custom_label"),
            Some("custom_alarm".to_string())
        );
        assert_eq!(env.get_f32("quality_max_yaw"), Some(35.5));
        assert_eq!(env.get("NON_EXISTING"), None);
    }

    #[test]
    fn test_get_case_fallback_preserves_precedence() {
        let env = PackageEnv::parse_str("MIXED=value_from_upper\nmixed=value_from_lower");

        assert_eq!(env.get("MiXeD"), Some("value_from_upper"));
        assert_eq!(env.get("mixed"), Some("value_from_lower"));
    }

    #[test]
    fn test_load_from_temp_file() {
        let unique_id = uuid::Uuid::now_v7();
        let dir = std::env::temp_dir().join(format!("algo_sdk_env_test_{unique_id}"));
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");
        let env_file = dir.join(".env");
        let mut f = std::fs::File::create(&env_file).expect("创建临时文件失败");
        writeln!(f, "MODEL_PATH=model/test.rknn").expect("写入失败");
        writeln!(f, "CONFIDENCE=0.8").expect("写入失败");
        drop(f);

        let env = PackageEnv::load(&dir);
        assert_eq!(
            env.get_str("MODEL_PATH"),
            Some("model/test.rknn".to_string())
        );
        assert_eq!(env.get_f32("confidence"), Some(0.8));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_optional_model_path_allows_missing_default_only() {
        let unique_id = uuid::Uuid::now_v7();
        let dir = std::env::temp_dir().join(format!("algo_sdk_optional_model_test_{unique_id}"));
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");

        let env = PackageEnv::default();
        let missing = env
            .resolve_optional_model_path(&dir, "OPTIONAL_MODEL_PATH", "model/optional.rknn")
            .expect("默认可选模型缺失时应返回候选路径");
        assert_eq!(missing, dir.join("model/optional.rknn"));

        let explicit = PackageEnv::parse_str("OPTIONAL_MODEL_PATH=model/optional.rknn");
        assert!(explicit
            .resolve_optional_model_path(&dir, "OPTIONAL_MODEL_PATH", "model/optional.rknn")
            .is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_model_path_safety() {
        let unique_id = uuid::Uuid::now_v7();
        let dir = std::env::temp_dir().join(format!("algo_sdk_env_safety_test_{unique_id}"));
        let model_dir = dir.join("model");
        std::fs::create_dir_all(&model_dir).expect("创建模型目录失败");
        let default_model = model_dir.join("default.rknn");
        std::fs::File::create(&default_model).expect("创建模型文件失败");

        let env = PackageEnv::default();
        // 1. 回退到默认路径
        let resolved = env
            .resolve_model_path(&dir, "MODEL_PATH", "model/default.rknn")
            .expect("应当成功解析默认模型");
        assert_eq!(resolved, default_model.canonicalize().expect("规范化失败"));

        // 2. 拒绝带有 .. 的穿透路径
        let malicious_env = PackageEnv::parse_str("MODEL_PATH=../secret.rknn");
        let err = malicious_env
            .resolve_model_path(&dir, "MODEL_PATH", "model/default.rknn")
            .expect_err("应当拒绝穿透路径");
        assert!(matches!(err, AlgoError::ModelLoad { .. }));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
