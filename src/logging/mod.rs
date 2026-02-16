use anyhow::{Context, Result};
use chrono::{Datelike, Local, NaiveDate, Timelike};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing_subscriber::{
    EnvFilter, Registry,
    fmt::{self, FormatEvent, FormatFields, format::Writer},
    layer::{Layer, SubscriberExt},
    registry::LookupSpan,
    util::SubscriberInitExt,
};

mod task;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingConfig {
    pub level: String,
    pub directory: String,
    pub file_prefix: String,
    #[serde(default = "default_retention_days")]
    pub retention_days: i64,
    #[serde(default = "default_true")]
    pub compress_history: bool,
    #[serde(default = "default_cleanup_interval")]
    pub cleanup_interval_seconds: u64,
}

fn default_true() -> bool {
    true
}

fn default_retention_days() -> i64 {
    30
}

fn default_cleanup_interval() -> u64 {
    3600
}

static LOG_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

pub fn init(config: LoggingConfig) -> Result<()> {
    std::fs::create_dir_all(&config.directory)?;

    let rotate_requested = Arc::new(AtomicBool::new(false));
    let file_appender = CurrentFileWriter::new(
        Path::new(&config.directory),
        &config.file_prefix,
        Arc::clone(&rotate_requested),
    )?;

    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    LOG_GUARD.set(guard).ok();

    let console_filter = EnvFilter::new("debug");
    let file_filter = EnvFilter::new(&config.level);

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

    task::spawn_log_maintenance_task(config, rotate_requested);

    Ok(())
}

struct CurrentFileWriter {
    directory: PathBuf,
    prefix: String,
    rotate_requested: Arc<AtomicBool>,
    file: File,
}

impl CurrentFileWriter {
    fn new(directory: &Path, prefix: &str, rotate_requested: Arc<AtomicBool>) -> Result<Self> {
        let today = Local::now().date_naive();
        let file_path = build_current_log_path(directory, prefix, today);
        let file = open_append_file(&file_path)?;
        Ok(Self {
            directory: directory.to_path_buf(),
            prefix: prefix.to_string(),
            rotate_requested,
            file,
        })
    }

    fn reopen_current_file(&mut self) -> std::io::Result<()> {
        let today = Local::now().date_naive();
        let file_path = build_current_log_path(&self.directory, &self.prefix, today);
        let file = open_append_file(&file_path)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
        self.file = file;
        Ok(())
    }
}

impl Write for CurrentFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.rotate_requested.swap(false, Ordering::AcqRel) {
            self.reopen_current_file()?;
        }
        self.file.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

fn open_append_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("打开日志文件失败, path={}", path.display()))
}

pub(crate) fn build_current_log_path(directory: &Path, prefix: &str, date: NaiveDate) -> PathBuf {
    directory.join(format!("{}.{}.log", prefix, date.format("%Y-%m-%d")))
}

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
        write!(
            writer,
            "[{:02}-{:02} {:02}:{:02}:{:02}.{:03}] ",
            now.month(),
            now.day(),
            now.hour(),
            now.minute(),
            now.second(),
            now.timestamp_subsec_millis()
        )?;
        let color = match *meta.level() {
            tracing::Level::ERROR => "\x1b[31m",
            tracing::Level::WARN => "\x1b[33m",
            tracing::Level::INFO => "\x1b[32m",
            _ => "\x1b[34m",
        };
        write!(
            writer,
            "{}{:5}\x1b[0m {}:{} > ",
            color,
            meta.level().to_string(),
            meta.file().unwrap_or("unknown"),
            meta.line().unwrap_or(0)
        )?;
        ctx.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

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
            event.metadata().level().to_string()
        )?;
        ctx.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_current_log_path() {
        let date = NaiveDate::from_ymd_opt(2026, 2, 6);
        match date {
            Some(date) => {
                let path = build_current_log_path(Path::new("logs"), "worker", date);
                assert_eq!(path, PathBuf::from("logs/worker.2026-02-06.log"));
            }
            None => panic!("测试日期初始化失败"),
        }
    }
}
