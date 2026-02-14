//! MP3 (MPEG-1/2 Audio Layer III) 解码器.
//!
//! 使用 puremp3 纯 Rust 库进行实际的帧解码, 包含完整的:
//! - Huffman 解码
//! - 反量化 (requantization)
//! - 立体声处理 (MS Stereo / Intensity Stereo)
//! - IMDCT 逆变换
//! - 多相合成滤波器 (Synthesis Filterbank) -> PCM
//!
//! 解码流程 (封装 puremp3):
//! 1. 从 demuxer 接收完整的 MP3 帧 (含帧头)
//! 2. 将帧数据推入内部缓冲区
//! 3. 调用 puremp3 解码器解码为 F32 PCM 采样
//! 4. 输出交错格式的音频帧

use std::io;
use tao_core::{ChannelLayout, Rational, SampleFormat, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::CodecParameters;
use crate::decoder::Decoder;
use crate::frame::{AudioFrame, Frame};
use crate::packet::Packet;

/// 流式读取缓冲区, 用于向 puremp3 提供连续的 MP3 数据流.
///
/// 支持追加数据和按需读取, 解码完成后可压缩已消耗的部分.
struct StreamBuffer {
    /// 原始数据
    data: Vec<u8>,
    /// 当前读取位置
    pos: usize,
}

impl StreamBuffer {
    /// 创建空缓冲区
    fn new() -> Self {
        Self {
            data: Vec::new(),
            pos: 0,
        }
    }

    /// 向缓冲区追加数据
    fn push(&mut self, bytes: &[u8]) {
        self.data.extend_from_slice(bytes);
    }

    /// 压缩已消耗的数据, 释放内存
    fn compact(&mut self) {
        if self.pos > 0 {
            self.data.drain(..self.pos);
            self.pos = 0;
        }
    }

    /// 当前读取位置
    fn position(&self) -> usize {
        self.pos
    }

    /// 恢复读取位置 (解码失败时回退)
    fn set_position(&mut self, pos: usize) {
        self.pos = pos;
    }
}

impl io::Read for StreamBuffer {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let available = self.data.len() - self.pos;
        if available == 0 {
            // 数据不足, 返回 EOF 让 puremp3 报错
            return Ok(0);
        }
        let n = buf.len().min(available);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// MP3 解码器
///
/// 封装 puremp3 纯 Rust 解码库, 通过 Decoder trait 提供统一接口.
/// 内部维护一个流式缓冲区, 将 demuxer 提供的离散帧数据转化为
/// puremp3 所需的连续字节流.
pub struct Mp3Decoder {
    /// puremp3 解码器实例
    inner: puremp3::Mp3Decoder<StreamBuffer>,
    /// 解码器是否已打开
    opened: bool,
    /// 是否正在 flush
    flushing: bool,
    /// 待输出的解码帧
    output_frame: Option<Frame>,
    /// PTS 追踪 (累积采样数)
    next_pts: i64,
    /// 采样率
    sample_rate: u32,
    /// 声道数
    channels: u32,
    /// 声道布局
    channel_layout: ChannelLayout,
}

impl Mp3Decoder {
    /// 创建 MP3 解码器 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        let buffer = StreamBuffer::new();
        Ok(Box::new(Self {
            inner: puremp3::Mp3Decoder::new(buffer),
            opened: false,
            flushing: false,
            output_frame: None,
            next_pts: 0,
            sample_rate: 44100,
            channels: 2,
            channel_layout: ChannelLayout::from_channels(2),
        }))
    }

    /// 将 puremp3 解码结果转换为 AudioFrame
    fn convert_frame(
        frame: &puremp3::Frame,
        pts: i64,
        sample_rate: u32,
        channels: u32,
        channel_layout: ChannelLayout,
    ) -> AudioFrame {
        let spf = frame.num_samples as u32;
        let ch = channels as usize;

        // 构建交错 F32 输出
        let total_bytes = frame.num_samples * ch * 4;
        let mut interleaved = Vec::with_capacity(total_bytes);
        for i in 0..frame.num_samples {
            for c in 0..ch {
                let sample = frame.samples[c][i].clamp(-1.0, 1.0);
                interleaved.extend_from_slice(&sample.to_le_bytes());
            }
        }

        let mut af = AudioFrame::new(spf, sample_rate, SampleFormat::F32, channel_layout);
        af.data = vec![interleaved];
        af.pts = pts;
        af.time_base = Rational::new(1, sample_rate as i32);
        af.duration = spf as i64;

        af
    }
}

impl Decoder for Mp3Decoder {
    fn codec_id(&self) -> CodecId {
        CodecId::Mp3
    }

    fn name(&self) -> &str {
        "mp3"
    }

    fn open(&mut self, _params: &CodecParameters) -> TaoResult<()> {
        self.opened = true;
        self.flushing = false;
        self.output_frame = None;
        self.next_pts = 0;
        // 重新创建解码器以清空状态
        self.inner = puremp3::Mp3Decoder::new(StreamBuffer::new());
        Ok(())
    }

    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::Codec("MP3 解码器未打开".into()));
        }

        let data = &packet.data;
        if data.len() < 4 {
            return Err(TaoError::InvalidData("MP3 数据包太短".into()));
        }

        // 将帧数据推入缓冲区
        let pos_before = self.inner.get_ref().position();
        self.inner.get_mut().push(data);

        // 尝试解码
        match self.inner.next_frame() {
            Ok(frame) => {
                // 更新采样参数
                self.sample_rate = frame.header.sample_rate.hz();
                self.channels = frame.header.channels.num_channels() as u32;
                self.channel_layout = ChannelLayout::from_channels(self.channels);

                let spf = frame.num_samples as i64;
                let af = Self::convert_frame(
                    &frame,
                    self.next_pts,
                    self.sample_rate,
                    self.channels,
                    self.channel_layout,
                );
                self.next_pts += spf;
                self.output_frame = Some(Frame::Audio(af));

                // 压缩已消耗的缓冲区
                self.inner.get_mut().compact();
            }
            Err(_) => {
                // 解码失败 (数据不足或无效帧), 恢复缓冲区位置
                self.inner.get_mut().set_position(pos_before);
            }
        }

        Ok(())
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        self.output_frame.take().ok_or(TaoError::NeedMoreData)
    }

    fn flush(&mut self) {
        self.flushing = true;
        self.output_frame = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_parameters::{CodecParameters, CodecParamsType};

    /// 构造一个最小合法 MP3 帧 (MPEG-1, Layer III, 128kbps, 44100Hz, stereo)
    fn make_mp3_frame() -> Vec<u8> {
        // 同步字 + MPEG-1 + Layer III + 无CRC
        // 0xFF 0xFB = 1111_1111 1111_1011
        // 比特率 128kbps = index 9 (1001)
        // 采样率 44100 = index 0 (00)
        // padding=0, private=0
        // 声道: stereo=00, mode_ext=00, copyright=0, original=1, emphasis=00
        let header: [u8; 4] = [0xFF, 0xFB, 0x90, 0x04];
        let frame_size = 144 * 128000 / 44100; // 417 bytes
        let mut frame = vec![0u8; frame_size as usize];
        frame[..4].copy_from_slice(&header);
        // 填充有效载荷 (模拟真实数据)
        for (i, byte) in frame.iter_mut().enumerate().skip(4) {
            *byte = ((i * 7 + 3) % 256) as u8;
        }
        frame
    }

    fn default_params() -> CodecParameters {
        CodecParameters {
            codec_id: CodecId::Mp3,
            bit_rate: 0,
            params: CodecParamsType::None,
            extra_data: Vec::new(),
        }
    }

    #[test]
    fn test_发送合成帧不崩溃() {
        // 注意: 合成帧的载荷不是有效的 Huffman 编码数据,
        // puremp3 无法解码, 但 send_packet 不应崩溃或返回错误.
        let mut decoder = Mp3Decoder::create().unwrap();
        decoder.open(&default_params()).unwrap();
        let frame_data = make_mp3_frame();
        let mut packet = Packet::from_data(frame_data);
        packet.pts = 0;
        packet.duration = 1152;
        // send_packet 应该不会 panic, 即使内部解码失败也会优雅处理
        assert!(decoder.send_packet(&packet).is_ok());
        // 合成帧解码失败, receive_frame 返回 NeedMoreData
        assert!(decoder.receive_frame().is_err());
    }

    #[test]
    fn test_多帧连续发送() {
        let mut decoder = Mp3Decoder::create().unwrap();
        decoder.open(&default_params()).unwrap();
        // 连续发送多个帧, 不应崩溃
        for _ in 0..5 {
            let frame_data = make_mp3_frame();
            let packet = Packet::from_data(frame_data);
            assert!(decoder.send_packet(&packet).is_ok());
        }
    }

    #[test]
    fn test_未打开报错() {
        let mut decoder = Mp3Decoder::create().unwrap();
        let packet = Packet::from_data(make_mp3_frame());
        assert!(decoder.send_packet(&packet).is_err());
    }

    #[test]
    fn test_flush_和_eof() {
        let mut decoder = Mp3Decoder::create().unwrap();
        decoder.open(&default_params()).unwrap();
        decoder.flush();
        assert!(decoder.receive_frame().is_err());
    }
}
