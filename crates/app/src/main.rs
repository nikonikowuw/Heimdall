use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod reconcile;

#[derive(Parser, Debug)]
#[command(name = "heimdall")]
#[command(about = "Heimdall 边缘端一体化 AI 视频分析系统", version)]
struct Args {
    /// 配置文件路径 (可选，默认依次查找 config.toml 或 config.json)
    #[arg(short, long)]
    config: Option<String>,

    /// 数据库 SQLite 文件路径 (覆盖配置文件)
    #[arg(short, long)]
    db: Option<String>,

    /// Web 控制台与 API 监听端口 (覆盖配置文件)
    #[arg(short, long)]
    port: Option<u16>,

    /// 监听地址 (覆盖配置文件)
    #[arg(long)]
    host: Option<String>,

    /// 开发调试模式：启动前重置并重建本地数据库
    #[arg(short = 'r', long, default_value_t = false)]
    reset_db: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 0. 隐藏子命令：自测子进程物理隔离入口（用于算法包沙箱校验防崩溃）
    let raw_args: Vec<String> = std::env::args().collect();
    handle_verify_algo_subprocess(&raw_args);

    let args = Args::parse();

    // 1. 加载系统配置 (环境变量 > .env > config.toml > 内置默认值)
    let mut cfg = config::load_config(args.config.as_deref()).context("加载系统配置失败")?;

    // CLI 显式参数覆盖配置文件
    if let Some(db) = args.db {
        cfg.database.path = db;
    }
    if let Some(port) = args.port {
        cfg.server.port = port;
    }
    if let Some(host) = args.host {
        cfg.server.host = host;
    }

    // 2. 初始化结构化日志
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| cfg.logging.filter.clone());
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // 提升进程文件描述符上限 (防止高并发多媒体流与 DMA-BUF fd 耗尽)
    raise_fd_limit();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        port = cfg.server.port,
        host = %cfg.server.host,
        max_package_size_mb = cfg.server.max_package_size_mb,
        db_path = %cfg.database.path,
        max_concurrent_decoders = cfg.pipeline.max_concurrent_decoders,
        permit_timeout_ms = cfg.pipeline.permit_timeout_ms,
        max_burst_timeout_ms = cfg.pipeline.max_burst_timeout_ms,
        reset_db = args.reset_db,
        "正在启动 Heimdall 单进程服务..."
    );

    // 3. 执行版本化数据库迁移 (Refinery) 并建立 SeaORM 连接池 (SQLite WAL 模式)
    let db_path = std::path::Path::new(&cfg.database.path);
    if args.reset_db {
        tracing::warn!(
            db_path = %db_path.display(),
            "已启用 --reset-db 参数，正在清空并重置本地数据库..."
        );
        db::reset_database(db_path).context("重置本地数据库失败")?;
    } else {
        db::run_migrations(db_path).context("执行数据库版本迁移失败")?;
    }

    let db_conn = db::init_db(&cfg.database.path)
        .await
        .context("初始化 SQLite 数据库失败")?;
    tracing::info!("SQLite 数据库版本迁移与连接池初始化完成 (WAL 模式)");

    // 4. 初始化视频分析管线调度器 (注入配置参数与全局 VPU 通道池)
    let snapshot_cfg = pipeline::SnapshotConfig {
        phase_diff_threshold_ms: cfg.pipeline.phase_diff_threshold_ms,
        max_burst_packets: cfg.pipeline.max_burst_packets,
        max_burst_timeout_ms: cfg.pipeline.max_burst_timeout_ms,
        capture_mode: cfg.pipeline.capture_mode,
    };

    let evidence_dir = cfg.storage.evidence_dir.clone();
    let pipeline_mgr = Arc::new(pipeline::PipelineManager::with_all_options(
        cfg.storage.evidence_dir,
        snapshot_cfg,
        cfg.pipeline.max_concurrent_decoders,
        cfg.pipeline.permit_timeout_ms,
    ));
    tracing::info!("核心视频分析管线调度器初始化完成 (全局 VPU 通道池就绪)");

    // 5. 检查双轨初始化状态与环境变量
    let env_password = std::env::var("ARGUS_ADMIN_PASSWORD").ok();
    if let Some(pwd) = env_password {
        let pwd = pwd.trim();
        if !pwd.is_empty() {
            let username =
                std::env::var("ARGUS_ADMIN_USERNAME").unwrap_or_else(|_| "admin".to_string());
            let hash = api::crypto::hash_password(pwd);
            if db::AdminUserRepo::ensure_silent_admin(&db_conn, &username, &hash).await? {
                tracing::info!(username = %username, "已通过环境变量自动完成管理员静默初始化");
            }
        }
    }

    // 5. 组装 API 共享状态与路由器并同步初始化与撤销时间戳
    let max_upload_size_bytes = cfg
        .server
        .max_package_size_bytes()
        .context("算法包上传大小配置无效")?;
    let state = api::AppState::new_with_limit(db_conn, pipeline_mgr, max_upload_size_bytes)
        .with_storage_cleaner(evidence_dir);
    api::sync_auth_state(&state).await;

    // 同步加载数据库中持久化的存储保留与水位配置至运行时 StorageCleaner
    if let Some(cleaner) = state.storage_cleaner.as_ref() {
        if let Ok(Some(json_str)) = db::SystemConfigRepo::get(&state.db, "storage_config").await {
            if let Ok(saved_cfg) = serde_json::from_str::<types::StorageConfig>(&json_str) {
                let mut runtime_cfg = cleaner.get_config().await;
                runtime_cfg.min_free_ratio = saved_cfg.min_free_ratio;
                runtime_cfg.target_free_ratio = saved_cfg.target_free_ratio;
                runtime_cfg.emergency_free_ratio = saved_cfg.emergency_free_ratio;
                runtime_cfg.critical_free_ratio = saved_cfg.critical_free_ratio;
                runtime_cfg.batch_delete_size = saved_cfg.batch_delete_size as u64;
                runtime_cfg.alarm_retention_days = saved_cfg.alarm_retention_days;
                runtime_cfg.alarm_quota_mb = saved_cfg.alarm_quota_mb;
                runtime_cfg.recognition_retention_days = saved_cfg.recognition_retention_days;
                runtime_cfg.recognition_quota_mb = saved_cfg.recognition_quota_mb;
                runtime_cfg.capture_retention_days = saved_cfg.capture_retention_days;
                runtime_cfg.capture_quota_mb = saved_cfg.capture_quota_mb;
                runtime_cfg.overwrite_mode = saved_cfg.overwrite_mode;
                runtime_cfg.auto_cleanup_enabled = saved_cfg.auto_cleanup_enabled;
                cleaner.update_config(runtime_cfg).await;
                tracing::info!("已从系统配置成功加载历史存储保留与自适应水位参数");
            }
        }

        // 启动常驻后台存储水位自适应巡检与过期凭据清理任务 (300s 周期)
        let store = Arc::new(api::DbEvictionStoreAdapter(state.db.clone()));
        cleaner
            .clone()
            .start_periodic_worker(store, std::time::Duration::from_secs(300));
        tracing::info!("后台存储水位与证据生命周期自适应巡检工作线程已启动 (300s 周期)");
    }

    // 启动后台静默待机摄像头定时巡检与防抖三态调度器 (30s 周期)
    let probe_svc = Arc::new(api::CameraProbeService::from_state(&state));
    probe_svc.start_periodic_probe_worker(std::time::Duration::from_secs(30));

    // 执行网络服务冷启动防失联自愈检查（恢复意外断电或重启前未确认的网卡快照）
    api::NetworkService::recover_pending_snapshots_on_startup().await;

    // 执行冷启动自愈对齐与活跃算法包装载
    match reconcile::reconcile_and_seed_algorithms(&state.db, &state.algo_registry).await {
        Ok((seeded, loaded)) => {
            tracing::info!(
                seeded_count = seeded,
                active_loaded_count = loaded,
                "算法包冷启动自愈对齐与运行时装载完成"
            );
        }
        Err(err) => {
            tracing::warn!(error = %err, "算法包自愈对齐流程产生警告，继续以容灾模式启动");
        }
    }

    let is_init = state
        .is_initialized
        .load(std::sync::atomic::Ordering::Relaxed);
    if !is_init {
        tracing::warn!(
            "系统尚未初始化管理员账号，访问 Web 控制台时将强制导航至开箱向导进行初始配置"
        );
    } else {
        tracing::info!("管理员账号已就绪，系统运行在正常防护模式");
    }

    let state_shutdown = state.clone();
    let app = api::create_app(state);

    let addr: SocketAddr = format!("{}:{}", cfg.server.host, cfg.server.port)
        .parse()
        .context("无效的监听地址或端口")?;

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .context("绑定监听端口失败")?;

    tracing::info!("Heimdall Web 控制台与 API 服务已就绪: http://{}", addr);

    let shutdown_fut = async move {
        shutdown_signal().await;
        tracing::info!("正在广播全局停机通知，主动切断长连接流与后台巡检任务...");
        state_shutdown.notify_shutdown();

        // 兜底保护：若 2.5 秒内未完成退出，或用户再次按下 Ctrl+C，立即强制退出
        tokio::spawn(async {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(2500)) => {
                    tracing::warn!("优雅停机超时 (2.5s)，触发强制退出");
                    std::process::exit(0);
                }
                _ = tokio::signal::ctrl_c() => {
                    tracing::warn!("再次接收到 Ctrl+C 中断信号，立即强制退出");
                    std::process::exit(0);
                }
            }
        });
    };

    // 5. 启动 HTTP / WebSocket 服务并监听优雅停机信号
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_fut)
    .await
    .context("HTTP 服务运行发生异常")?;

    tracing::info!("Heimdall 服务已安全优雅停机");
    Ok(())
}

/// 处理算法包沙箱物理隔离自检子进程请求
fn handle_verify_algo_subprocess(raw_args: &[String]) {
    if raw_args.len() < 3 || raw_args[1] != infer::VERIFY_ALGO_ARG {
        return;
    }
    let pkg_path = std::path::Path::new(&raw_args[2]);
    if let Err(err) = execute_algo_verification(pkg_path) {
        eprintln!("{err}");
        std::process::exit(1);
    }
    std::process::exit(0);
}

fn execute_algo_verification(pkg_path: &std::path::Path) -> Result<(), String> {
    let manifest_path = pkg_path.join(infer::ALGO_MANIFEST_FILENAME);
    let manifest_str =
        std::fs::read_to_string(&manifest_path).map_err(|e| format!("读取 manifest 失败: {e}"))?;
    let manifest: infer::AlgoManifest =
        serde_json::from_str(&manifest_str).map_err(|e| format!("解析 manifest 失败: {e}"))?;
    let entry_lib = infer::sandbox::find_entry_library(pkg_path, &manifest.algorithm_id)
        .map_err(|e| format!("查找动态库失败: {e}"))?;
    let report =
        infer::sandbox::AlgoSandbox::run_in_process_self_test(pkg_path, &entry_lib, &manifest)
            .map_err(|e| format!("自测执行失败: {e}"))?;
    let json = serde_json::to_string(&report).map_err(|e| format!("序列化自测报告失败: {e}"))?;
    println!("{json}");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("未能注册 Ctrl+C 信号监听器");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("未能注册 SIGTERM 信号监听器")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("接收到 Ctrl+C 中断信号，准备退出..."),
        _ = terminate => tracing::info!("接收到 SIGTERM 终止信号，准备退出..."),
    }
}

/// 自动提升进程打开文件描述符上限 (RLIMIT_NOFILE)
/// 针对高并发 RTSP 连接与多媒体 DMA-BUF fd 密集操作，消除 EMFILE 异常风险
fn raise_fd_limit() {
    #[cfg(unix)]
    {
        // SAFETY: 调用标准 POSIX getrlimit / setrlimit 查询并设置进程文件描述符限制
        unsafe {
            let mut rlim = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) == 0 {
                let old_cur = rlim.rlim_cur;
                // 目标设定为 65535 或硬上限
                let target = 65535.min(rlim.rlim_max);
                if target > rlim.rlim_cur {
                    rlim.rlim_cur = target;
                    if libc::setrlimit(libc::RLIMIT_NOFILE, &rlim) == 0 {
                        tracing::info!(
                            old_limit = old_cur,
                            new_limit = target,
                            hard_limit = rlim.rlim_max,
                            "已成功提升进程最大文件描述符上限 (RLIMIT_NOFILE)"
                        );
                        return;
                    }
                }
                tracing::debug!(
                    cur_limit = old_cur,
                    hard_limit = rlim.rlim_max,
                    "当前进程文件描述符限制 (RLIMIT_NOFILE)"
                );
            }
        }
    }
}
