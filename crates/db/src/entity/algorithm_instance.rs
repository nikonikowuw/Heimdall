use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "algorithm_instances")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(unique, column_type = "Text")]
    pub instance_id: String,
    pub task_id: i64,
    pub camera_id: String,
    pub algorithm_id: String,
    pub analysis_fps: i32,
    #[sea_orm(column_type = "Text")]
    pub params_json: String,
    #[sea_orm(column_type = "Text")]
    pub rules_json: String,
    #[sea_orm(column_type = "Text")]
    pub motion_gate_json: String,
    pub enabled: bool,
    pub actual_status: i32,
    pub status_message: String,
    /// 用户期望配置代际，每次实例级 Apply 提交期望配置时递增
    pub desired_revision: i64,
    /// 运行时确认目标 Worker 已生效的配置代际；与 `desired_revision` 相等即已收敛
    pub applied_revision: i64,
    /// 运行时配置应用状态：0=applied / 1=pending / 2=failed
    pub runtime_apply_state: i32,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl Model {
    /// 解析运行时配置应用状态；未知取值回退为 `failed`（宁可显式暴露异常，也不假装已生效）
    pub fn runtime_apply_state(&self) -> types::InstanceApplyState {
        types::InstanceApplyState::from_i32(self.runtime_apply_state)
            .unwrap_or(types::InstanceApplyState::Failed)
    }
}
