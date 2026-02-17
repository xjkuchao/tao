//! 日志初始化模块.
//!
//! 双输出:
//! - console: 彩色, debug 级别
//! - file: 无色, 无 target, 默认 info, 可通过 -v/-vv/-vvv 或 TAO_LOG 环境变量调整
//!
//! 日志文件输出到 $cwd/logs/{prefix}.{date}.log

use chrono::{Datelike, Local, Timelike};
use std::sync::OnceLock;
use tracing_subscriber::{
    EnvFilter, Registry,
    fmt::{self, FormatEvent, FormatFields, format::Writer},
    layer::{Layer, SubscriberExt},
    registry::LookupSpan,
    util::SubscriberInitExt,
};

static LOG_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

/// 初始化日志系统
///
/// - `file_prefix`: 日志文件前缀 (如 "tao-play")
/// - `verbosity`: 0=info, 1=debug, 2+=trace (由 -v/-vv/-vvv 控制)
pub fn init(file_prefix: &str, verbosity: u8) {
    std::fs::create_dir_all("logs").ok();

    let file_appender = tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix(file_prefix)
        .filename_suffix("log")
        .build("logs")
        .expect("创建日志文件失败");

    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    LOG_GUARD.set(guard).ok();

    // Console: debug 级别, 彩色
    let console_filter = EnvFilter::new("debug");
    let console_layer = fmt::Layer::default()
        .with_writer(std::io::stdout)
        .with_ansi(true)
        .event_format(ConsoleFormatter)
        .with_filter(console_filter);

    // File: 默认 info, 通过 -v 提升, TAO_LOG 环境变量可覆盖
    let file_level = match verbosity {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let file_filter =
        EnvFilter::try_from_env("TAO_LOG").unwrap_or_else(|_| EnvFilter::new(file_level));

    let file_layer = fmt::Layer::default()
        .with_writer(non_blocking)
        .with_ansi(false)
        .event_format(FileFormatter)
        .with_filter(file_filter);

    Registry::default()
        .with(console_layer)
        .with(file_layer)
        .init();
}

/// Console 格式: 彩色, 带时间戳和源码位置
struct ConsoleFormatter;

impl<S, N> FormatEvent<S, N> for ConsoleFormatter
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &fmt::FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let now = Local::now();
        let meta = event.metadata();
        let color = match *meta.level() {
            tracing::Level::ERROR => "\x1b[31m",
            tracing::Level::WARN => "\x1b[33m",
            tracing::Level::INFO => "\x1b[32m",
            _ => "\x1b[34m",
        };
        write!(
            writer,
            "[{:02}-{:02} {:02}:{:02}:{:02}.{:03}] {}{:5}\x1b[0m > ",
            now.month(),
            now.day(),
            now.hour(),
            now.minute(),
            now.second(),
            now.timestamp_subsec_millis(),
            color,
            meta.level(),
        )?;
        ctx.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// File 格式: 无色, 无 target, 时间戳 + 级别 + 消息
struct FileFormatter;

impl<S, N> FormatEvent<S, N> for FileFormatter
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &fmt::FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let now = Local::now();
        write!(
            writer,
            "[{:02}-{:02} {:02}:{:02}:{:02}.{:03}] {:5} > ",
            now.month(),
            now.day(),
            now.hour(),
            now.minute(),
            now.second(),
            now.timestamp_subsec_millis(),
            event.metadata().level(),
        )?;
        ctx.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}
