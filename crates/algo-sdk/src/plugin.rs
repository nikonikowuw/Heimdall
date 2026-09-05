//! 算法插件核心 Trait 与初始化上下文 (plugin)

use std::path::Path;

use serde::de::DeserializeOwned;

use crate::c_abi::*;
use crate::emitter::ResultEmitter;
use crate::error::AlgoError;
use crate::frame::SafeFrame;

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
