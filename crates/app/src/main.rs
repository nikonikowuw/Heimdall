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
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1. 初始化结构化日志
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,api=debug,pipeline=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        port = args.port,
        host = %args.host,
        db_path = %args.db,
        "正在启动 Argus 单进程服务..."
    );

    // 2. 初始化数据库 (SQLite WAL 模式)
    let db_conn = db::init_db(&args.db)
        .await
        .context("初始化 SQLite 数据库失败")?;
    db::create_tables_if_not_exist(&db_conn)
        .await
        .context("初始化数据表 Schema 失败")?;
    tracing::info!("SQLite 数据库连接与 Schema 初始化完成 (WAL 模式)");

    // 3. 初始化视频分析管线调度器
    let pipeline_mgr = Arc::new(pipeline::PipelineManager::new());
    tracing::info!("核心视频分析管线调度器初始化完成");

    // 4. 组装 API 共享状态与路由器
    let state = api::AppState::new(db_conn, pipeline_mgr);
    let app = api::create_app(state);

    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .context("无效的监听地址或端口")?;

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .context("绑定监听端口失败")?;

    tracing::info!("Argus Web 控制台与 API 服务已就绪: http://{}", addr);

    // 5. 启动 HTTP / WebSocket 服务并监听优雅停机信号
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("HTTP 服务运行发生异常")?;

    tracing::info!("Argus 服务已安全优雅停机");
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
