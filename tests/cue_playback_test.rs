//! CUE 文件播放测试
//!
//! 验证 CUE sheet 解析和章节信息提取功能.

use tao_format::demuxer::Demuxer;
use tao_format::format_id::FormatId;
use tao_format::io::IoContext;
use tao_format::registry::FormatRegistry;
use std::io::Write;
use std::path::PathBuf;

/// 创建测试目录和文件
fn setup_test_files() -> (PathBuf, PathBuf, tempfile::TempDir) {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let wav_path = temp_dir.path().join("test.wav");
    let cue_path = temp_dir.path().join("test.cue");
    
    // 创建 WAV 文件
    create_test_wav_at(&wav_path);
    
    // 创建 CUE 文件，使用绝对路径
    let wav_path_str = wav_path.to_str().unwrap();
    create_test_cue_at(&cue_path, wav_path_str);
    
    (wav_path, cue_path, temp_dir)
}

/// 在指定路径创建 WAV 文件
fn create_test_wav_at(path: &PathBuf) {
    let mut wav_file = std::fs::File::create(path).unwrap();
    
    // 创建一个 1 秒的静音 WAV 文件
    let sample_rate = 44100u32;
    let channels = 2u16;
    let bits_per_sample = 16u16;
    let duration_secs = 60; // 60 秒，足够包含所有 track
    let num_samples = sample_rate * duration_secs;
    let data_size = num_samples * channels as u32 * (bits_per_sample / 8) as u32;
    let file_size = 36 + data_size; // RIFF header + fmt + data
    
    // RIFF header
    wav_file.write_all(b"RIFF").unwrap();
    wav_file.write_all(&file_size.to_le_bytes()).unwrap();
    wav_file.write_all(b"WAVE").unwrap();
    
    // fmt 块
    wav_file.write_all(b"fmt ").unwrap();
    wav_file.write_all(&16u32.to_le_bytes()).unwrap(); // fmt 块大小
    wav_file.write_all(&1u16.to_le_bytes()).unwrap();  // PCM 格式
    wav_file.write_all(&channels.to_le_bytes()).unwrap();
    wav_file.write_all(&sample_rate.to_le_bytes()).unwrap();
    let byte_rate = sample_rate * channels as u32 * (bits_per_sample / 8) as u32;
    wav_file.write_all(&byte_rate.to_le_bytes()).unwrap();
    let block_align = channels * (bits_per_sample / 8);
    wav_file.write_all(&block_align.to_le_bytes()).unwrap();
    wav_file.write_all(&bits_per_sample.to_le_bytes()).unwrap();
    
    // data 块
    wav_file.write_all(b"data").unwrap();
    wav_file.write_all(&data_size.to_le_bytes()).unwrap();
    
    // 填充静音数据
    let silence_chunk = vec![0u8; 4096];
    let mut remaining = data_size as usize;
    while remaining > 0 {
        let to_write = remaining.min(silence_chunk.len());
        wav_file.write_all(&silence_chunk[..to_write]).unwrap();
        remaining -= to_write;
    }
    
    wav_file.flush().unwrap();
}

/// 在指定路径创建 CUE 文件
fn create_test_cue_at(path: &PathBuf, wav_filename: &str) {
    let mut cue_file = std::fs::File::create(path).unwrap();
    
    let cue_content = format!(r#"REM GENRE "Pop"
REM DATE "2000"
PERFORMER "Jay Chou"
TITLE "Jay"
FILE "{}" WAVE
  TRACK 01 AUDIO
    TITLE "可爱女人"
    PERFORMER "Jay Chou"
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    TITLE "完美主义"
    PERFORMER "Jay Chou"
    INDEX 01 00:15:00
  TRACK 03 AUDIO
    TITLE "星晴"
    PERFORMER "Jay Chou"
    INDEX 01 00:30:00
"#, wav_filename);
    
    cue_file.write_all(cue_content.as_bytes()).unwrap();
    cue_file.flush().unwrap();
}

#[test]
fn test_wav_file_alone() {
    // 测试单独的 WAV 文件是否能被识别
    let temp_dir = tempfile::TempDir::new().unwrap();
    let wav_path = temp_dir.path().join("test.wav");
    create_test_wav_at(&wav_path);
    
    let wav_path_str = wav_path.to_str().unwrap();
    let mut registry = FormatRegistry::new();
    tao_format::register_all(&mut registry);
    
    let mut io = IoContext::open_read(wav_path_str).unwrap();
    let probe_result = registry.probe_input(&mut io, Some(wav_path_str)).unwrap();
    
    assert_eq!(probe_result.format_id, FormatId::Wav);
    println!("WAV 文件识别成功！");
}

#[test]
fn test_cue_parsing() {
    // 初始化日志
    let _ = env_logger::builder().is_test(true).try_init();
    
    // 创建测试文件
    let (_wav_path, cue_path, _temp_dir) = setup_test_files();
    let cue_path_str = cue_path.to_str().unwrap();
    

    // 打印 CUE 文件内容以供调试
    println!("CUE 文件路径: {}", cue_path_str);
    let cue_content = std::fs::read_to_string(&cue_path).unwrap();
    println!("CUE 文件内容:\n{}", cue_content);
    
    // 初始化 FormatRegistry
    let mut registry = FormatRegistry::new();
    tao_format::register_all(&mut registry);
    
    // 打开 CUE 文件
    let mut io = IoContext::open_read(cue_path_str).unwrap();
    let probe_result = registry.probe_input(&mut io, Some(cue_path_str)).unwrap();
    
    assert_eq!(probe_result.format_id, FormatId::Cue, "应该识别为 CUE 格式");
    
    // 创建 demuxer 并打开
    let mut demuxer = registry.create_demuxer(probe_result.format_id).unwrap();
    demuxer.open(&mut io).unwrap();
    
    // 验证章节信息
    let chapters = demuxer.chapters();
    assert_eq!(chapters.len(), 3, "应该有 3 个章节");
    
    // 验证第一个章节
    let ch1 = &chapters[0];
    assert_eq!(ch1.start_time, Some(0.0));
    let title1 = ch1.metadata.iter()
        .find(|(k, _)| k == "title")
        .map(|(_, v)| v.as_str());
    assert_eq!(title1, Some("可爱女人"));
    
    // 验证第二个章节
    let ch2 = &chapters[1];
    assert_eq!(ch2.start_time, Some(15.0));
    assert_eq!(ch2.end_time, Some(30.0));
    let title2 = ch2.metadata.iter()
        .find(|(k, _)| k == "title")
        .map(|(_, v)| v.as_str());
    assert_eq!(title2, Some("完美主义"));
    
    // 验证第三个章节
    let ch3 = &chapters[2];
    assert_eq!(ch3.start_time, Some(30.0));
    let title3 = ch3.metadata.iter()
        .find(|(k, _)| k == "title")
        .map(|(_, v)| v.as_str());
    assert_eq!(title3, Some("星晴"));
    
    // 验证全局元数据
    let metadata = demuxer.metadata();
    let album = metadata.iter()
        .find(|(k, _)| k == "album")
        .map(|(_, v)| v.as_str());
    assert_eq!(album, Some("Jay"));
    
    let artist = metadata.iter()
        .find(|(k, _)| k == "artist")
        .map(|(_, v)| v.as_str());
    assert_eq!(artist, Some("Jay Chou"));
    
    println!("CUE 解析测试通过!");
}

#[test]
fn test_cue_stream_access() {
    // 创建测试文件
    let (_wav_path, cue_path, _temp_dir) = setup_test_files();
    let cue_path_str = cue_path.to_str().unwrap();
    
    // 初始化并打开
    let mut registry = FormatRegistry::new();
    tao_format::register_all(&mut registry);
    let mut io = IoContext::open_read(cue_path_str).unwrap();
    let demuxer = registry.open_input(&mut io, Some(cue_path_str)).unwrap();
    
    // 验证流信息 (应该是底层 WAV 文件的流)
    let streams = demuxer.streams();
    assert_eq!(streams.len(), 1, "应该有 1 个音频流");
    assert_eq!(streams[0].media_type, tao_core::MediaType::Audio);
    
    println!("CUE 流访问测试通过!");
}
