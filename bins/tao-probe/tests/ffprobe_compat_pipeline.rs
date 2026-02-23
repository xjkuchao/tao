//! ffprobe 兼容流水线测试.

use std::process::Command;
use std::sync::{Mutex, OnceLock};

use tempfile::tempdir;

static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug)]
struct CmdResult {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_tao_probe(args: &[&str]) -> Result<CmdResult, String> {
    let output = Command::new(env!("CARGO_BIN_EXE_tao-probe"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .map_err(|e| format!("启动 tao-probe 失败: {}", e))?;

    Ok(CmdResult {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn run_ffprobe(args: &[&str]) -> Result<CmdResult, String> {
    let output = Command::new("ffprobe")
        .args(args)
        .output()
        .map_err(|e| format!("启动 ffprobe 失败: {}", e))?;

    Ok(CmdResult {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn has_ffprobe() -> bool {
    Command::new("ffprobe")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn normalize_dynamic_text(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'0' && i + 1 < bytes.len() && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X')
        {
            let mut j = i + 2;
            while j < bytes.len() && (bytes[j] as char).is_ascii_hexdigit() {
                j += 1;
            }
            if j > i + 2 {
                out.push_str("0xADDR");
                i = j;
                continue;
            }
        }

        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn make_minimal_wav() -> Result<(tempfile::TempDir, String), String> {
    let dir = tempdir().map_err(|e| format!("创建临时目录失败: {}", e))?;
    let file = dir.path().join("sample.wav");

    // 8kHz/16bit/mono, 16 个采样点静音.
    let sample_rate: u32 = 8_000;
    let channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let samples: u32 = 16;
    let block_align = channels * (bits_per_sample / 8);
    let byte_rate = sample_rate * block_align as u32;
    let data_size = samples * block_align as u32;
    let riff_size = 36 + data_size;

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_size.to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&bits_per_sample.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());
    bytes.resize((44 + data_size) as usize, 0u8);

    std::fs::write(&file, bytes).map_err(|e| format!("写入 WAV 失败: {}", e))?;
    Ok((dir, file.to_string_lossy().to_string()))
}

#[test]
fn test_parser_unknown_option_alignment() {
    let _guard = TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    if !has_ffprobe() {
        return;
    }

    let args = ["-v", "error", "-foo", "bar"];
    let tao = run_tao_probe(&args).expect("tao-probe 执行失败");
    let ff = run_ffprobe(&args).expect("ffprobe 执行失败");

    assert_eq!(tao.code, ff.code, "退出码应与 ffprobe 一致");
    assert_eq!(
        tao.stderr.trim(),
        ff.stderr.trim(),
        "未知参数错误文案应与 ffprobe 一致"
    );
}

#[test]
fn test_parser_missing_argument_alignment() {
    let _guard = TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    if !has_ffprobe() {
        return;
    }

    let args = ["-v", "error", "-show_entries"];
    let tao = run_tao_probe(&args).expect("tao-probe 执行失败");
    let ff = run_ffprobe(&args).expect("ffprobe 执行失败");

    assert_eq!(tao.code, ff.code, "退出码应一致");
    assert_eq!(
        tao.stderr.trim(),
        ff.stderr.trim(),
        "缺失参数错误文案应一致"
    );
}

#[test]
fn test_writer_json_basic_output() {
    let _guard = TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let (_dir, wav_path) = make_minimal_wav().expect("构造 WAV 样本失败");
    let args = [
        "-v",
        "error",
        "-show_format",
        "-show_streams",
        "-of",
        "json",
        &wav_path,
    ];
    let tao = run_tao_probe(&args).expect("tao-probe 执行失败");

    assert_eq!(tao.code, 0, "show_format/show_streams JSON 输出应成功");
    let parsed: serde_json::Value =
        serde_json::from_str(&tao.stdout).expect("stdout 应为合法 JSON");

    assert!(
        parsed.get("format").is_some(),
        "JSON 输出应包含 format section"
    );
    assert!(
        parsed.get("streams").is_some(),
        "JSON 输出应包含 streams section"
    );
}

#[test]
fn test_show_packets_alignment_with_ffprobe() {
    let _guard = TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    if !has_ffprobe() {
        return;
    }

    let (_dir, wav_path) = make_minimal_wav().expect("构造 WAV 样本失败");
    let args = ["-v", "error", "-show_packets", &wav_path];
    let tao = run_tao_probe(&args).expect("tao-probe 执行失败");
    let ff = run_ffprobe(&args).expect("ffprobe 执行失败");

    assert_eq!(tao.code, ff.code, "show_packets 退出码应与 ffprobe 一致");
    assert_eq!(
        tao.stdout, ff.stdout,
        "show_packets stdout 应与 ffprobe 一致"
    );
    assert_eq!(
        normalize_dynamic_text(&tao.stderr),
        normalize_dynamic_text(&ff.stderr),
        "show_packets stderr 应与 ffprobe 一致（忽略动态地址）"
    );
}

#[test]
fn test_select_streams_audio_first_matches_wav() {
    let _guard = TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let (_dir, wav_path) = make_minimal_wav().expect("构造 WAV 样本失败");
    let args = [
        "-v",
        "error",
        "-show_streams",
        "-select_streams",
        "a:0",
        "-of",
        "json",
        &wav_path,
    ];
    let tao = run_tao_probe(&args).expect("tao-probe 执行失败");

    assert_eq!(tao.code, 0, "select_streams 应成功执行");
    let parsed: serde_json::Value =
        serde_json::from_str(&tao.stdout).expect("stdout 应为合法 JSON");
    let stream = parsed
        .get("streams")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .expect("JSON 输出应包含 streams[0]");
    assert_eq!(
        stream.get("codec_type").and_then(|v| v.as_str()),
        Some("audio"),
        "a:0 选择后应为音频流"
    );
}

#[test]
fn test_show_entries_filters_stream_fields() {
    let _guard = TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let (_dir, wav_path) = make_minimal_wav().expect("构造 WAV 样本失败");
    let args = [
        "-v",
        "error",
        "-show_streams",
        "-show_entries",
        "stream=index,codec_type",
        "-of",
        "json",
        &wav_path,
    ];
    let tao = run_tao_probe(&args).expect("tao-probe 执行失败");
    assert_eq!(tao.code, 0, "show_entries 应成功执行");

    let parsed: serde_json::Value =
        serde_json::from_str(&tao.stdout).expect("stdout 应为合法 JSON");
    let stream = parsed
        .get("streams")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .expect("JSON 输出应包含 streams[0]");
    assert!(
        stream.get("index").is_some(),
        "stream section 应保留 index 字段"
    );
    assert!(
        stream.get("codec_type").is_some(),
        "stream section 应保留 codec_type 字段"
    );
    assert!(
        stream.get("codec_name").is_none(),
        "show_entries 过滤后不应包含 codec_name 字段"
    );
}
