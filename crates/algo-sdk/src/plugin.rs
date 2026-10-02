//! 算法插件核心 Trait 与初始化上下文 (plugin)

use std::path::Path;

use serde::de::DeserializeOwned;

use crate::c_abi::*;
use crate::emitter::ResultEmitter;
use crate::error::AlgoError;
use crate::frame::SafeFrame;
use crate::runtime::FallbackPolicy;

/// 算法实例初始化上下文
#[derive(Debug)]
pub struct InitContext<'a> {
    /// 算法包在宿主物理磁盘上的根目录
    pub package_root: &'a Path,
    /// 宿主平台标识 (如 "rk3588", "darwin-aarch64", "ascend-910b")
    pub platform_id: &'a str,
    /// 算法实例唯一 ID
    pub instance_id: &'a str,
    /// 是否处于安装自检模式 (Self-Test)
    pub is_self_test: bool,
    /// 调用方**显式**声明的回退策略；`None` 表示交给平台与自检规则解析。
    ///
    /// 生产路径（宿主经 C ABI 装载）恒为 `None`，策略由宿主自报的 `platform_id`
    /// 与是否处于自检模式决定；只有本地开发工具（`run_local` 等）才应设为
    /// [`FallbackPolicy::Allow`]，因为开发机物理上没有 `librknnrt`，其意图是
    /// "本机跑通模拟回退"，而不是伪装成生产硬件推理。
    ///
    /// **该字段不可翻越安装自检硬门**：`is_self_test == true` 时策略恒为
    /// [`FallbackPolicy::RequireHardware`]，本字段被忽略。
    pub fallback_policy_override: Option<FallbackPolicy>,
}

impl<'a> InitContext<'a> {
    /// 构造新的初始化上下文
    ///
    /// 回退策略交由平台与自检规则解析（`fallback_policy_override == None`），
    /// 与历史行为逐位一致。
    pub fn new(
        package_root: &'a Path,
        platform_id: &'a str,
        instance_id: &'a str,
        is_self_test: bool,
    ) -> Self {
        Self {
            package_root,
            platform_id,
            instance_id,
            is_self_test,
            fallback_policy_override: None,
        }
    }

    /// 显式声明回退策略（本地开发工具用，加法式链式入口）
    ///
    /// 典型用法：`run_local` 等开发工具在无 `librknnrt` 的机器上以
    /// [`FallbackPolicy::Allow`] 运行模拟回退，不依赖 `.env` 逃生口。
    pub fn with_fallback_policy_override(mut self, policy: FallbackPolicy) -> Self {
        self.fallback_policy_override = Some(policy);
        self
    }

    /// 加载当前算法包根目录下的私有 `.env` 文件（不污染全局环境）
    pub fn load_env(&self) -> crate::env::PackageEnv {
        crate::env::PackageEnv::load(self.package_root)
    }
}

/// 算法插件核心契约 Trait
pub trait AlgoPlugin: Sized + Send + 'static {
    /// 插件配置类型，必须支持 Serde 反序列化与默认值
    type Config: DeserializeOwned + Default;

    /// 初始化算法插件实例（通常加载模型权重与初始化推理会话）
    fn init(ctx: &InitContext<'_>, config: Self::Config) -> Result<Self, AlgoError>;

    /// 处理单帧视频数据并向宿主发射检测/告警结果
    fn process(
        &mut self,
        frame: SafeFrame<'_>,
        emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError>;

    /// 刷新/冲刷算法内部缓冲区（如跟踪器或时序分析算法）
    fn flush(&mut self, _emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
        Ok(())
    }

    /// 更新运行时配置；插件未实现动态更新时明确返回 NotImplemented。
    fn update_config(&mut self, _config: Self::Config) -> Result<(), AlgoError> {
        Err(AlgoError::NotImplemented)
    }

    /// 更新空间布防规则列表（如 ROI 多边形、绊线等）
    fn set_rules(&mut self, _rules: &[AvRule]) -> Result<(), AlgoError> {
        Ok(())
    }
}
