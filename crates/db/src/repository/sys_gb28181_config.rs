use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, IntoActiveModel, Set};

use crate::entity::sys_gb28181_config::{ActiveModel, Entity, Model};
use crate::error::DbError;

#[derive(Debug)]
pub struct SysGb28181ConfigRepo;

impl SysGb28181ConfigRepo {
    /// 获取当前 GB28181 系统配置（单例 id = 1）
    pub async fn get(db: &DatabaseConnection) -> Result<types::SysGb28181Config, DbError> {
        if let Some(model) = Entity::find_by_id(1).one(db).await.map_err(DbError::from)? {
            Ok(model.into())
        } else {
            let default_cfg = types::SysGb28181Config::default();
            let now_ms = chrono::Utc::now().timestamp_millis();
            let active = ActiveModel {
                id: Set(1),
                sip_id: Set(default_cfg.sip_id.clone()),
                sip_domain: Set(default_cfg.sip_domain.clone()),
                sip_port: Set(default_cfg.sip_port as i32),
                sip_password: Set(default_cfg.sip_password.clone()),
                rtp_port_range_start: Set(default_cfg.rtp_port_range_start as i32),
                rtp_port_range_end: Set(default_cfg.rtp_port_range_end as i32),
                auto_catalog_sync: Set(if default_cfg.auto_catalog_sync { 1 } else { 0 }),
                heartbeat_timeout_sec: Set(default_cfg.heartbeat_timeout_sec as i32),
                updated_at_ms: Set(now_ms),
            };
            let inserted = active.insert(db).await.map_err(DbError::from)?;
            Ok(inserted.into())
        }
    }

    /// 更新 GB28181 系统配置
    pub async fn update(
        db: &DatabaseConnection,
        req: types::UpdateGb28181ConfigRequest,
    ) -> Result<types::SysGb28181Config, DbError> {
        let existing = Entity::find_by_id(1)
            .one(db)
            .await
            .map_err(DbError::from)?
            .unwrap_or_else(|| Model {
                id: 1,
                sip_id: "34020000002000000001".to_string(),
                sip_domain: "3402000000".to_string(),
                sip_port: 5060,
                sip_password: "admin123".to_string(),
                rtp_port_range_start: 30000,
                rtp_port_range_end: 30500,
                auto_catalog_sync: 1,
                heartbeat_timeout_sec: 180,
                updated_at_ms: 0,
            });

        let mut active = existing.into_active_model();
        if let Some(sip_id) = req.sip_id {
            active.sip_id = Set(sip_id);
        }
        if let Some(sip_domain) = req.sip_domain {
            active.sip_domain = Set(sip_domain);
        }
        if let Some(sip_port) = req.sip_port {
            active.sip_port = Set(sip_port as i32);
        }
        if let Some(sip_password) = req.sip_password {
            active.sip_password = Set(sip_password);
        }
        if let Some(rtp_start) = req.rtp_port_range_start {
            active.rtp_port_range_start = Set(rtp_start as i32);
        }
        if let Some(rtp_end) = req.rtp_port_range_end {
            active.rtp_port_range_end = Set(rtp_end as i32);
        }
        if let Some(auto_sync) = req.auto_catalog_sync {
            active.auto_catalog_sync = Set(if auto_sync { 1 } else { 0 });
        }
        if let Some(hb_timeout) = req.heartbeat_timeout_sec {
            active.heartbeat_timeout_sec = Set(hb_timeout as i32);
        }
        active.updated_at_ms = Set(chrono::Utc::now().timestamp_millis());

        let updated = active.update(db).await.map_err(DbError::from)?;
        Ok(updated.into())
    }
}
