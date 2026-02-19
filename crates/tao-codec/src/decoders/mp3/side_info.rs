//! MP3 侧边信息 (Side Information) 解析

use super::bitreader::BitReader;
use super::header::{ChannelMode, Mp3Header, MpegVersion};
use tao_core::TaoResult;

/// Granule (颗粒) 信息
#[derive(Debug, Clone, Default)]
pub struct Granule {
    pub part2_3_length: u32,
    pub big_values: u32,
    pub global_gain: u32,
    pub scalefac_compress: u32,
    pub windows_switching_flag: bool,

    pub block_type: u8,
    pub mixed_block_flag: bool,
    pub table_select: [u8; 3],
    pub subblock_gain: [u8; 3],

    pub region0_count: u32,
    pub region1_count: u32,

    pub preflag: bool,
    pub scalefac_scale: bool,
    pub count1table_select: bool,
}

/// Side Information
#[derive(Debug, Clone)]
pub struct SideInfo {
    pub main_data_begin: u32,
    pub private_bits: u32,
    pub scfsi: [[u8; 4]; 2],         // [channel][band] (MPEG1 only)
    pub granules: [[Granule; 2]; 2], // [granule][channel] (MPEG1: 2x2, MPEG2: 1x2)
}

impl SideInfo {
    pub fn parse(reader: &mut BitReader, header: &Mp3Header) -> TaoResult<Self> {
        let nch = if header.mode == ChannelMode::SingleChannel {
            1
        } else {
            2
        };
        let is_mpeg1 = header.version == MpegVersion::Mpeg1;
        let ngr = if is_mpeg1 { 2 } else { 1 };

        let mut side_info = SideInfo {
            main_data_begin: 0,
            private_bits: 0,
            scfsi: [[0; 4]; 2],
            granules: Default::default(),
        };

        // 1. main_data_begin
        side_info.main_data_begin = reader.read_bits(if is_mpeg1 { 9 } else { 8 }).unwrap();

        // 2. private_bits
        let private_bits_len = if is_mpeg1 {
            if nch == 1 { 5 } else { 3 }
        } else if nch == 1 {
            1
        } else {
            2
        };
        side_info.private_bits = reader.read_bits(private_bits_len).unwrap();

        // 3. scfsi (MPEG1 only)
        if is_mpeg1 {
            for ch in 0..nch {
                for band in 0..4 {
                    side_info.scfsi[ch][band] = reader.read_bits(1).unwrap() as u8;
                }
            }
        }

        // 4. Granule data
        for gr in 0..ngr {
            for ch in 0..nch {
                let dst = &mut side_info.granules[gr][ch];

                dst.part2_3_length = reader.read_bits(12).unwrap();
                dst.big_values = reader.read_bits(9).unwrap();
                dst.global_gain = reader.read_bits(8).unwrap();
                dst.scalefac_compress = reader.read_bits(if is_mpeg1 { 4 } else { 9 }).unwrap();
                dst.windows_switching_flag = reader.read_bool().unwrap();

                if dst.windows_switching_flag {
                    dst.block_type = reader.read_bits(2).unwrap() as u8;
                    dst.mixed_block_flag = reader.read_bool().unwrap();

                    for i in 0..2 {
                        dst.table_select[i] = reader.read_bits(5).unwrap() as u8;
                    }

                    for i in 0..3 {
                        dst.subblock_gain[i] = reader.read_bits(3).unwrap() as u8;
                    }

                    // windows_switching 时 region counts 不从比特流传输, 使用隐式值
                    // 参考 ISO 11172-3 / minimp3:
                    //   block_type=2 纯短块: region0_count=8, region1_count=36
                    //   block_type=2 混合块 / block_type=1,3: region0_count=7, region1_count=36
                    // region1_count 设为足够大的值确保 region2 为空
                    // (因为 windows_switching 时只传输 2 个 table_select)
                    if dst.block_type == 2 && !dst.mixed_block_flag {
                        dst.region0_count = 8;
                        dst.region1_count = 36;
                    } else {
                        dst.region0_count = 7;
                        dst.region1_count = 36;
                    }
                } else {
                    for i in 0..3 {
                        dst.table_select[i] = reader.read_bits(5).unwrap() as u8;
                    }
                    dst.region0_count = reader.read_bits(4).unwrap();
                    dst.region1_count = reader.read_bits(3).unwrap();
                    dst.block_type = 0;
                }

                if is_mpeg1 {
                    dst.preflag = reader.read_bool().unwrap();
                } else {
                    // MPEG-2/2.5 LSF: preflag 不从比特流读取, 由 scalefac_compress 推导.
                    dst.preflag = dst.scalefac_compress >= 500;
                }
                dst.scalefac_scale = reader.read_bool().unwrap();
                dst.count1table_select = reader.read_bool().unwrap();
            }
        }

        Ok(side_info)
    }
}
