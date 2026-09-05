use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(name = "argus")]
#[command(about = "Argus / Heimdall 边缘端一体化 AI 视频分析系统", version)]
struct Args {
    /// 数据库 SQLite 文件路径
    #[arg(short, long, default_value = "argus.db")]
    db: String,

    /// Web 控制台与 API 监听端口
    #[arg(short, long, default_value_t = 8000)]
    port: u16,

    /// 监听地址
    #[arg(long, default_value = "0.0.0.0")]
    host: String,
    /// 开发调试模式：启动前重置并重建本地数据库
    #[arg(short = 'r', long, default_value_t = false)]
    reset_db: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 0. 隐藏子命令：自测子进程物理隔离入口（用于算法包沙箱校验防崩溃）
    let raw_args: Vec<String> = std::env::args().collect();
    handle_verify_algo_subprocess(&raw_args);

    // 1. 初始化结构化日志
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,api=debug,media=debug,pipeline=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        port = args.port,
        host = %args.host,
        db_path = %args.db,
        reset_db = args.reset_db,
        "正在启动 Argus 单进程服务..."
    );

    // 2. 执行版本化数据库迁移 (Refinery) 并建立 SeaORM 连接池 (SQLite WAL 模式)
    let db_path = std::path::Path::new(&args.db);
    if args.reset_db {
        tracing::warn!(
            db_path = %db_path.display(),
            "已启用 --reset-db 参数，正在清空并重置本地数据库..."
        );
        db::reset_database(db_path).context("重置本地数据库失败")?;
    } else {
        db::run_migrations(db_path).context("执行数据库版本迁移失败")?;
    }

    let db_conn = db::init_db(&args.db)
        .await
        .context("初始化 SQLite 数据库失败")?;
    tracing::info!("SQLite 数据库版本迁移与连接池初始化完成 (WAL 模式)");

    // 3. 初始化视频分析管线调度器
    let pipeline_mgr = Arc::new(pipeline::PipelineManager::new());
    tracing::info!("核心视频分析管线调度器初始化完成");

    // 4. 检查双轨初始化状态与环境变量
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
    let state = api::AppState::new(db_conn, pipeline_mgr);
    state.sync_auth_state().await;

    // 启动后台静默待机摄像头定时巡检与防抖三态调度器 (30s 周期)
    Arc::new(state.clone()).start_periodic_probe_worker(std::time::Duration::from_secs(30));

    // 扫描并沙箱自检加载本地算法包 (algo-packages/{platform})
    let algo_dir = std::path::Path::new("algo-packages");
    if algo_dir.is_dir() {
        let current_platform = infer::current_platform_id();
        match state.algo_registry.scan_and_register(algo_dir, true).await {
            Ok(count) => {
                tracing::info!(
                    platform = current_platform,
                    count,
                    "本地算法包扫描与沙箱自检完成"
                );
            }
            Err(err) => {
                tracing::warn!(
                    platform = current_platform,
                    error = %err,
                    "扫描本地算法包失败"
                );
            }
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

    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .context("无效的监听地址或端口")?;

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .context("绑定监听端口失败")?;

    tracing::info!("Argus Web 控制台与 API 服务已就绪: http://{}", addr);

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
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_fut)
        .await
        .context("HTTP 服务运行发生异常")?;

    tracing::info!("Argus 服务已安全优雅停机");
    Ok(())
}

/// 处理算法包沙箱物理隔离自检子进程请求
fn handle_verify_algo_subprocess(raw_args: &[String]) {
    if raw_args.len() < 3 || raw_args[1] != "__verify-algo" {
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
    let manifest_path = pkg_path.join("manifest.json");
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
