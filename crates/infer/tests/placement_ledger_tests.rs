//! NPU 放置求解、双层账本与状态机集成测试 (T06–T12, T19)

use infer::npu::{DeviceInventory, ExecutionState, LedgerConfig, PlacementError, PlacementLedger};
use types::placement::AffinityIntent;

#[test]
fn test_reservation_cancel_before_start_releases_budget_t06() {
    let inv = DeviceInventory::mock_rk3588();
    let ledger = PlacementLedger::new(inv, LedgerConfig::default());

    let intent = AffinityIntent::Auto {
        policy: "spread".to_string(),
    };
    let (res, _wire, needs_init) = ledger
        .reserve_placement(
            "cam-1",
            "grp-1",
            &intent,
            &["yolov8n".to_string()],
            1,
            false,
        )
        .expect("预留应成功");

    assert!(needs_init);
    assert_eq!(res.decision.core_mask, 1);
    assert_eq!(ledger.active_reservations_count(), 1);

    // 在工作线程启动前取消，执行释放
    let released = ledger.release_placement(res.id).expect("释放应成功");
    assert_eq!(released.id, res.id);
    assert_eq!(ledger.active_reservations_count(), 0);

    // 再次释放同一 reservation 应失败（幂等防重复扣除）
    let err = ledger
        .release_placement(res.id)
        .expect_err("重复释放应报错");
    assert!(matches!(err, PlacementError::ReservationNotFound(_)));
}

#[test]
fn test_weight_loading_future_cancellation_retains_owner_t07() {
    let inv = DeviceInventory::mock_rk3588();
    let ledger = PlacementLedger::new(inv, LedgerConfig::default());

    let intent = AffinityIntent::Auto {
        policy: "spread".to_string(),
    };

    // 实例 1 预留
    let (res1, _wire1, init1) = ledger
        .reserve_placement(
            "cam-1",
            "grp-1",
            &intent,
            &["yolov8n".to_string()],
            1,
            false,
        )
        .expect("实例 1 预留应成功");
    assert!(init1, "首个实例应触发权重初始化");

    // 实例 2 预留同一模型
    let (res2, _wire2, init2) = ledger
        .reserve_placement(
            "cam-2",
            "grp-2",
            &intent,
            &["yolov8n".to_string()],
            1,
            false,
        )
        .expect("实例 2 预留应成功");
    assert!(!init2, "同模型第二个实例不应重复初始化");

    // 模拟实例 1 取消并释放
    ledger
        .release_placement(res1.id)
        .expect("释放实例 1 应成功");

    // 断言实例 2 依然有效持有权重，不受实例 1 取消的影响
    let query2 = ledger
        .get_reservation(res2.id)
        .expect("实例 2 必须依然存在");
    assert_eq!(query2.id, res2.id);
    assert_eq!(ledger.active_reservations_count(), 1);
}

#[test]
fn test_concurrent_auto_spread_round_robin_t08() {
    let inv = DeviceInventory::mock_rk3588(); // 3 核心: core 0, 1, 2
    let config = LedgerConfig {
        max_instances_per_core: 2,
        ..Default::default()
    };
    let ledger = PlacementLedger::new(inv, config);

    let intent = AffinityIntent::Auto {
        policy: "spread".to_string(),
    };

    // 连续预留 3 个实例，应依次轮转分配到 3 个核心
    let (r1, _, _) = ledger
        .reserve_placement("cam-1", "grp-1", &intent, &[], 1, false)
        .expect("r1 成功");
    let (r2, _, _) = ledger
        .reserve_placement("cam-2", "grp-2", &intent, &[], 1, false)
        .expect("r2 成功");
    let (r3, _, _) = ledger
        .reserve_placement("cam-3", "grp-3", &intent, &[], 1, false)
        .expect("r3 成功");

    assert_eq!(r1.decision.core_mask, 1, "首个分配至 Core 0");
    assert_eq!(r2.decision.core_mask, 2, "第二分配至 Core 1");
    assert_eq!(r3.decision.core_mask, 4, "第三分配至 Core 2");

    // 第 4-6 个实例再次轮转填满 3 核心 (每个核心达到 max_instances_per_core = 2)
    let (r4, _, _) = ledger
        .reserve_placement("cam-4", "grp-4", &intent, &[], 1, false)
        .expect("r4 成功");
    let (r5, _, _) = ledger
        .reserve_placement("cam-5", "grp-5", &intent, &[], 1, false)
        .expect("r5 成功");
    let (r6, _, _) = ledger
        .reserve_placement("cam-6", "grp-6", &intent, &[], 1, false)
        .expect("r6 成功");

    assert_eq!(r4.decision.core_mask, 1);
    assert_eq!(r5.decision.core_mask, 2);
    assert_eq!(r6.decision.core_mask, 4);

    // 第 7 个实例应触发全设备配额满错误
    let err7 = ledger
        .reserve_placement("cam-7", "grp-7", &intent, &[], 1, false)
        .expect_err("超出设备配额上限应失败");
    assert!(matches!(err7, PlacementError::DeviceQuotaExceeded { .. }));
}

#[test]
fn test_manual_pinning_and_offline_quota_isolation_t09() {
    let inv = DeviceInventory::mock_rk3588();
    let config = LedgerConfig {
        max_instances_per_core: 2,
        max_offline_workers: 1,
        ..Default::default()
    };
    let ledger = PlacementLedger::new(inv, config);

    // 1. Manual 指定绑定 Core 2
    let manual_intent = AffinityIntent::Manual {
        device_id: "rknn-npu0".to_string(),
        core_index: 2,
    };
    let (m1, _, _) = ledger
        .reserve_placement("cam-manual", "grp-manual", &manual_intent, &[], 1, false)
        .expect("manual 成功");
    assert_eq!(m1.decision.core_mask, 4);

    // 2. 离线 Worker 配额隔离验证
    let auto_intent = AffinityIntent::Auto {
        policy: "spread".to_string(),
    };
    let (off1, _, _) = ledger
        .reserve_placement("offline-1", "grp-off", &auto_intent, &[], 1, true)
        .expect("第一个离线任务成功");
    assert!(off1.is_offline);

    // 第二个离线任务超出 max_offline_workers = 1，应被拦截
    let err_off2 = ledger
        .reserve_placement("offline-2", "grp-off", &auto_intent, &[], 1, true)
        .expect_err("离线配额超限应被拦截");
    assert!(matches!(
        err_off2,
        PlacementError::OfflineQuotaExceeded { .. }
    ));

    // 但在线任务不受离线配额影响，仍可正常准入
    let (online_ok, _, _) = ledger
        .reserve_placement("cam-online", "grp-online", &auto_intent, &[], 1, false)
        .expect("在线任务应不受离线配额限制");
    assert!(!online_ok.is_offline);
}

#[test]
fn test_quarantined_instance_replacement_blocks_reuse_t10() {
    let inv = DeviceInventory::mock_rk3588();
    let config = LedgerConfig {
        max_instances_per_core: 1, // 单核心仅容纳 1 实例
        ..Default::default()
    };
    let ledger = PlacementLedger::new(inv, config);

    let manual_core0 = AffinityIntent::Manual {
        device_id: "rknn-npu0".to_string(),
        core_index: 0,
    };

    // 实例 1 占用 Core 0
    let (r1, _, _) = ledger
        .reserve_placement("cam-1", "grp-1", &manual_core0, &[], 1, false)
        .expect("r1 成功");

    // 实例 1 发生异常转入 Quarantined
    ledger.mark_quarantined(r1.id);
    let q = ledger.get_reservation(r1.id).expect("应查到预留项");
    assert_eq!(q.state, ExecutionState::Quarantined);

    // 由于处于 Quarantined 状态仍占额（未彻底清理），同核心新实例不可直接抢占
    let err_new = ledger
        .reserve_placement("cam-retry", "grp-retry", &manual_core0, &[], 1, false)
        .expect_err("Core 0 被隔离实例占用，配额未释放前应拒绝新实例");
    assert!(matches!(err_new, PlacementError::CoreQuotaExceeded { .. }));

    // 只有在彻底完成清理并 release 后，配额方可回收
    ledger.release_placement(r1.id).expect("释放隔离实例");
    let (r_retry_ok, _, _) = ledger
        .reserve_placement("cam-retry", "grp-retry", &manual_core0, &[], 1, false)
        .expect("释放后 Core 0 应可成功准入");
    assert_eq!(r_retry_ok.decision.core_mask, 1);
}

#[test]
fn test_topology_generation_advancement_preserves_old_release_t11() {
    let inv = DeviceInventory::mock_rk3588(); // 代际 1
    let ledger = PlacementLedger::new(inv, LedgerConfig::default());

    let (r1, _, _) = ledger
        .reserve_placement("cam-1", "grp-1", &AffinityIntent::default(), &[], 1, false)
        .expect("r1 成功");
    assert_eq!(r1.topology_generation, 1);

    // 模拟系统发生硬件变更（如降级为单核 RK3568），拓扑代际变为 2
    let mut new_inv = DeviceInventory::mock_rk3568();
    new_inv.topology_generation = 2;
    ledger.update_inventory(new_inv);

    // 断言：在旧代际 (generation 1) 创建的 reservation 依然能正常释放，绝不能被新代际拒绝！
    let released = ledger
        .release_placement(r1.id)
        .expect("旧代际资源必须允许正常释放");
    assert_eq!(released.topology_generation, 1);
    assert_eq!(ledger.active_reservations_count(), 0);
}

#[test]
fn test_quarantine_breaker_triggers_and_rejects_new_admissions_t12() {
    let inv = DeviceInventory::mock_rk3588();
    let config = LedgerConfig {
        max_quarantined_workers: 0, // 设置阈值为 0，模拟当前隔离数 >= 阈值
        ..Default::default()
    };
    let ledger = PlacementLedger::new(inv, config);

    let err = ledger
        .reserve_placement("cam-1", "grp-1", &AffinityIntent::default(), &[], 1, false)
        .expect_err("达到隔离阈值应触发防雪崩熔断");

    assert_eq!(err, PlacementError::QuarantineBreakerTriggered);
}

#[test]
fn test_cooling_weight_reuse_and_generational_safe_timer_t19() {
    let inv = DeviceInventory::mock_rk3588();
    let ledger = PlacementLedger::new(inv, LedgerConfig::default());

    // 1. 实例 1 预留并使用权重
    let (r1, _, init1) = ledger
        .reserve_placement(
            "cam-1",
            "grp-1",
            &AffinityIntent::default(),
            &["scrfd".to_string()],
            1,
            false,
        )
        .expect("r1 成功");
    assert!(init1);

    // 2. 实例 1 释放，此时无活跃实例，scrfd 权重进入 Cooling 冷却退火期
    ledger.release_placement(r1.id).expect("释放 r1");

    // 3. 实例 2 在退火完成前请求同一 scrfd 模型
    let (r2, _, init2) = ledger
        .reserve_placement(
            "cam-2",
            "grp-2",
            &AffinityIntent::default(),
            &["scrfd".to_string()],
            1,
            false,
        )
        .expect("r2 成功");

    // 核心断言：权重处于 Cooling 期时被新任务复用，无需重新初始化！
    assert!(!init2, "Cooling 状态的权重被复用，不应重新触发模型初始化");
    assert_eq!(ledger.active_reservations_count(), 1);

    ledger.release_placement(r2.id).expect("释放 r2");
    assert_eq!(ledger.active_reservations_count(), 0);
}
