#!/usr/bin/env bash
# ==============================================================================
# algo-new.sh — 快速脚手架生成全新的标准算法包
#
# 用法:
#   ./scripts/algo-new.sh <platform> <package_name> [alarm_type_id]
#
# 参数:
#   platform:        目标平台 (rk3568 | rk3576 | rk3588 | macos)
#   package_name:    算法包名（蛇形命名，如 phone_detection）
#   alarm_type_id:   [可选] 告警类型标识（如 ALARM_PHONE，默认根据包名自动转大写）
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# 颜色定义
CYAN='\033[36m'
GREEN='\033[32m'
YELLOW='\033[33m'
RED='\033[31m'
RESET='\033[0m'

if [ $# -lt 2 ]; then
    echo -e "${RED}错误: 缺少参数！${RESET}"
    echo "用法: $0 <platform: rk3568|rk3576|rk3588|macos> <package_name> [alarm_type_id]"
    echo "示例: $0 rk3568 phone_detection ALARM_PHONE"
    exit 1
fi

PLATFORM="$1"
PKG_NAME="$2"
ALARM_TYPE_ID="${3:-ALARM_$(echo "${PKG_NAME}" | tr '[:lower:]' '[:upper:]')}"

# 路径与平台映射
case "${PLATFORM}" in
    rk3568)
        TARGET_DIR="${WORKSPACE_ROOT}/algo-packages/rknn/rk3568/${PKG_NAME}"
        WORKSPACE_CARGO="${WORKSPACE_ROOT}/algo-packages/rknn/rk3568/Cargo.toml"
        PLATFORM_ID="rk3568-rknn"
        TARGET_SOC="rk3568"
        REL_SDK_PATH="../../../../crates/algo-sdk"
        CORE_MASK_CONST="algo_sdk::rknn::RKNN_NPU_CORE_0"
        SDK_FEATURES='["rga", "rknn", "testing-hardware"]'
        MODEL_FILE="model/model.rknn"
        ;;
    rk3576)
        TARGET_DIR="${WORKSPACE_ROOT}/algo-packages/rknn/rk3576/${PKG_NAME}"
        WORKSPACE_CARGO="${WORKSPACE_ROOT}/algo-packages/rknn/rk3576/Cargo.toml"
        PLATFORM_ID="rk3576-rknn"
        TARGET_SOC="rk3576"
        REL_SDK_PATH="../../../../crates/algo-sdk"
        CORE_MASK_CONST="algo_sdk::rknn::RKNN_NPU_CORE_0_1"
        SDK_FEATURES='["rga", "rknn", "testing-hardware"]'
        MODEL_FILE="model/model.rknn"
        ;;
    rk3588)
        TARGET_DIR="${WORKSPACE_ROOT}/algo-packages/rknn/rk3588/${PKG_NAME}"
        WORKSPACE_CARGO="${WORKSPACE_ROOT}/algo-packages/rknn/rk3588/Cargo.toml"
        PLATFORM_ID="rk3588-rknn"
        TARGET_SOC="rk3588"
        REL_SDK_PATH="../../../../crates/algo-sdk"
        CORE_MASK_CONST="algo_sdk::rknn::RKNN_NPU_CORE_0_1_2"
        SDK_FEATURES='["rga", "rknn", "testing-hardware"]'
        MODEL_FILE="model/model.rknn"
        ;;
    macos)
        TARGET_DIR="${WORKSPACE_ROOT}/algo-packages/macos/arm64/${PKG_NAME}"
        WORKSPACE_CARGO="${WORKSPACE_ROOT}/algo-packages/macos/Cargo.toml"
        PLATFORM_ID="macos-arm64-coreml"
        TARGET_SOC="apple-silicon"
        REL_SDK_PATH="../../../../crates/algo-sdk"
        CORE_MASK_CONST=""
        SDK_FEATURES='["testing-hardware"]'
        MODEL_FILE="model/model.mlmodelc"
        ;;
    *)
        echo -e "${RED}不支持的平台: ${PLATFORM}。可选: rk3568, rk3576, rk3588, macos${RESET}"
        exit 1
        ;;
esac

if [ -d "${TARGET_DIR}" ]; then
    echo -e "${RED}错误: 目录已存在: ${TARGET_DIR}${RESET}"
    exit 1
fi

echo -e "${CYAN}[algo-new]${RESET} 正在为平台 ${GREEN}${PLATFORM}${RESET} 生成算法包: ${GREEN}${PKG_NAME}${RESET}..."

# 创建目录结构
mkdir -p "${TARGET_DIR}/src/bin"
mkdir -p "${TARGET_DIR}/model"
# 创建占位模型文件（供本地模拟运行与测试通过，生产部署时替换为真实模型权重）
touch "${TARGET_DIR}/${MODEL_FILE}"

# 1. 生成 Cargo.toml
cat <<EOF > "${TARGET_DIR}/Cargo.toml"
[package]
name = "${PKG_NAME}-${PLATFORM_ID}"
version = "1.0.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[[bin]]
name = "run_local"
path = "src/bin/run_local.rs"

[dependencies]
algo-sdk = { path = "${REL_SDK_PATH}", features = ${SDK_FEATURES} }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tracing = "0.1"
EOF

# 2. 生成 manifest.json
cat <<EOF > "${TARGET_DIR}/manifest.json"
{
  "manifest_version": "1.0.0",
  "algorithm_id": "${PKG_NAME}",
  "version": "1.0.0",
  "name": "${PKG_NAME}",
  "algorithm_type": "detector",
  "alarm_type_id": "${ALARM_TYPE_ID}",
  "platform_id": "${PLATFORM_ID}",
  "min_adapter_version": "1.0.0",
  "runtime_constraints": {
    "target_soc": "${TARGET_SOC}"
  },
  "resource_profile": {
    "min_free_memory_mb": 128,
    "fps_tiers": [
      { "fps": 5, "units": 50 },
      { "fps": 15, "units": 150 },
      { "fps": 30, "units": 300 }
    ]
  },
  "self_test": {
    "timeout_ms": 10000,
    "input_mode": "test_image"
  }
}
EOF

# 3. 生成 config.schema.json
cat <<EOF > "${TARGET_DIR}/config.schema.json"
{
  "\$schema": "http://json-schema.org/draft-07/schema#",
  "title": "${PKG_NAME} Configuration",
  "type": "object",
  "properties": {
    "confidence_threshold": {
      "type": "number",
      "minimum": 0.0,
      "maximum": 1.0,
      "default": 0.45,
      "description": "检测框置信度阈值"
    },
    "iou_threshold": {
      "type": "number",
      "minimum": 0.0,
      "maximum": 1.0,
      "default": 0.45,
      "description": "NMS 重叠度抑制阈值"
    },
    "custom_alarm_label": {
      "type": ["string", "null"],
      "default": null,
      "description": "自定义告警标签（可选）"
    }
  }
}
EOF

# 4. 生成 .env 模板
cat <<EOF > "${TARGET_DIR}/.env"
# 算法包局部配置覆盖（优先级低于宿主显式传参，高于代码默认值）
CONFIDENCE_THRESHOLD=0.45
IOU_THRESHOLD=0.45
# CUSTOM_ALARM_LABEL=ALERT
EOF

# 5. 生成 src/config.rs (基于 algo_config! 宏)
cat <<EOF > "${TARGET_DIR}/src/config.rs"
//! 算法配置定义与三级优先级支持

use algo_sdk::algo_config;
pub use algo_sdk::env::PackageEnv;

pub const DEFAULT_CONFIDENCE: f32 = 0.45;
pub const DEFAULT_IOU: f32 = 0.45;

algo_config! {
    /// 实例运行时配置
    #[derive(Debug, Clone, PartialEq)]
    pub struct InstanceConfig {
        /// 检测框置信度过滤阈值
        pub confidence_threshold: f32 = DEFAULT_CONFIDENCE,

        /// NMS 重合度抑制阈值
        pub iou_threshold: f32 = DEFAULT_IOU,

        /// 自定义业务告警标签（可选）
        pub custom_alarm_label: Option<String> = None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = InstanceConfig::default();
        assert_eq!(cfg.confidence_threshold, DEFAULT_CONFIDENCE);
        assert_eq!(cfg.iou_threshold, DEFAULT_IOU);
        assert!(cfg.custom_alarm_label.is_none());
    }
}
EOF

# 6. 生成 src/plugin.rs
if [ "${PLATFORM}" = "macos" ]; then
cat <<EOF > "${TARGET_DIR}/src/plugin.rs"
//! 算法核心检测管线实现 (macOS Apple Silicon)

use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;
use algo_sdk::plugin::{AlgoPlugin, InitContext};
use crate::config::InstanceConfig;

pub struct Detector {
    pub config: InstanceConfig,
}

impl AlgoPlugin for Detector {
    type Config = InstanceConfig;

    fn init(ctx: &InitContext<'_>, mut config: Self::Config) -> Result<Self, AlgoError> {
        let env = ctx.load_env();
        config.apply_env(&env);
        Ok(Self { config })
    }

    fn process(&mut self, _frame: SafeFrame<'_>, _emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
        let _ = &self.config;
        Ok(())
    }

    fn update_config(&mut self, new_config: Self::Config) -> Result<(), AlgoError> {
        self.config = new_config;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use algo_sdk::testing::{LocalPluginRunner, MockFrameBuilder};

    #[test]
    fn test_detector_empty_process() {
        let package_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let ctx = InitContext::new(
            package_root,
            "${PLATFORM_ID}",
            "test-0",
            false,
        );
        let mut detector = Detector::init(&ctx, InstanceConfig::default()).expect("init");
        let frame = MockFrameBuilder::new().dimensions(640, 640).to_nv12(16).build();
        let (elapsed_ms, results) =
            LocalPluginRunner::run_once(&mut detector, frame.as_safe_frame()).expect("run_once");
        assert!(elapsed_ms >= 0.0);
        assert!(results.is_empty());
    }
}
EOF
else
cat <<EOF > "${TARGET_DIR}/src/plugin.rs"
//! 算法核心检测管线实现

use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;
use algo_sdk::plugin::{AlgoPlugin, InitContext};
use algo_sdk::rknn::{RknnSession, RknnSessionOptions};
use crate::config::InstanceConfig;

pub struct Detector {
    pub session: RknnSession,
    pub config: InstanceConfig,
}

impl AlgoPlugin for Detector {
    type Config = InstanceConfig;

    fn init(ctx: &InitContext<'_>, mut config: Self::Config) -> Result<Self, AlgoError> {
        let env = ctx.load_env();
        config.apply_env(&env);

        let model_path = ctx.package_root.join("${MODEL_FILE}");
        let session = RknnSession::open_or_fallback(
            ctx.package_root,
            &model_path,
            RknnSessionOptions::with_core_mask(${CORE_MASK_CONST}),
        )?;

        Ok(Self { session, config })
    }

    fn process(&mut self, _frame: SafeFrame<'_>, _emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
        // 推理逻辑与后处理
        // TODO: 将输入帧经由 RgaCvEngine 预处理后送入 session 推理，并调用 parse_yolov8_int8 解码
        let _ = &self.session;
        let _ = &self.config;
        Ok(())
    }

    fn update_config(&mut self, new_config: Self::Config) -> Result<(), AlgoError> {
        self.config = new_config;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use algo_sdk::testing::{LocalPluginRunner, MockFrameBuilder};

    #[test]
    fn test_detector_empty_process() {
        let package_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let ctx = InitContext::new(
            package_root,
            "${PLATFORM_ID}",
            "test-0",
            false,
        );
        let mut detector = Detector::init(&ctx, InstanceConfig::default()).expect("init");
        let frame = MockFrameBuilder::new().dimensions(640, 640).to_nv12(16).build();
        let (elapsed_ms, results) =
            LocalPluginRunner::run_once(&mut detector, frame.as_safe_frame()).expect("run_once");
        assert!(elapsed_ms >= 0.0);
        assert!(results.is_empty());
    }
}
EOF
fi

# 7. 生成 src/lib.rs (FFI 统一导出宏)
cat <<EOF > "${TARGET_DIR}/src/lib.rs"
//! 算法包动态链接库顶层导出

pub mod config;
pub mod plugin;

algo_sdk::export_algo!(
    crate::plugin::Detector,
    algo_id: "${PKG_NAME}",
    version: "1.0.0",
    algo_type: "detector",
    alarm_type_id: "${ALARM_TYPE_ID}"
);
EOF

# 8. 生成 src/bin/run_local.rs (基于 LocalPluginRunner 的本地调测工具)
LIB_CRATE_NAME="${PKG_NAME//-/_}_${PLATFORM_ID//-/_}"
cat <<EOF > "${TARGET_DIR}/src/bin/run_local.rs"
//! 算法本地调测与基准测试二进制

use std::path::Path;
use algo_sdk::error::AlgoError;
use algo_sdk::plugin::{AlgoPlugin, InitContext};
use algo_sdk::testing::{LocalPluginRunner, MockFrameBuilder};
use ${LIB_CRATE_NAME}::config::InstanceConfig;
use ${LIB_CRATE_NAME}::plugin::Detector;

fn main() -> Result<(), AlgoError> {
    let package_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ctx = InitContext::new(
        package_root,
        "${PLATFORM_ID}",
        "local-debug",
        false,
    );
    let mut detector = Detector::init(&ctx, InstanceConfig::default())?;
    let frame = MockFrameBuilder::new().dimensions(640, 640).to_nv12(16).build();
    let (stats, _) = LocalPluginRunner::benchmark(&mut detector, frame.as_safe_frame(), 10)?;
    println!("{stats}");

    Ok(())
}
EOF

# 9. 自动追加到所在平台的 Cargo.toml workspace members (如果未包含)
MEMBER_SPEC="${PKG_NAME}"
if [ "${PLATFORM}" = "macos" ]; then
    MEMBER_SPEC="arm64/${PKG_NAME}"
fi

if grep -q "\"${MEMBER_SPEC}\"" "${WORKSPACE_CARGO}"; then
    echo -e "${YELLOW}[algo-new]${RESET} 工作区已注册: ${MEMBER_SPEC}"
else
    echo -e "${CYAN}[algo-new]${RESET} 正在将 ${GREEN}${MEMBER_SPEC}${RESET} 注册到 ${WORKSPACE_CARGO}..."
    awk -v member="    \"${MEMBER_SPEC}\"," '/members = \[/ { print; print member; next }1' "${WORKSPACE_CARGO}" > "${WORKSPACE_CARGO}.tmp"
    mv "${WORKSPACE_CARGO}.tmp" "${WORKSPACE_CARGO}"
fi

echo -e "${GREEN}[algo-new]${RESET} 算法包创建成功！"
echo -e "  路径: ${TARGET_DIR}"
echo -e "  你可以立即运行以下命令验证测试："
echo -e "  ${CYAN}cargo test --manifest-path ${WORKSPACE_CARGO} -p ${PKG_NAME}-${PLATFORM_ID}${RESET}"
