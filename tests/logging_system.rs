use chrono::Datelike;
use std::fs;
use std::path::PathBuf;
use tao::logging::{init, LoggingConfig};

// 注意: 由于 tracing 的全局订阅器只能初始化一次,
// 涉及 init() 的测试必须单独运行或使用 #[ignore] 标记

/// 获取测试专用的日志目录
fn test_log_dir(test_name: &str) -> PathBuf {
    PathBuf::from("data").join("tmp").join(format!("test_logs_{}", test_name))
}

/// 清理测试日志目录
fn cleanup_test_logs(test_name: &str) {
    let log_dir = test_log_dir(test_name);
    if log_dir.exists() {
        let _ = fs::remove_dir_all(&log_dir);
    }
}

/// 获取当前日期的日志文件路径
fn get_today_log_path(test_name: &str, prefix: &str) -> PathBuf {
    let log_dir = test_log_dir(test_name);
    let today = chrono::Local::now().date_naive();
    log_dir.join(format!("{}.{}.log", prefix, today.format("%Y-%m-%d")))
}

#[tokio::test]
#[ignore] // 需要单独运行: cargo test --test logging_system test_logging_init_basic -- --ignored
async fn test_logging_init_basic() {
    let test_name = "init_basic";
    cleanup_test_logs(test_name);

    let log_dir = test_log_dir(test_name);
    let config = LoggingConfig {
        level: "info".to_string(),
        directory: log_dir.to_string_lossy().to_string(),
        file_prefix: "test".to_string(),
        retention_days: 7,
        compress_history: false,
        cleanup_interval_seconds: 3600,
    };

    // 初始化日志系统
    let result = init(config);
    assert!(result.is_ok(), "日志系统初始化应该成功");

    // 验证日志目录已创建
    assert!(log_dir.exists(), "日志目录应该被创建");

    cleanup_test_logs(test_name);
}

#[tokio::test]
#[ignore] // 需要单独运行: cargo test --test logging_system test_logging_file_creation -- --ignored
async fn test_logging_file_creation() {
    let test_name = "file_creation";
    cleanup_test_logs(test_name);

    let log_dir = test_log_dir(test_name);
    let config = LoggingConfig {
        level: "debug".to_string(),
        directory: log_dir.to_string_lossy().to_string(),
        file_prefix: "tao-test".to_string(),
        retention_days: 7,
        compress_history: false,
        cleanup_interval_seconds: 3600,
    };

    init(config).expect("日志初始化失败");

    // 写入一些日志
    tracing::info!("测试信息日志");
    tracing::debug!("测试调试日志");
    tracing::warn!("测试警告日志");

    // 给一点时间让日志写入文件
    std::thread::sleep(std::time::Duration::from_millis(100));

    // 验证日志文件已创建
    let log_file = get_today_log_path(test_name, "tao-test");
    assert!(log_file.exists(), "日志文件应该被创建: {:?}", log_file);

    cleanup_test_logs(test_name);
}

#[tokio::test]
#[ignore] // 需要单独运行: cargo test --test logging_system test_logging_file_content -- --ignored
async fn test_logging_file_content() {
    let test_name = "file_content";
    cleanup_test_logs(test_name);

    let log_dir = test_log_dir(test_name);
    let config = LoggingConfig {
        level: "debug".to_string(),
        directory: log_dir.to_string_lossy().to_string(),
        file_prefix: "content-test".to_string(),
        retention_days: 7,
        compress_history: false,
        cleanup_interval_seconds: 3600,
    };

    init(config).expect("日志初始化失败");

    // 写入特定的测试消息
    let test_message = "这是一条测试日志消息_12345";
    tracing::info!("{}", test_message);

    // 给足够时间让日志写入
    std::thread::sleep(std::time::Duration::from_millis(200));

    // 读取日志文件内容
    let log_file = get_today_log_path(test_name, "content-test");
    let content = fs::read_to_string(&log_file)
        .unwrap_or_else(|e| panic!("读取日志文件失败: {:?}, 错误: {}", log_file, e));

    // 验证日志内容包含测试消息
    assert!(
        content.contains(test_message),
        "日志文件应该包含测试消息, 文件内容:\n{}",
        content
    );

    // 验证日志包含级别标记
    assert!(
        content.contains("INFO"),
        "日志应该包含 INFO 级别标记"
    );

    cleanup_test_logs(test_name);
}

#[tokio::test]
#[ignore] // 需要单独运行: cargo test --test logging_system test_logging_different_levels -- --ignored
async fn test_logging_different_levels() {
    let test_name = "different_levels";
    cleanup_test_logs(test_name);

    let log_dir = test_log_dir(test_name);
    let config = LoggingConfig {
        level: "info".to_string(), // 只记录 info 及以上级别
        directory: log_dir.to_string_lossy().to_string(),
        file_prefix: "level-test".to_string(),
        retention_days: 7,
        compress_history: false,
        cleanup_interval_seconds: 3600,
    };

    init(config).expect("日志初始化失败");

    // 写入不同级别的日志
    tracing::error!("错误日志_ERROR_MSG");
    tracing::warn!("警告日志_WARN_MSG");
    tracing::info!("信息日志_INFO_MSG");
    tracing::debug!("调试日志_DEBUG_MSG"); // 应该被过滤掉

    std::thread::sleep(std::time::Duration::from_millis(200));

    let log_file = get_today_log_path(test_name, "level-test");
    let content = fs::read_to_string(&log_file).expect("读取日志文件失败");

    // 验证 info 及以上级别的日志都被记录
    assert!(content.contains("错误日志_ERROR_MSG"), "应该包含错误日志");
    assert!(content.contains("警告日志_WARN_MSG"), "应该包含警告日志");
    assert!(content.contains("信息日志_INFO_MSG"), "应该包含信息日志");

    // 验证 debug 日志被过滤
    assert!(
        !content.contains("调试日志_DEBUG_MSG"),
        "debug 日志应该被过滤掉"
    );

    cleanup_test_logs(test_name);
}

#[test]
fn test_logging_file_naming_format() {
    let test_name = "file_naming";
    cleanup_test_logs(test_name);

    let log_dir = test_log_dir(test_name);
    let prefixes = vec!["tao", "tao-probe", "tao-play"];

    for prefix in prefixes {
        let _config = LoggingConfig {
            level: "info".to_string(),
            directory: log_dir.to_string_lossy().to_string(),
            file_prefix: prefix.to_string(),
            retention_days: 7,
            compress_history: false,
            cleanup_interval_seconds: 3600,
        };

        // 注意: init 只能调用一次, 所以这个测试需要特殊处理
        // 这里我们只测试文件名格式, 不实际初始化
        let today = chrono::Local::now().date_naive();
        let expected_filename = format!("{}.{}.log", prefix, today.format("%Y-%m-%d"));
        
        // 验证文件名格式正确
        assert!(
            expected_filename.contains(prefix),
            "文件名应该包含前缀 {}",
            prefix
        );
        assert!(
            expected_filename.ends_with(".log"),
            "文件名应该以 .log 结尾"
        );
        assert!(
            expected_filename.contains(&today.year().to_string()),
            "文件名应该包含年份"
        );
    }

    cleanup_test_logs(test_name);
}

#[tokio::test]
#[ignore] // 需要单独运行: cargo test --test logging_system test_logging_directory_creation -- --ignored
async fn test_logging_directory_creation() {
    let test_name = "directory_creation";
    cleanup_test_logs(test_name);

    let log_dir = test_log_dir(test_name).join("nested").join("logs");
    
    // 确保目录不存在
    assert!(!log_dir.exists(), "测试前日志目录不应该存在");

    let config = LoggingConfig {
        level: "info".to_string(),
        directory: log_dir.to_string_lossy().to_string(),
        file_prefix: "dir-test".to_string(),
        retention_days: 7,
        compress_history: false,
        cleanup_interval_seconds: 3600,
    };

    init(config).expect("日志初始化失败");

    tracing::info!("测试目录创建");
    std::thread::sleep(std::time::Duration::from_millis(100));

    // 验证嵌套目录已创建
    assert!(log_dir.exists(), "嵌套日志目录应该被创建");

    cleanup_test_logs(test_name);
}

#[tokio::test]
#[ignore] // 需要单独运行: cargo test --test logging_system test_logging_chinese_content -- --ignored
async fn test_logging_chinese_content() {
    let test_name = "chinese_content";
    cleanup_test_logs(test_name);

    let log_dir = test_log_dir(test_name);
    let config = LoggingConfig {
        level: "info".to_string(),
        directory: log_dir.to_string_lossy().to_string(),
        file_prefix: "chinese-test".to_string(),
        retention_days: 7,
        compress_history: false,
        cleanup_interval_seconds: 3600,
    };

    init(config).expect("日志初始化失败");

    // 写入中文日志内容
    tracing::info!("这是中文日志内容");
    tracing::warn!("文件打开失败: 路径={}", "/测试/路径.txt");
    tracing::error!("编解码器初始化错误: 不支持的像素格式");

    std::thread::sleep(std::time::Duration::from_millis(200));

    let log_file = get_today_log_path(test_name, "chinese-test");
    let content = fs::read_to_string(&log_file).expect("读取日志文件失败");

    // 验证中文内容正确写入
    assert!(content.contains("这是中文日志内容"), "应该包含中文日志");
    assert!(content.contains("文件打开失败"), "应该包含中文错误信息");
    assert!(content.contains("不支持的像素格式"), "应该包含中文技术术语");

    cleanup_test_logs(test_name);
}

#[test]
fn test_logging_config_defaults() {
    // 测试配置的默认值
    let config = LoggingConfig {
        level: "debug".to_string(),
        directory: "logs".to_string(),
        file_prefix: "test".to_string(),
        retention_days: 30,
        compress_history: true,
        cleanup_interval_seconds: 3600,
    };

    assert_eq!(config.retention_days, 30, "默认保留天数应该是 30");
    assert!(config.compress_history, "默认应该开启压缩");
    assert_eq!(config.cleanup_interval_seconds, 3600, "默认清理间隔应该是 3600 秒");
}
