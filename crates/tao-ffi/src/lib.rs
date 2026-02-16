//! # tao-ffi
//!
//! Tao 多媒体框架 C FFI 导出层.
//!
//! 本 crate 负责将 Tao 的 Rust API 导出为 C 兼容的函数接口,
//! 编译为 DLL (Windows) / SO (Linux) / dylib (macOS) 供 C/C++ 等语言调用.
//!
//! # 命名规范
//!
//! 所有导出函数以 `tao_` 前缀命名.
//!
//! # 内存管理
//!
//! - 由 Tao 分配的内存必须通过对应的 `tao_*_free()` 函数释放
//! - 调用方分配的缓冲区由调用方负责释放

use std::ffi::CStr;
use std::os::raw::{c_char, c_int};
use std::ptr;

use tao_codec::codec_parameters::{AudioCodecParams, CodecParamsType};
use tao_codec::{CodecId, CodecParameters, CodecRegistry, Decoder, Encoder, Frame, Packet};
use tao_core::{ChannelLayout, MediaType, SampleFormat, TaoError};
use tao_format::{FormatRegistry, IoContext};
use tao_resample::ResampleContext;
use tao_scale::{ScaleAlgorithm, ScaleContext};

// =============================================================================
// 错误码 (对应 C 头文件中的 #define)
// =============================================================================

pub const TAO_OK: c_int = 0;
pub const TAO_ERROR: c_int = -1;
pub const TAO_EOF: c_int = -2;
pub const TAO_NEED_MORE_DATA: c_int = -3;

// =============================================================================
//  opaque 指针类型
// =============================================================================

/// 格式上下文 (封装 registry + demuxer + io)
pub struct TaoFormatContext {
    pub(crate) io: IoContext,
    pub(crate) demuxer: Box<dyn tao_format::Demuxer>,
}

/// 编解码器上下文 (封装 decoder 或 encoder)
pub enum TaoCodecContextInner {
    Decoder(Box<dyn Decoder>),
    Encoder(Box<dyn Encoder>),
}

pub struct TaoCodecContext {
    pub(crate) inner: TaoCodecContextInner,
}

/// 压缩数据包
pub struct TaoPacket(pub(crate) Packet);

/// 解码帧
pub struct TaoFrame(pub(crate) Frame);

/// 图像缩放上下文
pub struct TaoScaleContext(pub(crate) ScaleContext);

/// 音频重采样上下文
pub struct TaoResampleContext(pub(crate) ResampleContext);

// =============================================================================
// CodecId 映射 (C int <-> Rust CodecId)
// =============================================================================

fn codec_id_from_int(id: c_int) -> Option<CodecId> {
    match id {
        0 => Some(CodecId::None),
        1 => Some(CodecId::H264),
        2 => Some(CodecId::H265),
        3 => Some(CodecId::Vp8),
        4 => Some(CodecId::Vp9),
        5 => Some(CodecId::Av1),
        6 => Some(CodecId::Mpeg1Video),
        7 => Some(CodecId::Mpeg2Video),
        8 => Some(CodecId::Mpeg4),
        9 => Some(CodecId::Theora),
        10 => Some(CodecId::Mjpeg),
        11 => Some(CodecId::Png),
        12 => Some(CodecId::RawVideo),
        13 => Some(CodecId::Aac),
        14 => Some(CodecId::Mp3),
        15 => Some(CodecId::Mp2),
        16 => Some(CodecId::Opus),
        17 => Some(CodecId::Vorbis),
        18 => Some(CodecId::Flac),
        19 => Some(CodecId::Alac),
        20 => Some(CodecId::PcmS16le),
        21 => Some(CodecId::PcmS16be),
        22 => Some(CodecId::PcmS24le),
        23 => Some(CodecId::PcmS32le),
        24 => Some(CodecId::PcmF32le),
        25 => Some(CodecId::PcmU8),
        26 => Some(CodecId::Ac3),
        27 => Some(CodecId::Eac3),
        28 => Some(CodecId::Dts),
        29 => Some(CodecId::Srt),
        30 => Some(CodecId::Ass),
        31 => Some(CodecId::Webvtt),
        32 => Some(CodecId::DvdSubtitle),
        33 => Some(CodecId::HdmvPgsSubtitle),
        _ => None,
    }
}

fn codec_id_to_int(id: CodecId) -> c_int {
    match id {
        CodecId::None => 0,
        CodecId::H264 => 1,
        CodecId::H265 => 2,
        CodecId::Vp8 => 3,
        CodecId::Vp9 => 4,
        CodecId::Av1 => 5,
        CodecId::Mpeg1Video => 6,
        CodecId::Mpeg2Video => 7,
        CodecId::Mpeg4 => 8,
        CodecId::Theora => 9,
        CodecId::Mjpeg => 10,
        CodecId::Png => 11,
        CodecId::RawVideo => 12,
        CodecId::Aac => 13,
        CodecId::Mp3 => 14,
        CodecId::Mp2 => 15,
        CodecId::Opus => 16,
        CodecId::Vorbis => 17,
        CodecId::Flac => 18,
        CodecId::Alac => 19,
        CodecId::PcmS16le => 20,
        CodecId::PcmS16be => 21,
        CodecId::PcmS24le => 22,
        CodecId::PcmS32le => 23,
        CodecId::PcmF32le => 24,
        CodecId::PcmU8 => 25,
        CodecId::Ac3 => 26,
        CodecId::Eac3 => 27,
        CodecId::Dts => 28,
        CodecId::Srt => 29,
        CodecId::Ass => 30,
        CodecId::Webvtt => 31,
        CodecId::DvdSubtitle => 32,
        CodecId::HdmvPgsSubtitle => 33,
        _ => 0, // 未知编解码器映射到 None
    }
}

fn media_type_to_int(mt: MediaType) -> c_int {
    match mt {
        MediaType::Video => 0,
        MediaType::Audio => 1,
        MediaType::Subtitle => 2,
        MediaType::Data => 3,
        MediaType::Attachment => 4,
    }
}

fn tao_error_to_int(e: &TaoError) -> c_int {
    match e {
        TaoError::Eof => TAO_EOF,
        TaoError::NeedMoreData => TAO_NEED_MORE_DATA,
        _ => TAO_ERROR,
    }
}

// =============================================================================
// Version / Init
// =============================================================================

/// 获取 Tao 版本号字符串
///
/// 返回的字符串指针为静态分配, 无需释放.
///
/// # Safety
///
/// 返回的指针在程序生命周期内有效.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_version() -> *const c_char {
    // SAFETY: 字面量末尾包含 \0
    c"0.1.0".as_ptr()
}

/// 获取 Tao 版本号的数字表示
///
/// 格式: (主版本 << 16) | (次版本 << 8) | 修订版本
///
/// # Safety
///
/// 无特殊安全要求.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_version_int() -> u32 {
    let (major, minor, patch): (u32, u32, u32) = (0, 1, 0);
    (major << 16) | (minor << 8) | patch
}

/// 获取 Tao 构建配置信息
///
/// # Safety
///
/// 返回的指针在程序生命周期内有效.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_build_info() -> *const c_char {
    c"tao 0.1.0 -- 纯 Rust 多媒体框架".as_ptr()
}

/// 初始化 Tao 库
///
/// 在使用其他 Tao 函数前必须先调用此函数. 可安全多次调用.
///
/// # Safety
///
/// 无特殊安全要求.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_init() {
    // 初始化全局注册表、日志系统等 (当前为空)
}

/// 关闭 Tao 库, 释放全局资源
///
/// # Safety
///
/// 无特殊安全要求.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_shutdown() {
    // 释放全局资源 (当前为空)
}

// =============================================================================
// Format (Demuxer)
// =============================================================================

/// 打开输入文件并探测格式
///
/// # Safety
///
/// filename 必须指向有效的以 null 结尾的 C 字符串.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_format_open_input(filename: *const c_char) -> *mut TaoFormatContext {
    if filename.is_null() {
        return ptr::null_mut();
    }

    let filename_str = match unsafe { CStr::from_ptr(filename) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    let io = match IoContext::open_read(filename_str) {
        Ok(io) => io,
        Err(_) => return ptr::null_mut(),
    };

    let mut format_registry = FormatRegistry::new();
    tao_format::register_all(&mut format_registry);

    let mut io = io;
    let demuxer = match format_registry.open_input(&mut io, Some(filename_str)) {
        Ok(d) => d,
        Err(_) => return ptr::null_mut(),
    };

    let ctx = TaoFormatContext { io, demuxer };
    Box::into_raw(Box::new(ctx))
}

/// 读取下一个数据包
///
/// 成功时 *packet 指向新分配的 TaoPacket, 调用方必须使用 tao_packet_free 释放.
///
/// # Safety
///
/// ctx 和 packet 必须非空. packet 指向有效的指针变量.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_format_read_packet(
    ctx: *mut TaoFormatContext,
    packet: *mut *mut TaoPacket,
) -> c_int {
    if ctx.is_null() || packet.is_null() {
        return TAO_ERROR;
    }

    let ctx = unsafe { &mut *ctx };
    let pkt = match ctx.demuxer.read_packet(&mut ctx.io) {
        Ok(p) => p,
        Err(e) => return tao_error_to_int(&e),
    };

    let tao_pkt = Box::new(TaoPacket(pkt));
    unsafe {
        *packet = Box::into_raw(tao_pkt);
    }
    TAO_OK
}

/// 获取流数量
///
/// # Safety
///
/// ctx 必须为由 tao_format_open_input 返回的有效指针, 或为 null (返回 -1).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_format_get_stream_count(ctx: *const TaoFormatContext) -> c_int {
    if ctx.is_null() {
        return -1;
    }
    let ctx = unsafe { &*ctx };
    ctx.demuxer.streams().len() as c_int
}

/// 获取指定流的编解码器 ID
///
/// # Safety
///
/// ctx 必须为由 tao_format_open_input 返回的有效指针.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_format_get_stream_codec_id(
    ctx: *const TaoFormatContext,
    stream_index: c_int,
) -> c_int {
    if ctx.is_null() || stream_index < 0 {
        return -1;
    }
    let ctx = unsafe { &*ctx };
    let streams = ctx.demuxer.streams();
    let idx = stream_index as usize;
    if idx >= streams.len() {
        return -1;
    }
    codec_id_to_int(streams[idx].codec_id)
}

/// 获取指定流的媒体类型
///
/// 返回: 0=Video, 1=Audio, 2=Subtitle, 3=Data, 4=Attachment
///
/// # Safety
///
/// ctx 必须为由 tao_format_open_input 返回的有效指针.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_format_get_stream_media_type(
    ctx: *const TaoFormatContext,
    stream_index: c_int,
) -> c_int {
    if ctx.is_null() || stream_index < 0 {
        return -1;
    }
    let ctx = unsafe { &*ctx };
    let streams = ctx.demuxer.streams();
    let idx = stream_index as usize;
    if idx >= streams.len() {
        return -1;
    }
    media_type_to_int(streams[idx].media_type)
}

/// 关闭格式上下文并释放资源
///
/// # Safety
///
/// ctx 必须为由 tao_format_open_input 返回的有效指针, 调用后不可再使用.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_format_close(ctx: *mut TaoFormatContext) {
    if !ctx.is_null() {
        // SAFETY: 确保指针有效后 drop
        let _ = unsafe { Box::from_raw(ctx) };
    }
}

// =============================================================================
// Codec (Decoder/Encoder)
// =============================================================================

/// 创建解码器
///
/// # Safety
///
/// codec_id 必须为有效的编解码器 ID (见 CodecId 映射).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_codec_create_decoder(codec_id: c_int) -> *mut TaoCodecContext {
    let id = match codec_id_from_int(codec_id) {
        Some(id) => id,
        None => return ptr::null_mut(),
    };

    let mut registry = CodecRegistry::new();
    tao_codec::register_all(&mut registry);

    let decoder = match registry.create_decoder(id) {
        Ok(d) => d,
        Err(_) => return ptr::null_mut(),
    };

    let ctx = TaoCodecContext {
        inner: TaoCodecContextInner::Decoder(decoder),
    };
    Box::into_raw(Box::new(ctx))
}

/// 打开解码器
///
/// extra_data 可为 null (extra_data_size 此时应为 0).
///
/// # Safety
///
/// extra_data 若非 null 则必须指向至少 extra_data_size 字节的有效内存.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_codec_open_decoder(
    ctx: *mut TaoCodecContext,
    sample_rate: c_int,
    channels: c_int,
    extra_data: *const u8,
    extra_data_size: c_int,
) -> c_int {
    if ctx.is_null() || sample_rate <= 0 || channels <= 0 {
        return TAO_ERROR;
    }

    let ctx = unsafe { &mut *ctx };
    let TaoCodecContextInner::Decoder(decoder) = &mut ctx.inner else {
        return TAO_ERROR;
    };

    let extra = if extra_data.is_null() || extra_data_size <= 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(extra_data, extra_data_size as usize).to_vec() }
    };

    let params = CodecParameters {
        codec_id: decoder.codec_id(),
        extra_data: extra,
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: sample_rate as u32,
            channel_layout: ChannelLayout::from_channels(channels as u32),
            sample_format: SampleFormat::S16,
            frame_size: 0,
        }),
    };

    match decoder.open(&params) {
        Ok(()) => TAO_OK,
        Err(e) => tao_error_to_int(&e),
    }
}

/// 向解码器送入数据包
///
/// 送入 null 表示 flush.
///
/// # Safety
///
/// packet 若非 null 必须指向有效的 TaoPacket.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_codec_send_packet(
    ctx: *mut TaoCodecContext,
    packet: *const TaoPacket,
) -> c_int {
    if ctx.is_null() {
        return TAO_ERROR;
    }

    let ctx = unsafe { &mut *ctx };
    let TaoCodecContextInner::Decoder(decoder) = &mut ctx.inner else {
        return TAO_ERROR;
    };

    let pkt = if packet.is_null() {
        Packet::empty()
    } else {
        unsafe { (*packet).0.clone() }
    };

    match decoder.send_packet(&pkt) {
        Ok(()) => TAO_OK,
        Err(e) => tao_error_to_int(&e),
    }
}

/// 从解码器取出一帧
///
/// 成功时 *frame 指向新分配的 TaoFrame, 调用方必须使用 tao_frame_free 释放.
///
/// # Safety
///
/// ctx 和 frame 必须非空. frame 指向有效的指针变量.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_codec_receive_frame(
    ctx: *mut TaoCodecContext,
    frame: *mut *mut TaoFrame,
) -> c_int {
    if ctx.is_null() || frame.is_null() {
        return TAO_ERROR;
    }

    let ctx = unsafe { &mut *ctx };
    let TaoCodecContextInner::Decoder(decoder) = &mut ctx.inner else {
        return TAO_ERROR;
    };

    let f = match decoder.receive_frame() {
        Ok(f) => f,
        Err(e) => return tao_error_to_int(&e),
    };

    let tao_frame = Box::new(TaoFrame(f));
    unsafe {
        *frame = Box::into_raw(tao_frame);
    }
    TAO_OK
}

/// 创建编码器
///
/// # Safety
///
/// codec_id 必须为有效的编解码器 ID (见 CodecId 映射).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_codec_create_encoder(codec_id: c_int) -> *mut TaoCodecContext {
    let id = match codec_id_from_int(codec_id) {
        Some(id) => id,
        None => return ptr::null_mut(),
    };

    let mut registry = CodecRegistry::new();
    tao_codec::register_all(&mut registry);

    let encoder = match registry.create_encoder(id) {
        Ok(e) => e,
        Err(_) => return ptr::null_mut(),
    };

    let ctx = TaoCodecContext {
        inner: TaoCodecContextInner::Encoder(encoder),
    };
    Box::into_raw(Box::new(ctx))
}

/// 打开编码器
///
/// # Safety
///
/// ctx 必须为由 tao_codec_create_encoder 返回的有效指针.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_codec_open_encoder(
    ctx: *mut TaoCodecContext,
    sample_rate: c_int,
    channels: c_int,
) -> c_int {
    if ctx.is_null() || sample_rate <= 0 || channels <= 0 {
        return TAO_ERROR;
    }

    let ctx = unsafe { &mut *ctx };
    let TaoCodecContextInner::Encoder(encoder) = &mut ctx.inner else {
        return TAO_ERROR;
    };

    let params = CodecParameters {
        codec_id: encoder.codec_id(),
        extra_data: Vec::new(),
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: sample_rate as u32,
            channel_layout: ChannelLayout::from_channels(channels as u32),
            sample_format: SampleFormat::S16,
            frame_size: 0,
        }),
    };

    match encoder.open(&params) {
        Ok(()) => TAO_OK,
        Err(e) => tao_error_to_int(&e),
    }
}

/// 向编码器送入一帧
///
/// 送入 null 表示 flush.
///
/// # Safety
///
/// ctx 必须为由 tao_codec_create_encoder 返回的有效指针. frame 若非 null 必须指向有效的 TaoFrame.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_codec_send_frame(
    ctx: *mut TaoCodecContext,
    frame: *const TaoFrame,
) -> c_int {
    if ctx.is_null() {
        return TAO_ERROR;
    }

    let ctx = unsafe { &mut *ctx };
    let TaoCodecContextInner::Encoder(encoder) = &mut ctx.inner else {
        return TAO_ERROR;
    };

    let frame_ref = if frame.is_null() {
        None
    } else {
        Some(unsafe { &(*frame).0 })
    };

    match encoder.send_frame(frame_ref) {
        Ok(()) => TAO_OK,
        Err(e) => tao_error_to_int(&e),
    }
}

/// 从编码器取出一个数据包
///
/// 成功时 *packet 指向新分配的 TaoPacket, 调用方必须使用 tao_packet_free 释放.
///
/// # Safety
///
/// ctx 和 packet 必须非空. packet 指向有效的指针变量.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_codec_receive_packet(
    ctx: *mut TaoCodecContext,
    packet: *mut *mut TaoPacket,
) -> c_int {
    if ctx.is_null() || packet.is_null() {
        return TAO_ERROR;
    }

    let ctx = unsafe { &mut *ctx };
    let TaoCodecContextInner::Encoder(encoder) = &mut ctx.inner else {
        return TAO_ERROR;
    };

    let pkt = match encoder.receive_packet() {
        Ok(p) => p,
        Err(e) => return tao_error_to_int(&e),
    };

    let tao_pkt = Box::new(TaoPacket(pkt));
    unsafe {
        *packet = Box::into_raw(tao_pkt);
    }
    TAO_OK
}

/// 关闭编解码器上下文
///
/// # Safety
///
/// ctx 必须为由 tao_codec_create_decoder/encoder 返回的有效指针, 调用后不可再使用.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_codec_close(ctx: *mut TaoCodecContext) {
    if !ctx.is_null() {
        let _ = unsafe { Box::from_raw(ctx) };
    }
}

// =============================================================================
// Packet / Frame 访问器与内存管理
// =============================================================================

/// 获取数据包数据指针
///
/// # Safety
///
/// 返回的指针在 TaoPacket 存活期间有效, 且不可写入.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_packet_data(pkt: *const TaoPacket) -> *const u8 {
    if pkt.is_null() {
        return ptr::null();
    }
    let pkt = unsafe { &*pkt };
    let data = pkt.0.data.as_ref();
    if data.is_empty() {
        ptr::null()
    } else {
        data.as_ptr()
    }
}

/// 获取数据包大小 (字节)
///
/// # Safety
///
/// pkt 必须为由 tao_format_read_packet 或 tao_codec_receive_packet 返回的有效指针.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_packet_size(pkt: *const TaoPacket) -> c_int {
    if pkt.is_null() {
        return -1;
    }
    unsafe { (*pkt).0.size() as c_int }
}

/// 获取数据包 PTS
///
/// # Safety
///
/// pkt 必须为有效的 TaoPacket 指针.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_packet_pts(pkt: *const TaoPacket) -> i64 {
    if pkt.is_null() {
        return -1;
    }
    unsafe { (*pkt).0.pts }
}

/// 获取数据包所属流索引
///
/// # Safety
///
/// pkt 必须为有效的 TaoPacket 指针.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_packet_stream_index(pkt: *const TaoPacket) -> c_int {
    if pkt.is_null() {
        return -1;
    }
    unsafe { (*pkt).0.stream_index as c_int }
}

/// 释放数据包
///
/// # Safety
///
/// pkt 必须为由 tao_format_read_packet 或 tao_codec_receive_packet 返回的有效指针, 调用后不可再使用.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_packet_free(pkt: *mut TaoPacket) {
    if !pkt.is_null() {
        let _ = unsafe { Box::from_raw(pkt) };
    }
}

/// 判断帧是否为音频
///
/// # Safety
///
/// frame 必须为有效的 TaoFrame 指针.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_frame_is_audio(frame: *const TaoFrame) -> c_int {
    if frame.is_null() {
        return 0;
    }
    match unsafe { &(*frame).0 } {
        Frame::Audio(_) => 1,
        Frame::Video(_) => 0,
    }
}

/// 判断帧是否为视频
///
/// # Safety
///
/// frame 必须为有效的 TaoFrame 指针.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_frame_is_video(frame: *const TaoFrame) -> c_int {
    if frame.is_null() {
        return 0;
    }
    match unsafe { &(*frame).0 } {
        Frame::Video(_) => 1,
        Frame::Audio(_) => 0,
    }
}

/// 获取音频帧采样数 (每声道). 视频帧返回 0.
///
/// # Safety
///
/// frame 必须为有效的 TaoFrame 指针.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_frame_nb_samples(frame: *const TaoFrame) -> c_int {
    if frame.is_null() {
        return -1;
    }
    match unsafe { &(*frame).0 } {
        Frame::Audio(a) => a.nb_samples as c_int,
        Frame::Video(_) => 0,
    }
}

/// 获取音频帧采样率. 视频帧返回 0.
///
/// # Safety
///
/// frame 必须为有效的 TaoFrame 指针.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_frame_sample_rate(frame: *const TaoFrame) -> c_int {
    if frame.is_null() {
        return -1;
    }
    match unsafe { &(*frame).0 } {
        Frame::Audio(a) => a.sample_rate as c_int,
        Frame::Video(_) => 0,
    }
}

/// 获取视频帧宽度. 音频帧返回 0.
///
/// # Safety
///
/// frame 必须为有效的 TaoFrame 指针.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_frame_width(frame: *const TaoFrame) -> c_int {
    if frame.is_null() {
        return -1;
    }
    match unsafe { &(*frame).0 } {
        Frame::Video(v) => v.width as c_int,
        Frame::Audio(_) => 0,
    }
}

/// 获取视频帧高度. 音频帧返回 0.
///
/// # Safety
///
/// frame 必须为有效的 TaoFrame 指针.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_frame_height(frame: *const TaoFrame) -> c_int {
    if frame.is_null() {
        return -1;
    }
    match unsafe { &(*frame).0 } {
        Frame::Video(v) => v.height as c_int,
        Frame::Audio(_) => 0,
    }
}

/// 获取帧指定平面的数据指针
///
/// plane 从 0 开始. 视频 YUV420P 有 3 平面, RGB 有 1 平面.
/// 音频交错格式仅 plane 0 有效.
///
/// # Safety
///
/// 返回的指针在 TaoFrame 存活期间有效.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_frame_data(frame: *const TaoFrame, plane: c_int) -> *const u8 {
    if frame.is_null() || plane < 0 {
        return ptr::null();
    }
    let frame = unsafe { &(*frame).0 };
    let plane_idx = plane as usize;
    let data = match frame {
        Frame::Video(v) => v.data.get(plane_idx),
        Frame::Audio(a) => a.data.get(plane_idx),
    };
    match data {
        Some(d) if !d.is_empty() => d.as_ptr(),
        _ => ptr::null(),
    }
}

/// 获取帧指定平面的行字节数 (linesize)
///
/// # Safety
///
/// frame 必须为有效的 TaoFrame 指针.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_frame_linesize(frame: *const TaoFrame, plane: c_int) -> c_int {
    if frame.is_null() || plane < 0 {
        return -1;
    }
    let frame = unsafe { &(*frame).0 };
    let plane_idx = plane as usize;
    let linesize: Option<usize> = match frame {
        Frame::Video(v) => v.linesize.get(plane_idx).copied(),
        Frame::Audio(a) => a.data.get(plane_idx).map(|d| d.len()),
    };
    match linesize {
        Some(ls) => ls as c_int,
        None => -1,
    }
}

/// 释放帧
///
/// # Safety
///
/// frame 必须为由 tao_codec_receive_frame 返回的有效指针, 调用后不可再使用.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_frame_free(frame: *mut TaoFrame) {
    if !frame.is_null() {
        let _ = unsafe { Box::from_raw(frame) };
    }
}

// =============================================================================
// Scale 操作
// =============================================================================

/// 创建缩放上下文
///
/// src_format 和 dst_format 为像素格式 ID (与 tao-core PixelFormat 对应, 此处简化用 u32).
/// 常用: 0=Yuv420p, 1=Rgb24 等. 具体映射见 tao-core pixel_format.
///
/// # Safety
///
/// 无特殊安全要求.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_scale_context_create(
    src_width: u32,
    src_height: u32,
    src_format: u32,
    dst_width: u32,
    dst_height: u32,
    dst_format: u32,
) -> *mut TaoScaleContext {
    let src_pf = pixel_format_from_u32(src_format);
    let dst_pf = pixel_format_from_u32(dst_format);
    let ctx = ScaleContext::new(
        src_width,
        src_height,
        src_pf,
        dst_width,
        dst_height,
        dst_pf,
        ScaleAlgorithm::Bilinear,
    );
    Box::into_raw(Box::new(TaoScaleContext(ctx)))
}

/// 执行图像缩放/格式转换 (单平面格式如 RGB24)
///
/// 适用于单平面格式. 多平面格式需使用其他接口.
///
/// # Safety
///
/// src_data 和 dst_data 必须指向有效缓冲区, 大小足够.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_scale_scale(
    ctx: *mut TaoScaleContext,
    src_data: *const u8,
    src_linesize: c_int,
    dst_data: *mut u8,
    dst_linesize: c_int,
) -> c_int {
    if ctx.is_null() || src_data.is_null() || dst_data.is_null() {
        return TAO_ERROR;
    }
    let ctx = unsafe { &*ctx };
    let src_slice = unsafe {
        std::slice::from_raw_parts(
            src_data,
            (ctx.0.src_height as usize) * (src_linesize as usize),
        )
    };
    let dst_slice = unsafe {
        std::slice::from_raw_parts_mut(
            dst_data,
            (ctx.0.dst_height as usize) * (dst_linesize as usize),
        )
    };
    match ctx.0.scale(
        &[src_slice],
        &[src_linesize as usize],
        &mut [dst_slice],
        &[dst_linesize as usize],
    ) {
        Ok(()) => TAO_OK,
        Err(_) => TAO_ERROR,
    }
}

/// 释放缩放上下文
///
/// # Safety
///
/// ctx 必须为由 tao_scale_context_create 返回的有效指针, 调用后不可再使用.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_scale_context_free(ctx: *mut TaoScaleContext) {
    if !ctx.is_null() {
        let _ = unsafe { Box::from_raw(ctx) };
    }
}

fn pixel_format_from_u32(id: u32) -> tao_core::PixelFormat {
    use tao_core::PixelFormat;
    match id {
        0 => PixelFormat::Yuv420p,
        1 => PixelFormat::Rgb24,
        2 => PixelFormat::Bgr24,
        3 => PixelFormat::Yuv422p,
        4 => PixelFormat::Yuv444p,
        _ => PixelFormat::Yuv420p,
    }
}

// =============================================================================
// Resample 操作
// =============================================================================

/// 创建重采样上下文
///
/// sample_format: 0=None, 1=U8, 2=S16, 3=S32, 4=F32, 5=F64
///
/// # Safety
///
/// 无特殊安全要求.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_resample_context_create(
    src_sample_rate: u32,
    src_sample_format: u32,
    src_channels: u32,
    dst_sample_rate: u32,
    dst_sample_format: u32,
    dst_channels: u32,
) -> *mut TaoResampleContext {
    let src_sf = sample_format_from_u32(src_sample_format);
    let dst_sf = sample_format_from_u32(dst_sample_format);
    let ctx = ResampleContext::new(
        src_sample_rate,
        src_sf,
        ChannelLayout::from_channels(src_channels),
        dst_sample_rate,
        dst_sf,
        ChannelLayout::from_channels(dst_channels),
    );
    Box::into_raw(Box::new(TaoResampleContext(ctx)))
}

/// 执行重采样
///
/// 将 input 中的 nb_samples 个采样 (每声道) 转换, 输出写入 output 缓冲区.
/// output 必须由调用方预分配, 大小应足够 (通常 dst_nb_samples * channels * bytes_per_sample).
/// 返回实际输出的每声道采样数, 失败返回 -1.
///
/// # Safety
///
/// input 和 output 必须指向有效缓冲区, 且大小足够.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_resample_convert(
    ctx: *mut TaoResampleContext,
    input: *const u8,
    input_size: c_int,
    nb_samples: u32,
    output: *mut u8,
    output_size: c_int,
    output_nb_samples: *mut u32,
) -> c_int {
    if ctx.is_null()
        || input.is_null()
        || output.is_null()
        || output_nb_samples.is_null()
        || input_size <= 0
        || output_size <= 0
    {
        return TAO_ERROR;
    }

    let ctx = unsafe { &*ctx };
    let input_slice = unsafe { std::slice::from_raw_parts(input, input_size as usize) };
    let output_slice = unsafe { std::slice::from_raw_parts_mut(output, output_size as usize) };

    let (data, nb_out) = match ctx.0.convert(input_slice, nb_samples) {
        Ok(r) => r,
        Err(_) => return TAO_ERROR,
    };

    if data.len() > output_slice.len() {
        return TAO_ERROR;
    }
    output_slice[..data.len()].copy_from_slice(&data);
    unsafe {
        *output_nb_samples = nb_out;
    }
    TAO_OK
}

/// 释放重采样上下文
///
/// # Safety
///
/// ctx 必须为由 tao_resample_context_create 返回的有效指针, 调用后不可再使用.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tao_resample_context_free(ctx: *mut TaoResampleContext) {
    if !ctx.is_null() {
        let _ = unsafe { Box::from_raw(ctx) };
    }
}

fn sample_format_from_u32(id: u32) -> SampleFormat {
    match id {
        0 => SampleFormat::None,
        1 => SampleFormat::U8,
        2 => SampleFormat::S16,
        3 => SampleFormat::S32,
        4 => SampleFormat::F32,
        5 => SampleFormat::F64,
        _ => SampleFormat::S16,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        let version_ptr = unsafe { tao_version() };
        assert!(!version_ptr.is_null());
        let version = unsafe { CStr::from_ptr(version_ptr) };
        assert!(version.to_str().unwrap().starts_with("0.1"));

        let version_int = unsafe { tao_version_int() };
        assert_eq!(version_int, (1 << 8));

        let build_info = unsafe { tao_build_info() };
        assert!(!build_info.is_null());
        let info = unsafe { CStr::from_ptr(build_info) };
        assert!(info.to_str().unwrap().contains("tao"));
    }

    #[test]
    fn test_codec_id_mapping() {
        // 往返映射
        for id in [
            CodecId::None,
            CodecId::H264,
            CodecId::Aac,
            CodecId::Mp3,
            CodecId::PcmS16le,
            CodecId::RawVideo,
        ] {
            let int_val = codec_id_to_int(id);
            let back = codec_id_from_int(int_val);
            assert_eq!(Some(id), back, "codec_id {:?} roundtrip failed", id);
        }

        // 无效 ID 返回 None
        assert!(codec_id_from_int(-1).is_none());
        assert!(codec_id_from_int(999).is_none());
    }
}
