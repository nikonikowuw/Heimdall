//! 人员批量导入
//!
//! 归档上传 → 自适应解析（清单 / 约定）→ 串行提取录入 → 终态结构化报告。
//! 每名候选独立原子落库，单点失败不阻断整批；进度通过状态机与有界 WS 广播对外可见。

pub mod candidate;
pub mod manager;

pub use candidate::parse_candidates;
pub use manager::{PersonnelImportManager, TempImportSandbox};
