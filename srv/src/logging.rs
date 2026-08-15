use std::{env, error::Error, io, path::PathBuf};

use tracing_appender::{
    non_blocking::{NonBlockingBuilder, WorkerGuard},
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

const DEFAULT_LOG_DIR: &str = "logs";
const LOG_FILE_PREFIX: &str = "qddxp";
const LOG_FILE_SUFFIX: &str = "log";

/// 初始化控制台和文件日志，并返回后台写入线程的生命周期守卫。
///
/// 文件名由 `tracing-appender` 按 UTC 日期生成为 `qddxp.YYYY-MM-DD.log`，跨日后自动
/// 切换到新文件。守卫必须一直存活到进程退出，否则后台线程会提前停止且尾部日志可能
/// 尚未刷新到磁盘。
pub fn init() -> Result<WorkerGuard, Box<dyn Error + Send + Sync>> {
    let log_dir = env::var_os("LOG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_LOG_DIR));

    // 显式创建并校验目录。日志落盘是部署要求，目录不可写时应在启动阶段直接失败，
    // 不能静默退化成只有控制台日志，否则故障发生后会误以为文件中已有完整记录。
    std::fs::create_dir_all(&log_dir).map_err(|source| {
        io::Error::new(
            source.kind(),
            format!(
                "cannot create log directory {}: {source}",
                log_dir.display()
            ),
        )
    })?;

    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix(LOG_FILE_SUFFIX)
        .build(&log_dir)?;

    // 非阻塞写入避免请求线程等待磁盘 IO；关闭 lossy 后，队列压力过大时会施加背压，
    // 而不是丢弃对支付与库存问题排查至关重要的日志。
    let (file_writer, guard) = NonBlockingBuilder::default()
        .lossy(false)
        .finish(file_appender);
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // 保留控制台输出，方便本地开发和 `docker compose logs`；文件层禁用 ANSI 控制符，
    // 确保日志文件可被常规文本工具及后续采集系统直接解析。
    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(file_writer),
        )
        .try_init()?;

    tracing::info!(
        log_dir = %log_dir.display(),
        file_pattern = "qddxp.YYYY-MM-DD.log",
        rotation_timezone = "UTC",
        "daily rolling file logging initialized"
    );
    Ok(guard)
}
