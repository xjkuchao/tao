use super::{build_current_log_path, LoggingConfig};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Local, NaiveDate, TimeZone, Utc};
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::error;

pub(super) fn spawn_log_maintenance_task(config: LoggingConfig, rotate_requested: Arc<AtomicBool>) {
    tokio::spawn(async move {
        let mut cleanup_interval =
            tokio::time::interval(Duration::from_secs(config.cleanup_interval_seconds));

        if let Err(err) = ensure_current_log_file(&config) {
            error!("初始化当前日志文件失败: {}", err);
        }
        if let Err(err) = cleanup_logs(&config) {
            error!("启动时清理日志失败: {}", err);
        }

        let mut next_rollover_at = match compute_next_rollover(Local::now()) {
            Ok(at) => at,
            Err(err) => {
                error!("计算下一次翻滚时间失败: {}", err);
                tokio::time::Instant::now() + Duration::from_secs(1)
            }
        };

        loop {
            tokio::select! {
                _ = cleanup_interval.tick() => {
                    if let Err(err) = cleanup_logs(&config) {
                        error!("清理日志失败: {}", err);
                    }
                }
                _ = tokio::time::sleep_until(next_rollover_at) => {
                    if let Err(err) = rotate_current_log(&config) {
                        error!("日志翻滚失败: {}", err);
                    } else {
                        rotate_requested.store(true, Ordering::Release);
                    }

                    if let Err(err) = cleanup_logs(&config) {
                        error!("翻滚后清理日志失败: {}", err);
                    }

                    next_rollover_at = match compute_next_rollover(Local::now()) {
                        Ok(at) => at,
                        Err(err) => {
                            error!("重新计算下一次翻滚时间失败: {}", err);
                            tokio::time::Instant::now() + Duration::from_secs(1)
                        }
                    };
                }
            }
        }
    });
}

fn ensure_current_log_file(config: &LoggingConfig) -> Result<()> {
    let directory = Path::new(&config.directory);
    fs::create_dir_all(directory)?;
    let today = Local::now().date_naive();
    let current_path = build_current_log_path(directory, &config.file_prefix, today);
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&current_path)
        .with_context(|| format!("创建当前日志文件失败, path={}", current_path.display()))?;
    Ok(())
}

fn rotate_current_log(config: &LoggingConfig) -> Result<()> {
    let directory = Path::new(&config.directory);
    fs::create_dir_all(directory)?;

    let today = Local::now().date_naive();
    let current_path = build_current_log_path(directory, &config.file_prefix, today);
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&current_path)
        .with_context(|| format!("创建当前日期日志文件失败, path={}", current_path.display()))?;

    Ok(())
}

fn cleanup_logs(config: &LoggingConfig) -> Result<()> {
    let directory = Path::new(&config.directory);
    if !directory.exists() {
        return Ok(());
    }

    let today = Local::now().date_naive();
    let cutoff = today - ChronoDuration::days(config.retention_days);

    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_name = entry.file_name().to_string_lossy().to_string();
        let file_path = entry.path();

        let parsed = parse_rotated_log_name(&file_name, &config.file_prefix);
        let (date, compressed) = match parsed {
            Some(value) => value,
            None => continue,
        };

        if date < cutoff {
            let _ = fs::remove_file(&file_path);
            continue;
        }

        if config.compress_history && !compressed && date < today {
            let _ = compress_to_gz(&file_path);
        }
    }

    Ok(())
}

fn compress_to_gz(path: &Path) -> Result<()> {
    let gz_path = PathBuf::from(format!("{}.gz", path.display()));
    if gz_path.exists() {
        return Ok(());
    }

    let mut input =
        File::open(path).with_context(|| format!("打开待压缩日志失败, path={}", path.display()))?;
    let output = File::create(&gz_path)
        .with_context(|| format!("创建压缩日志失败, path={}", gz_path.display()))?;
    let mut encoder = GzEncoder::new(output, Compression::default());

    let mut buf = [0u8; 8 * 1024];
    loop {
        let read = input.read(&mut buf)?;
        if read == 0 {
            break;
        }
        encoder.write_all(&buf[..read])?;
    }

    encoder.finish()?;
    fs::remove_file(path)
        .with_context(|| format!("删除已压缩日志失败, path={}", path.display()))?;
    Ok(())
}

fn parse_rotated_log_name(file_name: &str, prefix: &str) -> Option<(NaiveDate, bool)> {
    let with_prefix = file_name.strip_prefix(prefix)?;
    let with_dot = with_prefix.strip_prefix('.')?;

    if let Some(date_part) = with_dot.strip_suffix(".log") {
        let date = parse_date(date_part)?;
        return Some((date, false));
    }

    if let Some(date_part) = with_dot.strip_suffix(".log.gz") {
        let date = parse_date(date_part)?;
        return Some((date, true));
    }

    None
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    if value.len() != 10 {
        return None;
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

fn compute_next_rollover(now: DateTime<Local>) -> Result<tokio::time::Instant> {
    let next_date = now.date_naive() + ChronoDuration::days(1);
    let next_midnight = next_date
        .and_hms_opt(0, 0, 0)
        .context("计算下一次日志翻滚时间失败")?;
    let next_local = Local
        .from_local_datetime(&next_midnight)
        .earliest()
        .context("转换本地时间失败")?;
    let system_time = SystemTime::from(next_local.with_timezone(&Utc));
    let duration = match system_time.duration_since(SystemTime::now()) {
        Ok(duration) => duration,
        Err(_) => Duration::from_secs(0),
    };
    Ok(tokio::time::Instant::now() + duration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_rotated_log_name() {
        let prefix = "worker";

        let parsed = parse_rotated_log_name("worker.2026-02-06.log", prefix);
        assert_eq!(
            parsed,
            NaiveDate::from_ymd_opt(2026, 2, 6).map(|d| (d, false))
        );

        let parsed = parse_rotated_log_name("worker.2026-02-06.log.gz", prefix);
        assert_eq!(
            parsed,
            NaiveDate::from_ymd_opt(2026, 2, 6).map(|d| (d, true))
        );

        let parsed = parse_rotated_log_name("worker.log", prefix);
        assert!(parsed.is_none());
    }

    #[test]
    fn test_rotate_current_log_creates_empty_rotated_file() {
        let temp_dir = TempDir::new();
        assert!(temp_dir.is_ok());
        let temp_dir = match temp_dir {
            Ok(temp_dir) => temp_dir,
            Err(err) => panic!("创建临时目录失败: {}", err),
        };
        let directory = temp_dir.path().to_string_lossy().to_string();

        let config = LoggingConfig {
            level: "info".to_string(),
            directory,
            file_prefix: "worker".to_string(),
            retention_days: 30,
            compress_history: true,
            cleanup_interval_seconds: 60,
        };

        let ensured = ensure_current_log_file(&config);
        assert!(ensured.is_ok(), "创建当前日志文件失败: {:?}", ensured.err());

        let rotate = rotate_current_log(&config);
        assert!(rotate.is_ok(), "执行日志翻滚失败: {:?}", rotate.err());

        let today = Local::now().date_naive();
        let current_path =
            build_current_log_path(Path::new(&config.directory), &config.file_prefix, today);
        assert!(current_path.exists(), "当前日志文件不存在");
        let metadata = current_path.metadata();
        assert!(
            metadata.is_ok(),
            "读取当前日志元数据失败: {:?}",
            metadata.err()
        );
        let metadata = match metadata {
            Ok(metadata) => metadata,
            Err(err) => panic!("读取当前日志元数据失败: {}", err),
        };
        assert_eq!(metadata.len(), 0);
    }
}
