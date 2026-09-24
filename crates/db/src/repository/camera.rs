use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    Set,
};

use crate::entity::camera::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

/// 探活结果更新参数
#[derive(Debug, Clone, Copy)]
pub struct ProbeUpdateParams<'a> {
    pub status: &'a str,
    pub codec: &'a str,
    pub width: i32,
    pub height: i32,
    pub fps: f64,
    pub error_code: &'a str,
}

#[derive(Debug, Clone)]
pub struct ProbeStatusSnapshot {
    pub status: String,
}

#[derive(Debug)]
pub struct CameraRepo;

impl CameraRepo {
    pub async fn list_all(db: &DatabaseConnection) -> Result<Vec<Model>, DbError> {
        let mut list = Entity::find().all(db).await.map_err(DbError::from)?;
        let rank = |status: &str| {
            types::ProbeStatus::parse(status)
                .map(|status| status.priority())
                .unwrap_or(5)
        };
        list.sort_by(|a, b| {
            rank(&a.last_probe_status)
                .cmp(&rank(&b.last_probe_status))
                .then_with(|| {
                    b.last_success_at
                        .cmp(&a.last_success_at)
                        .then_with(|| b.id.cmp(&a.id))
                })
        });
        Ok(list)
    }

    pub async fn find_by_camera_id(
        db: &DatabaseConnection,
        camera_id: &str,
    ) -> Result<Option<Model>, DbError> {
        Entity::find()
            .filter(Column::CameraId.eq(camera_id))
            .one(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn insert(
        db: &DatabaseConnection,
        active_model: ActiveModel,
    ) -> Result<Model, DbError> {
        active_model.insert(db).await.map_err(DbError::from)
    }

    pub async fn delete_by_camera_id(
        db: &DatabaseConnection,
        camera_id: &str,
    ) -> Result<u64, DbError> {
        let res = Entity::delete_many()
            .filter(Column::CameraId.eq(camera_id))
            .exec(db)
            .await?;
        Ok(res.rows_affected)
    }

    pub async fn count_all(db: &DatabaseConnection) -> Result<u64, DbError> {
        let count = Entity::find().count(db).await.map_err(DbError::from)?;
        Ok(count)
    }

    pub async fn count_healthy(db: &DatabaseConnection) -> Result<u64, DbError> {
        let count = Entity::find()
            .filter(Column::LastProbeStatus.eq("healthy"))
            .count(db)
            .await
            .map_err(DbError::from)?;
        Ok(count)
    }

    pub async fn update_probe_status(
        db: &DatabaseConnection,
        camera_id: &str,
        params: ProbeUpdateParams<'_>,
    ) -> Result<Option<ProbeStatusSnapshot>, DbError> {
        loop {
            let Some(model) = Self::find_by_camera_id(db, camera_id).await? else {
                return Ok(None);
            };
            let previous_status = model.last_probe_status;
            if Self::try_update_probe_status_if_current(db, camera_id, &previous_status, params)
                .await?
            {
                return Ok(Some(ProbeStatusSnapshot {
                    status: previous_status,
                }));
            }
        }
    }

    async fn try_update_probe_status_if_current(
        db: &DatabaseConnection,
        camera_id: &str,
        expected_status: &str,
        params: ProbeUpdateParams<'_>,
    ) -> Result<bool, DbError> {
        let now = chrono::Utc::now();
        let last_success_at =
            if types::ProbeStatus::parse(params.status) == Some(types::ProbeStatus::Healthy) {
                Set(Some(now))
            } else {
                sea_orm::ActiveValue::NotSet
            };
        let active = ActiveModel {
            last_probe_status: Set(params.status.to_string()),
            last_probe_at: Set(Some(now)),
            last_codec: Set(params.codec.to_string()),
            last_width: Set(params.width),
            last_height: Set(params.height),
            last_fps: Set(params.fps),
            last_probe_error_code: Set(params.error_code.to_string()),
            last_success_at,
            updated_at: Set(now),
            ..Default::default()
        };

        let result = Entity::update_many()
            .set(active)
            .filter(Column::CameraId.eq(camera_id))
            .filter(Column::LastProbeStatus.eq(expected_status))
            .exec(db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub async fn update_stream_mode(
        db: &DatabaseConnection,
        camera_id: &str,
        stream_mode: &str,
    ) -> Result<Option<Model>, DbError> {
        if let Some(model) = Self::find_by_camera_id(db, camera_id).await? {
            if model.stream_mode == stream_mode {
                return Ok(Some(model));
            }
            let mut active: ActiveModel = model.into();
            active.stream_mode = Set(stream_mode.to_string());
            active.updated_at = Set(chrono::Utc::now());
            let updated = active.update(db).await?;
            Ok(Some(updated))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{ActiveModel, CameraRepo, ProbeUpdateParams};
    use sea_orm::Set;

    #[tokio::test]
    async fn stale_probe_update_cannot_overwrite_a_transition() {
        let db = crate::init_test_db().await.unwrap();
        CameraRepo::insert(
            &db,
            ActiveModel {
                camera_id: Set("CAM-CAS".to_string()),
                name: Set("CAS test camera".to_string()),
                rtsp_url: Set("rtsp://127.0.0.1/live".to_string()),
                last_probe_status: Set("healthy".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let params = ProbeUpdateParams {
            status: "failed",
            codec: "",
            width: 0,
            height: 0,
            fps: 0.0,
            error_code: "timeout",
        };
        assert!(
            CameraRepo::try_update_probe_status_if_current(&db, "CAM-CAS", "healthy", params)
                .await
                .unwrap()
        );
        assert!(
            !CameraRepo::try_update_probe_status_if_current(&db, "CAM-CAS", "healthy", params)
                .await
                .unwrap()
        );

        let previous = CameraRepo::update_probe_status(&db, "CAM-CAS", params)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(previous.status, "failed");
        assert_eq!(
            CameraRepo::find_by_camera_id(&db, "CAM-CAS")
                .await
                .unwrap()
                .unwrap()
                .last_probe_status,
            "failed"
        );
    }
}
