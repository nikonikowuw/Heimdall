//! NPU 硬件回退策略与可用性判定 (`FallbackPolicy` / `HardwareStatus` / `HardwareAvailability`)
//!
//! # 背景
//!
//! 任一平台运行时的动态库加载失败都会被静默降级为 `debug_cpu_fallback_path` 模拟会话，
//! 该会话产出写死的模拟检测框。这样一来，"安装自检通过"就无法证明模型真的加载到了硬件。
//! 本模块把降级从"隐式默认"改为"显式策略"，并让调用方能判别三态：
//!
//! - [`HardwareStatus::Hardware`]：真实硬件运行时已就绪；
//! - [`HardwareStatus::Simulated`]：模拟会话（未使用硬件）；
//! - [`HardwareAvailability::Unavailable`]：要求硬件但不可用（构造返回 `Err`）。
//!
//! # 平台判定不使用 `cfg`
//!
//! 判据一律来自宿主自报的 `platform_id`，**不使用编译期 `cfg`**：同一进程可能同时装载
//! 多个平台的算法包，而 `cfg` 只能表达"本包编译成什么"，无法表达"宿主是什么"。
//! 同理，本模块也不反向依赖宿主 `crates/infer` 的 `normalize_platform_id`。

use std::path::Path;

use crate::env::PackageEnv;
use crate::error::AlgoError;

/// 算法包私有 `.env` 中用于**显式**放开 CPU 回退的键名
///
/// 仅作为运维显式覆盖使用（`ALLOW_CPU_FALLBACK=1`），**不作为默认策略来源**：
/// `.env` 被版本库忽略，新设备上必然缺失，以"部署时不存在的东西"作为安全开关是循环依赖。
pub const ALLOW_CPU_FALLBACK_ENV_KEY: &str = "ALLOW_CPU_FALLBACK";

/// 无硬件运行时的行为策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FallbackPolicy {
    /// 无硬件时降级为模拟会话（开发/调试；也是既有默认行为）
    #[default]
    Allow,
    /// 无硬件时必须返回错误，**不得**返回可用的模拟会话
    RequireHardware,
}

/// 会话的硬件可用性状态（成功构造域内的两态）
///
/// 第三态"需要硬件但不可用"由构造返回的 `Err(AlgoError::ModelLoad { .. })` 表达，
/// 该状态下不存在会话对象；经由 [`classify_session_availability`] 可归约为
/// [`HardwareAvailability::Unavailable`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareStatus {
    /// 真实硬件运行时已就绪
    Hardware,
    /// 开发调试模拟会话（未使用硬件）
    Simulated,
}

/// 会话构造结果的三态归约视图
///
/// 三态语义不同，不得合并为布尔：
/// - [`HardwareAvailability::Hardware`]：真实硬件可用；
/// - [`HardwareAvailability::Simulated`]：模拟降级；
/// - [`HardwareAvailability::Unavailable`]：需要硬件但不可用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareAvailability {
    /// 真实硬件运行时已就绪
    Hardware,
    /// 模拟会话（未使用硬件）
    Simulated,
    /// 要求真实硬件但不可用（构造失败）
    Unavailable,
}

/// 将"构造结果 + 会话状态"归约为可判别三态
///
/// 调用方典型用法：先 `match` 构造结果，再以本函数得到统一的三态标签
/// （例如写入自检报告或启动日志）。
pub fn classify_session_availability(
    result: &Result<HardwareStatus, AlgoError>,
) -> HardwareAvailability {
    match result {
        Ok(HardwareStatus::Hardware) => HardwareAvailability::Hardware,
        Ok(HardwareStatus::Simulated) => HardwareAvailability::Simulated,
        Err(_) => HardwareAvailability::Unavailable,
    }
}

/// 归一化平台代号，兼容历史冗长别名
///
/// 与宿主 `crates/infer/src/sandbox.rs::normalize_platform_id` 保持**最小必要重复**：
/// `algo-sdk` 不能反向依赖宿主 crate，因此这里只复刻别名表本身。
/// 两侧必须同步演进，一致性由 `platform_id_normalization_matches_host_contract`
/// 表驱动测试锁定（宿主侧对应 `crates/infer/tests/algo_sandbox_tests.rs`
/// 的 `test_platform_alias_table_stays_in_sync_with_algo_sdk_copy`）。
pub fn normalize_platform_id(id: &str) -> &str {
    match id {
        "macos-arm64" | "macos-arm64-coreml" | "darwin-arm64" => "macos-arm64",
        "linux-rknn" | "linux-arm64-rknn" | "rknn" => "linux-rknn",
        "linux-ascend" | "linux-arm64-ascend" | "ascend" => "linux-ascend",
        "linux-x64" | "generic-x86_64-cpu" | "linux-x86_64" => "linux-x64",
        other => other,
    }
}

/// 宿主平台是否强制要求真实硬件加速单元
///
/// 判据只有平台标识字符串，不含任何按目标 SoC / 宿主 OS 的 `cfg` 分支。
pub fn platform_requires_hardware(platform_id: &str) -> bool {
    matches!(
        normalize_platform_id(platform_id),
        "linux-rknn" | "linux-ascend"
    )
}

/// 解析无硬件时的回退策略
///
/// 优先级（高 → 低）：
/// 1. 安装自检模式 → [`FallbackPolicy::RequireHardware`]（自检不得用模拟会话冒充真实推理，
///    该硬门**不可**被 `.env` 或调用方显式声明覆盖）；
/// 2. 调用方显式声明（`explicit`，如本地 `run_local` 开发工具）→ 直接采纳；
/// 3. 算法包私有 `.env` 中 `ALLOW_CPU_FALLBACK=1` → [`FallbackPolicy::Allow`]
///    （运维在无硬件环境显式放开，用于本机跑通 rknn 包）；
/// 4. 宿主平台为硬件平台（见 [`platform_requires_hardware`]）→ [`FallbackPolicy::RequireHardware`]；
/// 5. 其他（macOS / x86 开发机）→ [`FallbackPolicy::Allow`]。
///
/// 注意：`.env` 缺失时（新设备上必然缺失）行为完全由平台标识与显式声明决定，
/// 不依赖任何部署期外部文件。
///
/// 生产路径（宿主经 C ABI 装载）恒传 `explicit == None`，因此策略仍只由平台与自检决定；
/// `explicit` 仅供本地开发工具在代码中声明"我要跑模拟回退"，不引入新的隐式开关。
pub fn resolve_fallback_policy(
    is_self_test: bool,
    platform_id: &str,
    env: Option<&PackageEnv>,
    explicit: Option<FallbackPolicy>,
) -> FallbackPolicy {
    if is_self_test {
        return FallbackPolicy::RequireHardware;
    }
    if let Some(policy) = explicit {
        return policy;
    }
    if env.is_some_and(|env| env.get_bool(ALLOW_CPU_FALLBACK_ENV_KEY) == Some(true)) {
        return FallbackPolicy::Allow;
    }
    if platform_requires_hardware(platform_id) {
        return FallbackPolicy::RequireHardware;
    }
    FallbackPolicy::Allow
}

/// 构造"要求硬件但不可用"的可定位错误
///
/// `RequireHardware` 的两条失败路径（已启用平台驱动但运行时加载失败、当前构建未启用任何
/// 硬件后端）必须给出一致且可操作的诊断，否则运维会看到两个形状不同的错误。
///
/// 错误变体固定为 [`AlgoError::ModelLoad`]（→ `AV_ERR_MODEL_LOAD_FAILED = -5`），
/// 零新增状态码、零 ABI 变更。
///
/// `cause` 为底层根因描述（动态库加载失败原因 / 未启用后端等）。
///
/// # 字段顺序受宿主读取缓冲约束（不可随意重排）
///
/// 宿主 `check_c_status` 用 **512 字节**栈缓冲读取 `last_error`
/// （`crates/infer/src/c_abi/loader.rs`），`copy_last_error` 按 `min(len, cap - 1)` 截断。
/// 而底层 `libloading` 的失败原因本身就可能超过 200 字节，叠加长 `package_root` 后整条消息
/// 轻易越过 512 字节。因此**操作指引必须排在最前**：位置越靠后越容易被截掉，
/// 一旦被截掉，运维看到的就只是"模型加载失败"，恰好退回本任务要消灭的不可诊断状态。
///
/// 顺序约定：结论 → 操作指引 → `package_root` → 模型路径 → 底层原因（最易截断，放最后）。
/// `hardware_unavailable_error_keeps_actionable_hint_within_host_error_buffer`
/// 与 `crates/algo-sdk/tests/fallback_policy_gate.rs` 共同锁定该顺序。
pub fn hardware_unavailable_error(
    package_root: &Path,
    model_path: &Path,
    cause: &str,
) -> AlgoError {
    AlgoError::ModelLoad {
        reason: format!(
            "要求硬件加速单元但不可用\n\
             - 请确认算法包 lib/ 下已随包携带平台运行时库（如 librknnrt.so），\
             且 ABI 与 CPU 架构匹配\n\
             - 如确需在本机显式运行模拟会话（禁止用于安装自检与生产部署），\
             请设置 {}=1\n\
             - package_root: {}\n\
             - 模型路径: {}\n\
             - 底层原因: {cause}",
            ALLOW_CPU_FALLBACK_ENV_KEY,
            package_root.display(),
            model_path.display(),
        ),
    }
}

/// 展开底层加载失败的原因为可直接嵌入的文本
///
/// `RknnRuntime::load` 失败时返回的已经是 [`AlgoError::ModelLoad`]，其 `Display` 自带
/// `模型加载失败: ` 前缀。若直接 `{e}` 插值，最终消息会变成
/// `模型加载失败: 要求硬件... （底层原因: 模型加载失败: 尝试加载 ...）`——
/// 既重复又会白耗宿主 512 字节读取窗口的宝贵配额（见 [`hardware_unavailable_error`]）。
/// 因此这里剥掉外层 `ModelLoad` 包装，只取内部 `reason`；非 `ModelLoad` 变体原样回退到
/// `Display`，不丢失信息。
pub fn describe_load_failure(error: &AlgoError) -> String {
    match error {
        AlgoError::ModelLoad { reason } => reason.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与宿主 `crates/infer/src/sandbox.rs::normalize_platform_id` 的全族一致性
    ///
    /// 这是本模块唯一的重复点，必须由表驱动测试显式锁定；两侧任一侧漂移都会在此失败。
    #[test]
    fn platform_id_normalization_matches_host_contract() {
        let cases = [
            ("macos-arm64", "macos-arm64"),
            ("macos-arm64-coreml", "macos-arm64"),
            ("darwin-arm64", "macos-arm64"),
            ("linux-rknn", "linux-rknn"),
            ("linux-arm64-rknn", "linux-rknn"),
            ("rknn", "linux-rknn"),
            ("linux-ascend", "linux-ascend"),
            ("linux-arm64-ascend", "linux-ascend"),
            ("ascend", "linux-ascend"),
            ("linux-x64", "linux-x64"),
            ("generic-x86_64-cpu", "linux-x64"),
            ("linux-x86_64", "linux-x64"),
            ("unknown-platform", "unknown-platform"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                normalize_platform_id(input),
                expected,
                "平台别名归一化与宿主契约不一致: {input}"
            );
        }
    }

    #[test]
    fn hardware_platforms_require_hardware_across_alias_family() {
        for id in [
            "linux-rknn",
            "linux-arm64-rknn",
            "rknn",
            "linux-ascend",
            "linux-arm64-ascend",
            "ascend",
        ] {
            assert!(
                platform_requires_hardware(id),
                "硬件平台 {id} 必须要求真实硬件"
            );
        }

        for id in [
            "macos-arm64",
            "macos-arm64-coreml",
            "darwin-arm64",
            "linux-x64",
            "generic-x86_64-cpu",
            "test-platform",
        ] {
            assert!(
                !platform_requires_hardware(id),
                "非硬件平台 {id} 必须保留无驱动回退能力"
            );
        }
    }

    #[test]
    fn self_test_always_requires_hardware_regardless_of_platform() {
        for id in ["macos-arm64", "linux-rknn", "linux-ascend", "test-platform"] {
            assert_eq!(
                resolve_fallback_policy(true, id, None, None),
                FallbackPolicy::RequireHardware,
                "安装自检模式在 {id} 上必须强制真实硬件"
            );
        }
    }

    #[test]
    fn self_test_gate_cannot_be_overridden_by_env() {
        let env = PackageEnv::parse_str("ALLOW_CPU_FALLBACK=1");
        assert_eq!(
            resolve_fallback_policy(true, "linux-rknn", Some(&env), None),
            FallbackPolicy::RequireHardware,
            "自检硬门必须优先于 .env 显式覆盖"
        );
    }

    /// 回归锁：自检硬门**不可**被调用方显式声明翻越
    ///
    /// 若本地开发工具（或任何调用方）能通过 `explicit` 把自检模式的策略改成 `Allow`，
    /// 安装自检就又可以用模拟会话冒充真实推理——本任务要消灭的失败模式会原封不动地回来。
    #[test]
    fn self_test_gate_cannot_be_overridden_by_explicit_declaration() {
        for id in ["linux-rknn", "macos-arm64", "test-platform"] {
            assert_eq!(
                resolve_fallback_policy(true, id, None, Some(FallbackPolicy::Allow)),
                FallbackPolicy::RequireHardware,
                "自检模式下 `explicit = Allow` 必须被忽略（{id}）"
            );
        }
        // `.env` 与显式声明同时存在也不得翻越
        let env = PackageEnv::parse_str("ALLOW_CPU_FALLBACK=1");
        assert_eq!(
            resolve_fallback_policy(true, "linux-rknn", Some(&env), Some(FallbackPolicy::Allow)),
            FallbackPolicy::RequireHardware,
            "自检硬门必须同时优先于 .env 与显式声明"
        );
    }

    /// 本地开发工具：显式声明 `Allow` 可在硬件平台上跑模拟回退
    ///
    /// 这是 `run_local` 不再依赖 `ALLOW_CPU_FALLBACK` 环境变量的依据：
    /// 意图写在代码里，而不是部署期外部文件里。
    #[test]
    fn explicit_declaration_allows_simulation_on_hardware_platform() {
        assert_eq!(
            resolve_fallback_policy(false, "linux-rknn", None, Some(FallbackPolicy::Allow)),
            FallbackPolicy::Allow,
            "显式声明必须覆盖平台默认的 RequireHardware"
        );
        // 反向：显式声明 RequireHardware 在非硬件平台上也生效（加法式对称性）
        assert_eq!(
            resolve_fallback_policy(
                false,
                "macos-arm64",
                None,
                Some(FallbackPolicy::RequireHardware)
            ),
            FallbackPolicy::RequireHardware,
            "显式声明 RequireHardware 必须在非硬件平台上生效"
        );
    }

    /// 显式声明优先于 `.env`（代码意图强于部署期文件）
    #[test]
    fn explicit_declaration_takes_precedence_over_env() {
        let env = PackageEnv::parse_str("ALLOW_CPU_FALLBACK=1");
        assert_eq!(
            resolve_fallback_policy(
                false,
                "macos-arm64",
                Some(&env),
                Some(FallbackPolicy::RequireHardware)
            ),
            FallbackPolicy::RequireHardware,
            "显式声明的 RequireHardware 不得被 .env 的 Allow 覆盖"
        );
    }

    #[test]
    fn non_hardware_platform_defaults_to_allow_without_env() {
        assert_eq!(
            resolve_fallback_policy(false, "macos-arm64", None, None),
            FallbackPolicy::Allow
        );
        assert_eq!(
            resolve_fallback_policy(false, "test-platform", None, None),
            FallbackPolicy::Allow
        );
    }

    #[test]
    fn hardware_platform_requires_hardware_without_env() {
        assert_eq!(
            resolve_fallback_policy(false, "linux-rknn", None, None),
            FallbackPolicy::RequireHardware
        );
        // `.env` 缺失（新设备）时行为不得依赖部署期外部文件
        assert_eq!(
            resolve_fallback_policy(false, "rknn", Some(&PackageEnv::default()), None),
            FallbackPolicy::RequireHardware
        );
    }

    #[test]
    fn env_explicitly_allows_cpu_fallback_on_hardware_platform() {
        let env = PackageEnv::parse_str("ALLOW_CPU_FALLBACK=1");
        assert_eq!(
            resolve_fallback_policy(false, "linux-rknn", Some(&env), None),
            FallbackPolicy::Allow,
            "运维显式覆盖必须生效，否则错误信息中的操作指引是假的"
        );

        // 非真值不放开
        for raw in ["ALLOW_CPU_FALLBACK=0", "ALLOW_CPU_FALLBACK=no"] {
            let env = PackageEnv::parse_str(raw);
            assert_eq!(
                resolve_fallback_policy(false, "linux-rknn", Some(&env), None),
                FallbackPolicy::RequireHardware,
                "{raw} 不得放开回退"
            );
        }
    }

    #[test]
    fn allow_is_the_default_policy() {
        assert_eq!(FallbackPolicy::default(), FallbackPolicy::Allow);
    }

    /// 回归锁：底层 `ModelLoad` 的 `Display` 前缀不得被重复嵌套
    ///
    /// 宿主只读 512 字节，重复的 `模型加载失败: ` 直接消耗配额，且让消息读起来像两个并列错误。
    #[test]
    fn describe_load_failure_strips_redundant_model_load_prefix() {
        let wrapped = AlgoError::ModelLoad {
            reason: "尝试加载 \"/usr/local/lib/librknnrt.so\" 失败".to_string(),
        };
        assert_eq!(
            describe_load_failure(&wrapped),
            "尝试加载 \"/usr/local/lib/librknnrt.so\" 失败"
        );
        assert!(
            !describe_load_failure(&wrapped).contains("模型加载失败"),
            "不得把 Display 自带的 `模型加载失败: ` 前缀带进最终消息"
        );

        // 非 ModelLoad 变体必须保留原有可读信息，不得被静默丢成空串
        let other = AlgoError::Internal {
            reason: "底层异常".to_string(),
        };
        assert_eq!(describe_load_failure(&other), other.to_string());
    }

    /// 回归锁：错误信息必须在**宿主 512 字节读取窗口**内保留全部可操作信息
    ///
    /// 宿主 `check_c_status` 用 512 字节栈缓冲读取 `last_error`
    /// （`crates/infer/src/c_abi/loader.rs`），超出部分被 `copy_last_error` 截断。
    /// 底层 `libloading` 的失败原因（含候选路径与 dlopen 报错）本身就接近或超过 200 字节，
    /// 一旦指引被排到 512 字节之后，运维看到的就是不可诊断的"模型加载失败"。
    ///
    /// 本用例用与宿主完全相同的截断规则断言三个关键定位项均落在前 511 字节内，
    /// 并覆盖极长 `package_root`（临时目录真实长度量级）。
    #[test]
    fn hardware_unavailable_error_keeps_actionable_hint_within_host_error_buffer() {
        /// 宿主缓冲：`crates/infer/src/c_abi/loader.rs` 的 `[0 as c_char; 512]`
        const HOST_ERROR_BUFFER: usize = 512;

        let roots = [
            ".",
            "/root/.heimdall/algo/rk3568/fire-detections/1.0.0",
            "/private/var/folders/9z/k9t8q4sd2xk_T/-Tmp-/algo_fallback_gate_11111111-2222-3333-4444-555555555555",
        ];
        // 复刻 `RknnRuntime::load` 在无 librknnrt 时交给本函数的真实底层原因长度。
        let cause = "无法加载平台运行时 librknnrt.so（底层失败原因: 尝试加载 \"/usr/local/lib/librknnrt.so\" 失败: dlopen(/usr/local/lib/librknnrt.so, 0x0005): tried: '/usr/local/lib/librknnrt.so' (no such file)）";

        for root in roots {
            let root = Path::new(root);
            let model_path = root.join("model/best_pure.rknn");
            let message = hardware_unavailable_error(root, &model_path, cause).to_string();
            let bytes = message.as_bytes();
            // 与 `copy_last_error` 一致：最多拷贝 `cap - 1` 字节，且按 UTF-8 边界可丢失尾字符。
            let visible = String::from_utf8_lossy(&bytes[..bytes.len().min(HOST_ERROR_BUFFER - 1)]);

            for needle in ["librknnrt", "package_root", ALLOW_CPU_FALLBACK_ENV_KEY] {
                assert!(
                    visible.contains(needle),
                    "`{needle}` 必须落在宿主 {HOST_ERROR_BUFFER} 字节读取窗口内，\
                     否则运维看不到可操作指引（root={root:?}, 全消息 {} 字节）",
                    bytes.len()
                );
            }
        }
    }

    #[test]
    fn availability_classifier_covers_three_states() {
        let hardware: Result<HardwareStatus, AlgoError> = Ok(HardwareStatus::Hardware);
        assert_eq!(
            classify_session_availability(&hardware),
            HardwareAvailability::Hardware
        );

        let simulated: Result<HardwareStatus, AlgoError> = Ok(HardwareStatus::Simulated);
        assert_eq!(
            classify_session_availability(&simulated),
            HardwareAvailability::Simulated
        );

        let unavailable: Result<HardwareStatus, AlgoError> = Err(AlgoError::ModelLoad {
            reason: "无硬件".to_string(),
        });
        assert_eq!(
            classify_session_availability(&unavailable),
            HardwareAvailability::Unavailable
        );
    }
}
