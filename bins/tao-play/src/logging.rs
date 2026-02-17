//! 日志初始化模块.
//!
//! 双输出 (统一级别):
//! - console: 彩色
//! - file: 无色, 无 target
//!
//! 级别体系 (优先级: TAO_LOG 环境变量 > 命令行 > 默认):
//! - 默认:   info  (关键生命周期事件)
//! - `-v`:   debug (内部状态/决策)
//! - `-vv`:  trace (仅 tao 项目 crate, 第三方依赖保持 info)
//! - `-vvv`: trace (全局, 含第三方依赖)
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

/// 本项目所有 crate 的 target 前缀 (用于 -vv 级别的定向 trace)
const TAO_CRATE_TARGETS: &[&str] = &[
    "tao",
    "tao_core",
    "tao_codec",
    "tao_format",
    "tao_filter",
    "tao_scale",
    "tao_resample",
    "tao_ffi",
    "tao_cli",
    "tao_probe",
    "tao_play",
];

/// 构建 -vv 级别的 EnvFilter: tao crate trace, 其余 info
fn build_tao_trace_filter() -> EnvFilter {
    let mut directives = TAO_CRATE_TARGETS
        .iter()
        .map(|t| format!("{t}=trace"))
        .collect::<Vec<_>>();
    directives.push("info".to_string());
    EnvFilter::new(directives.join(","))
}

/// 根据 verbosity 构建 EnvFilter
///
/// - 0: info
/// - 1: debug
/// - 2: trace (仅 tao crate, 第三方依赖保持 info)
/// - 3+: trace (全局, 含第三方依赖)
fn build_filter(verbosity: u8) -> EnvFilter {
    match verbosity {
        0 => EnvFilter::new("info"),
        1 => EnvFilter::new("debug"),
        2 => build_tao_trace_filter(),
        _ => EnvFilter::new("trace"),
    }
}

/// 初始化日志系统
///
/// - `file_prefix`: 日志文件前缀 (如 "tao-play")
/// - `verbosity`: 0=info, 1=debug, 2=trace(tao), 3+=trace(all)
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

    // 统一级别: TAO_LOG 环境变量 > 命令行 verbosity > 默认 info
    let console_filter =
        EnvFilter::try_from_env("TAO_LOG").unwrap_or_else(|_| build_filter(verbosity));
    let file_filter =
        EnvFilter::try_from_env("TAO_LOG").unwrap_or_else(|_| build_filter(verbosity));

    let console_layer = fmt::Layer::default()
        .with_writer(std::io::stdout)
        .with_ansi(true)
        .event_format(ConsoleFormatter)
        .with_filter(console_filter);

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
