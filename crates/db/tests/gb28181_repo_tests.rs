use db::{init_test_db, Gb28181DeviceRepo, SysGb28181ConfigRepo};
use types::{Gb28181ChannelDto, UpdateGb28181ConfigRequest};

#[tokio::test]
async fn test_gb28181_config_and_device_repos() {
    let db = init_test_db().await.expect("init test db");

    // 1. Get default config
    let cfg = SysGb28181ConfigRepo::get(&db).await.expect("get config");
    assert_eq!(cfg.sip_id, "34020000002000000001");
    assert_eq!(cfg.sip_port, 5060);

    // 2. Update config
    let updated = SysGb28181ConfigRepo::update(
        &db,
        UpdateGb28181ConfigRequest {
            sip_id: Some("34020000002000000002".to_string()),
            sip_domain: None,
            sip_port: Some(5061),
            sip_password: Some("secret123".to_string()),
            rtp_port_range_start: None,
            rtp_port_range_end: None,
            auto_catalog_sync: Some(false),
            heartbeat_timeout_sec: Some(120),
        },
    )
    .await
    .expect("update config");

    assert_eq!(updated.sip_id, "34020000002000000002");
    assert_eq!(updated.sip_port, 5061);
    assert_eq!(updated.sip_password, "secret123");
    assert!(!updated.auto_catalog_sync);
    assert_eq!(updated.heartbeat_timeout_sec, 120);

    // 3. Upsert device
    let dev = Gb28181DeviceRepo::upsert_device(
        &db,
        "34020000001180000001",
        "Hikvision NVR",
        "192.168.1.50",
        5060,
        "udp",
        "online",
    )
    .await
    .expect("upsert device");
    assert_eq!(dev.device_id, "34020000001180000001");
    assert_eq!(dev.status, "online");

    // 4. Batch upsert channels
    let channels = vec![
        Gb28181ChannelDto {
            device_id: "34020000001180000001".to_string(),
            channel_id: "34020000001310000001".to_string(),
            name: "Camera 1".to_string(),
            manufacturer: "Hikvision".to_string(),
            model: "DS-2CD2047G2".to_string(),
            status: "ON".to_string(),
            parent_id: "34020000001180000001".to_string(),
            sub_stream_supported: true,
            last_seen_ms: 1000,
            is_imported: false,
            camera_id: None,
        },
        Gb28181ChannelDto {
            device_id: "34020000001180000001".to_string(),
            channel_id: "34020000001310000002".to_string(),
            name: "Camera 2".to_string(),
            manufacturer: "Hikvision".to_string(),
            model: "DS-2CD2047G2".to_string(),
            status: "ON".to_string(),
            parent_id: "34020000001180000001".to_string(),
            sub_stream_supported: true,
            last_seen_ms: 1000,
            is_imported: false,
            camera_id: None,
        },
    ];
    Gb28181DeviceRepo::batch_upsert_channels(&db, "34020000001180000001", &channels)
        .await
        .expect("batch upsert channels");

    // 5. List devices with channels
    let list = Gb28181DeviceRepo::list_devices_with_channels(&db)
        .await
        .expect("list devices");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].channels.len(), 2);
    assert_eq!(list[0].channel_count, 2);
    assert!(!list[0].channels[0].is_imported);
}
