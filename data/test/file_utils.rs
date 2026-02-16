//! 测试文件和临时文件使用示例
//!
//! 本文件演示如何在 Tao 项目中正确使用测试文件和临时文件

use std::path::PathBuf;
use std::fs;

/// 获取样本文件目录
pub fn get_samples_dir() -> PathBuf {
    let base_dir = std::env::var("TAO_DATA_DIR").unwrap_or_else(|_| "data".to_string());
    PathBuf::from(base_dir).join("samples")
}

/// 获取视频样本文件目录
pub fn get_video_samples_dir() -> PathBuf {
    get_samples_dir().join("video")
}

/// 获取音频样本文件目录
pub fn get_audio_samples_dir() -> PathBuf {
    get_samples_dir().join("audio")
}

/// 获取临时文件目录
pub fn get_temp_dir() -> PathBuf {
    let base_dir = std::env::var("TAO_DATA_DIR").unwrap_or_else(|_| "data".to_string());
    PathBuf::from(base_dir).join("tmp")
}

/// 创建临时文件
pub fn create_temp_file(name: &str) -> std::io::Result<PathBuf> {
    let temp_dir = get_temp_dir();
    fs::create_dir_all(&temp_dir)?;
    
    let file_name = format!("tmp_{}_{}", name, std::process::id());
    let temp_file = temp_dir.join(file_name);
    
    Ok(temp_file)
}

/// 示例：在测试中使用样本文件
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_file_access() {
        let video_dir = get_video_samples_dir();
        println!("视频样本目录: {:?}", video_dir);
        
        // 检查目录是否存在
        assert!(video_dir.exists());
    }

    #[test]
    fn test_temp_file_creation() {
        let temp_file = create_temp_file("test").unwrap();
        println!("临时文件: {:?}", temp_file);
        
        // 确保文件在正确的目录中
        assert!(temp_file.starts_with("data/tmp"));
        assert!(temp_file.file_name().unwrap().to_str().unwrap().starts_with("tmp_test_"));
        
        // 清理临时文件
        if temp_file.exists() {
            fs::remove_file(&temp_file).unwrap();
        }
    }
}

/// 示例：在解码器测试中使用样本文件
pub fn get_theora_sample_path() -> PathBuf {
    get_video_samples_dir().join("theora_test.ogg")
}

/// 示例：创建解码测试的临时输出文件
pub fn create_decoder_output_temp_file(codec_name: &str) -> std::io::Result<PathBuf> {
    create_temp_file(&format!("decoder_{}_output", codec_name))
}
