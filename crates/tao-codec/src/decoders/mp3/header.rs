//! MP3 帧头解析

#![allow(dead_code)]

use tao_core::{TaoError, TaoResult};

/// MPEG 版本
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MpegVersion {
    Mpeg1 = 3,
    Mpeg2 = 2,
    Mpeg25 = 0,
    Reserved = 1,
}

/// MPEG Layer
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MpegLayer {
    Layer1 = 3,
    Layer2 = 2,
    Layer3 = 1,
    Reserved = 0,
}

/// 声道模式
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChannelMode {
    Stereo = 0,
    JointStereo = 1,
    DualChannel = 2,
    SingleChannel = 3,
}

/// MP3 帧头
#[derive(Debug, Clone)]
pub struct Mp3Header {
    pub version: MpegVersion,
    pub layer: MpegLayer,
    pub has_crc: bool,
    pub bitrate: u32,
    pub samplerate: u32,
    pub padding: bool,
    pub private: bool,
    pub mode: ChannelMode,
    pub mode_extension: u8,
    pub copyright: bool,
    pub original: bool,
    pub emphasis: u8,

    /// 帧大小 (字节)
    pub frame_size: usize,
    /// 侧边信息大小 (字节)
    pub side_info_size: usize,
}

impl Mp3Header {
    /// 解析 4 字节的帧头
    pub fn parse(header: u32) -> TaoResult<Self> {
        // AAAAAAAA AAABBCCD EEEEFFGH IIJJKLMM
        // A: Sync (11 bits) -> FFE (1111 1111 111)
        // B: Version (2 bits)
        // C: Layer (2 bits)
        // D: CRC (1 bit)
        // E: Bitrate (4 bits)
        // F: Samplerate (2 bits)
        // G: Padding (1 bit)
        // H: Private (1 bit)
        // I: Channel Mode (2 bits)
        // J: Mode Extension (2 bits)
        // K: Copyright (1 bit)
        // L: Original (1 bit)
        // M: Emphasis (2 bits)

        if (header & 0xFFE00000) != 0xFFE00000 {
            return Err(TaoError::InvalidData("Invalid sync word".into()));
        }

        let ver_idx = (header >> 19) & 0x3;
        let version = match ver_idx {
            3 => MpegVersion::Mpeg1,
            2 => MpegVersion::Mpeg2,
            0 => MpegVersion::Mpeg25,
            _ => return Err(TaoError::InvalidData("Reserved MPEG version".into())),
        };

        let layer_idx = (header >> 17) & 0x3;
        let layer = match layer_idx {
            3 => MpegLayer::Layer1, // Layer I
            2 => MpegLayer::Layer2, // Layer II
            1 => MpegLayer::Layer3, // Layer III
            _ => return Err(TaoError::InvalidData("Reserved MPEG layer".into())),
        };

        // 仅支持 Layer III
        if layer != MpegLayer::Layer3 {
            return Err(TaoError::Codec("Only Layer III is supported".into()));
        }

        let has_crc = ((header >> 16) & 0x1) == 0; // 0 means has CRC

        let bitrate_idx = ((header >> 12) & 0xF) as usize;
        if bitrate_idx == 0 || bitrate_idx == 15 {
            return Err(TaoError::InvalidData("Invalid bitrate index".into()));
        }

        let samplerate_idx = ((header >> 10) & 0x3) as usize;
        if samplerate_idx == 3 {
            return Err(TaoError::InvalidData("Invalid samplerate index".into()));
        }

        let padding = ((header >> 9) & 0x1) == 1;
        let private = ((header >> 8) & 0x1) == 1;

        let mode_idx = (header >> 6) & 0x3;
        let mode = match mode_idx {
            0 => ChannelMode::Stereo,
            1 => ChannelMode::JointStereo,
            2 => ChannelMode::DualChannel,
            3 => ChannelMode::SingleChannel,
            _ => unreachable!(),
        };

        let mode_extension = ((header >> 4) & 0x3) as u8;
        let copyright = ((header >> 3) & 0x1) == 1;
        let original = ((header >> 2) & 0x1) == 1;
        let emphasis = (header & 0x3) as u8;

        // Lookup tables
        let bitrate_kbps = Self::lookup_bitrate(version, layer, bitrate_idx);
        let bitrate_bps = bitrate_kbps * 1000;
        let samplerate = Self::lookup_samplerate(version, samplerate_idx);

        // 计算 Frame Size
        // Layer III: 144 * bitrate / samplerate + padding (MPEG1)
        //            72 * bitrate / samplerate + padding (MPEG2/2.5)
        // Note: bitrate in formula is bps
        let frame_size = if version == MpegVersion::Mpeg1 {
            (144 * bitrate_bps / samplerate + if padding { 1 } else { 0 }) as usize
        } else {
            (72 * bitrate_bps / samplerate + if padding { 1 } else { 0 }) as usize
        };

        // 计算 Side Info Size
        let side_info_size = match version {
            MpegVersion::Mpeg1 => {
                if mode == ChannelMode::SingleChannel {
                    17
                } else {
                    32
                }
            }
            _ => {
                if mode == ChannelMode::SingleChannel {
                    9
                } else {
                    17
                }
            }
        };

        Ok(Self {
            version,
            layer,
            has_crc,
            bitrate: bitrate_bps, // convert to bps
            samplerate,
            padding,
            private,
            mode,
            mode_extension,
            copyright,
            original,
            emphasis,
            frame_size,
            side_info_size,
        })
    }

    fn lookup_bitrate(version: MpegVersion, layer: MpegLayer, index: usize) -> u32 {
        // kbps tables
        // MPEG1, Layer3
        const V1_L3: [u32; 16] = [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
        ];
        // MPEG2/2.5, Layer3
        const V2_L3: [u32; 16] = [
            0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
        ];

        match (version, layer) {
            (MpegVersion::Mpeg1, MpegLayer::Layer3) => V1_L3[index],
            (MpegVersion::Mpeg2 | MpegVersion::Mpeg25, MpegLayer::Layer3) => V2_L3[index],
            _ => 0, // Should be caught by layer check earlier
        }
    }

    fn lookup_samplerate(version: MpegVersion, index: usize) -> u32 {
        match version {
            MpegVersion::Mpeg1 => [44100, 48000, 32000, 0][index],
            MpegVersion::Mpeg2 => [22050, 24000, 16000, 0][index],
            MpegVersion::Mpeg25 => [11025, 12000, 8000, 0][index],
            _ => 0,
        }
    }
}
