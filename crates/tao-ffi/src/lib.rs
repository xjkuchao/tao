//! # tao-ffi
//!
//! Tao 多媒体框架 C FFI 导出层.
//!
//! 本 crate 负责将 Tao 的 Rust API 导出为 C 兼容的函数接口,
//! 编译为 DLL (Windows) / SO (Linux) / dylib (macOS) 供 C/C++ 等语言调用.
//!
//! # 命名规范
//!
//! 所有导出函数以 `tao_` 前缀命名, 例如:
//! - `tao_version()` - 获取版本号
//! - `tao_codec_find_decoder()` - 查找解码器
//!
//! # 内存管理
//!
//! - 由 Tao 分配的内存必须通过对应的 `tao_*_free()` 函数释放
//! - 调用方分配的缓冲区由调用方负责释放

use std::os::raw::c_char;

/// 获取 Tao 版本号字符串
///
/// 返回的字符串指针为静态分配, 无需释放.
///
/// # Safety
///
/// 返回的指针在程序生命周期内有效.
#[unsafe(no_mangle)]
pub extern "C" fn tao_version() -> *const c_char {
    // 安全: 字面量末尾包含 \0
    c"0.1.0".as_ptr()
}

/// 获取 Tao 版本号的数字表示
///
/// 格式: (主版本 << 16) | (次版本 << 8) | 修订版本
#[unsafe(no_mangle)]
pub extern "C" fn tao_version_int() -> u32 {
    let (major, minor, patch): (u32, u32, u32) = (0, 1, 0);
    (major << 16) | (minor << 8) | patch
}

/// 获取 Tao 构建配置信息
///
/// # Safety
///
/// 返回的指针在程序生命周期内有效.
#[unsafe(no_mangle)]
pub extern "C" fn tao_build_info() -> *const c_char {
    c"tao 0.1.0 -- 纯 Rust 多媒体框架".as_ptr()
}

/// 初始化 Tao 库
///
/// 在使用其他 Tao 函数前必须先调用此函数.
/// 可安全多次调用.
#[unsafe(no_mangle)]
pub extern "C" fn tao_init() {
    // TODO: 初始化全局注册表、日志系统等
}

/// 关闭 Tao 库, 释放全局资源
#[unsafe(no_mangle)]
pub extern "C" fn tao_shutdown() {
    // TODO: 释放全局资源
}

// TODO: 后续添加更多 FFI 导出函数:
// - tao_demuxer_open()
// - tao_demuxer_read_packet()
// - tao_decoder_create()
// - tao_decoder_send_packet()
// - tao_decoder_receive_frame()
// - tao_encoder_create()
// - tao_muxer_create()
// 等等
