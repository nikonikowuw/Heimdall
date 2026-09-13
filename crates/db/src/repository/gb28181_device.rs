use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel,
    PaginatorTrait, QueryFilter, QueryOrder, Set, TransactionTrait,
};

use crate::entity::camera::Entity as CameraEntity;
use crate::entity::gb28181_channel::{
    ActiveModel as ChannelActiveModel, Column as ChannelColumn, Entity as ChannelEntity,
};
use crate::entity::gb28181_device::{
    ActiveModel as DeviceActiveModel, Column as DeviceColumn, Entity as DeviceEntity,
    Model as DeviceModel,
};
use crate::error::DbError;

#[derive(Debug)]
pub struct Gb28181DeviceRepo;

impl Gb28181DeviceRepo {
    /// 查找单个设备
    pub async fn find_by_device_id(
        db: &DatabaseConnection,
        device_id: &str,
    ) -> Result<Option<DeviceModel>, DbError> {
        DeviceEntity::find_by_id(device_id)
            .one(db)
            .await
            .map_err(DbError::from)
    }

    /// 注册或更新设备信息 (Keepalive / Register)
    pub async fn upsert_device(
        db: &DatabaseConnection,
        device_id: &str,
        name: &str,
        ip_addr: &str,
        sip_port: u16,
        transport: &str,
        status: &str,
    ) -> Result<DeviceModel, DbError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let existing = DeviceEntity::find_by_id(device_id)
            .one(db)
            .await
            .map_err(DbError::from)?;

        if let Some(dev) = existing {
            let mut active = dev.into_active_model();
            if !name.is_empty() {
                active.name = Set(name.to_string());
            }
            active.ip_addr = Set(ip_addr.to_string());
            active.sip_port = Set(sip_port as i32);
            active.transport = Set(transport.to_string());
            active.status = Set(status.to_string());
            active.last_keepalive_ms = Set(now_ms);
            active.updated_at_ms = Set(now_ms);
            active.update(db).await.map_err(DbError::from)
        } else {
            let active = DeviceActiveModel {
                device_id: Set(device_id.to_string()),
                name: Set(if name.is_empty() {
                    format!(
                        "GB-Device-{}",
                        &device_id[device_id.len().saturating_sub(6)..]
                    )
                } else {
                    name.to_string()
                }),
                ip_addr: Set(ip_addr.to_string()),
                sip_port: Set(sip_port as i32),
                transport: Set(transport.to_string()),
                status: Set(status.to_string()),
                channel_count: Set(0),
                last_keepalive_ms: Set(now_ms),
                created_at_ms: Set(now_ms),
                updated_at_ms: Set(now_ms),
            };
            active.insert(db).await.map_err(DbError::from)
        }
    }

    /// 更新设备心跳
    pub async fn record_keepalive(
        db: &DatabaseConnection,
        device_id: &str,
    ) -> Result<Option<DeviceModel>, DbError> {
        let existing = DeviceEntity::find_by_id(device_id)
            .one(db)
            .await
            .map_err(DbError::from)?;

        if let Some(dev) = existing {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let mut active = dev.into_active_model();
            active.status = Set("online".to_string());
            active.last_keepalive_ms = Set(now_ms);
            active.updated_at_ms = Set(now_ms);
            let updated = active.update(db).await.map_err(DbError::from)?;
            Ok(Some(updated))
        } else {
            Ok(None)
        }
    }

    /// 标记超时设备为 offline
    pub async fn mark_stale_devices_offline(
        db: &DatabaseConnection,
        threshold_ms: i64,
    ) -> Result<u64, DbError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let stale_devices = DeviceEntity::find()
            .filter(DeviceColumn::Status.eq("online"))
            .filter(DeviceColumn::LastKeepaliveMs.lt(threshold_ms))
            .all(db)
            .await
            .map_err(DbError::from)?;

        let count = stale_devices.len() as u64;
        for dev in stale_devices {
            let mut active = dev.into_active_model();
            active.status = Set("offline".to_string());
            active.updated_at_ms = Set(now_ms);
            let _ = active.update(db).await;
        }
        Ok(count)
    }

    /// 批量同步 Catalog 通道列表 (单一 SQLite 事务内原子执行)
    pub async fn batch_upsert_channels(
        db: &DatabaseConnection,
        device_id: &str,
        channels: &[types::Gb28181ChannelDto],
    ) -> Result<(), DbError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let dev_id_owned = device_id.to_string();
        let channels_owned = channels.to_vec();

        db.transaction::<_, (), DbError>(|txn| {
            Box::pin(async move {
                for ch in &channels_owned {
                    let existing = ChannelEntity::find()
                        .filter(ChannelColumn::DeviceId.eq(&dev_id_owned))
                        .filter(ChannelColumn::ChannelId.eq(&ch.channel_id))
                        .one(txn)
                        .await
                        .map_err(DbError::from)?;

                    if let Some(record) = existing {
                        let mut active = record.into_active_model();
                        if !ch.name.is_empty() {
                            active.name = Set(ch.name.clone());
                        }
                        active.manufacturer = Set(ch.manufacturer.clone());
                        active.model = Set(ch.model.clone());
                        active.status = Set(ch.status.clone());
                        active.parent_id = Set(ch.parent_id.clone());
                        active.sub_stream_supported =
                            Set(if ch.sub_stream_supported { 1 } else { 0 });
                        active.last_seen_ms = Set(now_ms);
                        let _ = active.update(txn).await.map_err(DbError::from)?;
                    } else {
                        let active = ChannelActiveModel {
                            device_id: Set(dev_id_owned.clone()),
                            channel_id: Set(ch.channel_id.clone()),
                            name: Set(ch.name.clone()),
                            manufacturer: Set(ch.manufacturer.clone()),
                            model: Set(ch.model.clone()),
                            status: Set(ch.status.clone()),
                            parent_id: Set(ch.parent_id.clone()),
                            sub_stream_supported: Set(if ch.sub_stream_supported { 1 } else { 0 }),
                            last_seen_ms: Set(now_ms),
                        };
                        let _ = active.insert(txn).await.map_err(DbError::from)?;
                    }
                }

                // 更新设备的通道总数
                let total_channels = ChannelEntity::find()
                    .filter(ChannelColumn::DeviceId.eq(&dev_id_owned))
                    .count(txn)
                    .await
                    .map_err(DbError::from)?;

                if let Some(dev) = DeviceEntity::find_by_id(&dev_id_owned)
                    .one(txn)
                    .await
                    .map_err(DbError::from)?
                {
                    let mut active = dev.into_active_model();
                    active.channel_count = Set(total_channels as i32);
                    active.updated_at_ms = Set(now_ms);
                    let _ = active.update(txn).await.map_err(DbError::from)?;
                }

                Ok(())
            })
        })
        .await
        .map_err(|e| match e {
            sea_orm::TransactionError::Connection(c) => DbError::from(c),
            sea_orm::TransactionError::Transaction(t) => t,
        })
    }

    /// 列出所有已注册设备及其通道，并关联 cameras 表标记已纳管状态
    pub async fn list_devices_with_channels(
        db: &DatabaseConnection,
    ) -> Result<Vec<types::Gb28181DeviceDto>, DbError> {
        let devices = DeviceEntity::find()
            .order_by_desc(DeviceColumn::LastKeepaliveMs)
            .all(db)
            .await
            .map_err(DbError::from)?;

        let all_channels = ChannelEntity::find()
            .order_by_asc(ChannelColumn::ChannelId)
            .all(db)
            .await
            .map_err(DbError::from)?;

        let all_cameras = CameraEntity::find().all(db).await.map_err(DbError::from)?;

        let mut result = Vec::with_capacity(devices.len());
        for dev in devices {
            let dev_channels: Vec<types::Gb28181ChannelDto> = all_channels
                .iter()
                .filter(|c| c.device_id == dev.device_id)
                .map(|c| {
                    let matching_cam = all_cameras.iter().find(|cam| {
                        cam.gb28181_device_id.as_deref() == Some(&c.device_id)
                            && cam.gb28181_channel_id.as_deref() == Some(&c.channel_id)
                    });
                    types::Gb28181ChannelDto {
                        device_id: c.device_id.clone(),
                        channel_id: c.channel_id.clone(),
                        name: c.name.clone(),
                        manufacturer: c.manufacturer.clone(),
                        model: c.model.clone(),
                        status: c.status.clone(),
                        parent_id: c.parent_id.clone(),
                        sub_stream_supported: c.sub_stream_supported != 0,
                        last_seen_ms: c.last_seen_ms,
                        is_imported: matching_cam.is_some(),
                        camera_id: matching_cam.map(|cam| cam.camera_id.clone()),
                    }
                })
                .collect();

            result.push(types::Gb28181DeviceDto {
                device_id: dev.device_id,
                name: dev.name,
                ip_addr: dev.ip_addr,
                sip_port: dev.sip_port as u16,
                transport: dev.transport,
                status: dev.status,
                channel_count: dev_channels.len() as u32,
                last_keepalive_ms: dev.last_keepalive_ms,
                created_at_ms: dev.created_at_ms,
                updated_at_ms: dev.updated_at_ms,
                channels: dev_channels,
            });
        }

        Ok(result)
    }
}
