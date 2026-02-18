use tao_core::{TaoError, TaoResult};

use super::bitreader::{LsbBitReader, ilog};

#[derive(Debug, Clone)]
pub(crate) struct ParsedSetup {
    pub(crate) mode_block_flags: Vec<bool>,
    pub(crate) floor_count: u32,
    pub(crate) residue_count: u32,
    pub(crate) mapping_count: u32,
}

pub(crate) fn parse_setup_packet(packet: &[u8], channels: u8) -> TaoResult<ParsedSetup> {
    if packet.len() < 8 {
        return Err(TaoError::InvalidData("Vorbis setup 头包长度不足".into()));
    }
    if packet[0] != 0x05 || &packet[1..7] != b"vorbis" {
        return Err(TaoError::InvalidData("Vorbis setup 头包标识无效".into()));
    }

    let mut br = LsbBitReader::new(&packet[7..]);
    let _codebook_count = parse_codebooks(&mut br).map_err(|e| {
        TaoError::InvalidData(format!(
            "Vorbis setup codebooks 解析失败(bit={}): {}",
            br.bit_position(),
            e
        ))
    })?;
    parse_time_domain_transforms(&mut br).map_err(|e| {
        TaoError::InvalidData(format!(
            "Vorbis setup time 解析失败(bit={}): {}",
            br.bit_position(),
            e
        ))
    })?;
    let floor_count = parse_floors(&mut br).map_err(|e| {
        TaoError::InvalidData(format!(
            "Vorbis setup floors 解析失败(bit={}): {}",
            br.bit_position(),
            e
        ))
    })?;
    let residue_count = parse_residues(&mut br).map_err(|e| {
        TaoError::InvalidData(format!(
            "Vorbis setup residues 解析失败(bit={}): {}",
            br.bit_position(),
            e
        ))
    })?;
    let mapping_count =
        parse_mappings(&mut br, channels, floor_count, residue_count).map_err(|e| {
            TaoError::InvalidData(format!(
                "Vorbis setup mappings 解析失败(bit={}): {}",
                br.bit_position(),
                e
            ))
        })?;
    let modes = parse_modes(&mut br, mapping_count).map_err(|e| {
        TaoError::InvalidData(format!(
            "Vorbis setup modes 解析失败(bit={}): {}",
            br.bit_position(),
            e
        ))
    })?;

    let framing_flag = br.read_flag()?;
    if !framing_flag {
        return Err(TaoError::InvalidData(
            "Vorbis setup 头包 framing_flag 非法".into(),
        ));
    }

    Ok(ParsedSetup {
        mode_block_flags: modes,
        floor_count,
        residue_count,
        mapping_count,
    })
}

fn parse_codebooks(br: &mut LsbBitReader<'_>) -> TaoResult<u32> {
    let codebook_count = br.read_bits(8)? + 1;
    for _ in 0..codebook_count {
        let sync = br.read_bits(24)?;
        if sync != 0x564342 {
            return Err(TaoError::InvalidData(format!(
                "Vorbis codebook 同步字错误: 0x{sync:06X}",
            )));
        }

        let dimensions = br.read_bits(16)?;
        if dimensions == 0 {
            return Err(TaoError::InvalidData(
                "Vorbis codebook dimensions 不能为 0".into(),
            ));
        }

        let entries = br.read_bits(24)?;
        if entries == 0 {
            return Err(TaoError::InvalidData(
                "Vorbis codebook entries 不能为 0".into(),
            ));
        }

        let ordered = br.read_flag()?;
        if ordered {
            let mut current_entry = 0u32;
            let mut _current_length = br.read_bits(5)? + 1;
            while current_entry < entries {
                let left = entries - current_entry;
                let bits = ilog(left);
                let number = br.read_bits(bits)?;
                if number == 0 || number > left {
                    return Err(TaoError::InvalidData(
                        "Vorbis codebook ordered 长度组无效".into(),
                    ));
                }
                current_entry += number;
                _current_length += 1;
            }
        } else {
            let sparse = br.read_flag()?;
            for _ in 0..entries {
                let used = if sparse { br.read_flag()? } else { true };
                if used {
                    let _length = br.read_bits(5)? + 1;
                }
            }
        }

        let lookup_type = br.read_bits(4)?;
        if lookup_type > 2 {
            return Err(TaoError::InvalidData(format!(
                "Vorbis codebook lookup_type 非法: {}",
                lookup_type,
            )));
        }
        if lookup_type == 1 || lookup_type == 2 {
            let _minimum = br.read_bits(32)?;
            let _maximum = br.read_bits(32)?;
            let value_bits = br.read_bits(4)? + 1;
            let _sequence_p = br.read_flag()?;

            let quant_values = if lookup_type == 1 {
                lookup1_values(entries, dimensions)
            } else {
                entries
                    .checked_mul(dimensions)
                    .ok_or_else(|| TaoError::InvalidData("Vorbis quant_values 溢出".into()))?
            };
            for _ in 0..quant_values {
                let _ = br.read_bits(value_bits as u8)?;
            }
        }
    }
    Ok(codebook_count)
}

fn parse_time_domain_transforms(br: &mut LsbBitReader<'_>) -> TaoResult<()> {
    let count = br.read_bits(6)? + 1;
    for _ in 0..count {
        let value = br.read_bits(16)?;
        if value != 0 {
            return Err(TaoError::InvalidData(
                "Vorbis time domain transform 必须为 0".into(),
            ));
        }
    }
    Ok(())
}

fn parse_floors(br: &mut LsbBitReader<'_>) -> TaoResult<u32> {
    let floor_count = br.read_bits(6)? + 1;
    for floor_idx in 0..floor_count {
        let floor_type_pos = br.bit_position();
        let floor_type = br.read_bits(16)?;
        match floor_type {
            0 => parse_floor0(br)?,
            1 => parse_floor1(br)?,
            _ => {
                let mut hints = Vec::new();
                for delta in -8i32..=8 {
                    let probe_pos = if delta < 0 {
                        floor_type_pos.saturating_sub((-delta) as usize)
                    } else {
                        floor_type_pos.saturating_add(delta as usize)
                    };
                    if let Ok(v) = br.read_bits_at(probe_pos, 16)
                        && (v == 0 || v == 1)
                    {
                        hints.push(format!("delta={delta},type={v}"));
                    }
                }
                return Err(TaoError::InvalidData(format!(
                    "Vorbis floor_type 不支持: {} (floor_idx={}, floor_count={}, bit={}, hints=[{}])",
                    floor_type,
                    floor_idx,
                    floor_count,
                    floor_type_pos,
                    hints.join(";")
                )));
            }
        }
    }
    Ok(floor_count)
}

fn parse_floor0(br: &mut LsbBitReader<'_>) -> TaoResult<()> {
    let _order = br.read_bits(8)?;
    let _rate = br.read_bits(16)?;
    let _bark_map_size = br.read_bits(16)?;
    let amp_bits = br.read_bits(6)?;
    if amp_bits == 0 {
        return Err(TaoError::InvalidData("Vorbis floor0 amp_bits 非法".into()));
    }
    let _amp_offset = br.read_bits(8)?;
    let book_count = br.read_bits(4)? + 1;
    for _ in 0..book_count {
        let _ = br.read_bits(8)?;
    }
    Ok(())
}

fn parse_floor1(br: &mut LsbBitReader<'_>) -> TaoResult<()> {
    let partitions = br.read_bits(5)?;
    let mut partition_classes = Vec::with_capacity(partitions as usize);
    let mut maximum_class = 0u32;
    for _ in 0..partitions {
        let class_num = br.read_bits(4)?;
        maximum_class = maximum_class.max(class_num);
        partition_classes.push(class_num);
    }

    let class_count = maximum_class + 1;
    let mut class_dimensions = vec![0u32; class_count as usize];
    for class_idx in 0..class_count {
        let dim = br.read_bits(3)? + 1;
        class_dimensions[class_idx as usize] = dim;

        let subclass = br.read_bits(2)?;
        if subclass > 0 {
            let _masterbook = br.read_bits(8)?;
        }
        let count = 1u32 << subclass;
        for _ in 0..count {
            let _ = br.read_bits(8)?;
        }
    }

    let multiplier = br.read_bits(2)? + 1;
    if multiplier > 4 {
        return Err(TaoError::InvalidData(
            "Vorbis floor1 multiplier 非法".into(),
        ));
    }

    let range_bits = br.read_bits(4)?;
    // floor1 的前两个点 X=0 和 X=(1<<range_bits) 为隐式常量, 不占用位流.
    for class_num in partition_classes {
        let class_idx = class_num as usize;
        let dim = class_dimensions[class_idx];
        for _ in 0..dim {
            let _ = br.read_bits(range_bits as u8)?;
        }
    }
    Ok(())
}

fn parse_residues(br: &mut LsbBitReader<'_>) -> TaoResult<u32> {
    let residue_count = br.read_bits(6)? + 1;
    for _ in 0..residue_count {
        let residue_type = br.read_bits(16)?;
        if residue_type > 2 {
            return Err(TaoError::InvalidData(format!(
                "Vorbis residue_type 不支持: {}",
                residue_type,
            )));
        }
        let _begin = br.read_bits(24)?;
        let _end = br.read_bits(24)?;
        let _partition_size = br.read_bits(24)? + 1;
        let classifications = br.read_bits(6)? + 1;
        let _classbook = br.read_bits(8)?;

        let mut cascades = vec![0u32; classifications as usize];
        for cascade in &mut cascades {
            let low_bits = br.read_bits(3)?;
            let bitflag = br.read_flag()?;
            let high_bits = if bitflag { br.read_bits(5)? } else { 0 };
            *cascade = (high_bits << 3) | low_bits;
        }

        for cascade in cascades {
            for bit in 0..8 {
                if (cascade & (1 << bit)) != 0 {
                    let _ = br.read_bits(8)?;
                }
            }
        }
    }
    Ok(residue_count)
}

fn parse_mappings(
    br: &mut LsbBitReader<'_>,
    channels: u8,
    floor_count: u32,
    residue_count: u32,
) -> TaoResult<u32> {
    let mapping_count = br.read_bits(6)? + 1;
    for _ in 0..mapping_count {
        let mapping_type = br.read_bits(16)?;
        if mapping_type != 0 {
            return Err(TaoError::InvalidData(format!(
                "Vorbis mapping_type 不支持: {}",
                mapping_type,
            )));
        }

        let submaps = if br.read_flag()? {
            br.read_bits(4)? + 1
        } else {
            1
        };

        if br.read_flag()? {
            let coupling_steps = br.read_bits(8)? + 1;
            let ch_bits = ilog(u32::from(channels) - 1);
            for _ in 0..coupling_steps {
                let magnitude = br.read_bits(ch_bits)?;
                let angle = br.read_bits(ch_bits)?;
                if magnitude == angle
                    || magnitude >= u32::from(channels)
                    || angle >= u32::from(channels)
                {
                    return Err(TaoError::InvalidData("Vorbis coupling 参数非法".into()));
                }
            }
        }

        let reserved = br.read_bits(2)?;
        if reserved != 0 {
            return Err(TaoError::InvalidData(
                "Vorbis mapping reserved bits 必须为 0".into(),
            ));
        }

        if submaps > 1 {
            for _ in 0..channels {
                let mux = br.read_bits(4)?;
                if mux >= submaps {
                    return Err(TaoError::InvalidData("Vorbis mapping mux 值越界".into()));
                }
            }
        }

        for _ in 0..submaps {
            let _time_submap = br.read_bits(8)?;
            let floor = br.read_bits(8)?;
            let residue = br.read_bits(8)?;
            if floor >= floor_count || residue >= residue_count {
                return Err(TaoError::InvalidData(
                    "Vorbis mapping floor/residue 索引越界".into(),
                ));
            }
        }
    }
    Ok(mapping_count)
}

fn parse_modes(br: &mut LsbBitReader<'_>, mapping_count: u32) -> TaoResult<Vec<bool>> {
    let mode_count = br.read_bits(6)? + 1;
    let mut mode_flags = Vec::with_capacity(mode_count as usize);
    for _ in 0..mode_count {
        let block_flag = br.read_flag()?;
        let window_type = br.read_bits(16)?;
        let transform_type = br.read_bits(16)?;
        if window_type != 0 || transform_type != 0 {
            return Err(TaoError::InvalidData(
                "Vorbis mode window/transform 必须为 0".into(),
            ));
        }

        let mapping = br.read_bits(8)?;
        if mapping >= mapping_count {
            return Err(TaoError::InvalidData("Vorbis mode mapping 索引越界".into()));
        }

        mode_flags.push(block_flag);
    }
    Ok(mode_flags)
}

fn lookup1_values(entries: u32, dimensions: u32) -> u32 {
    if entries == 0 || dimensions == 0 {
        return 0;
    }

    let mut lo = 1u32;
    let mut hi = entries.max(1);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if pow_le_entries(mid, dimensions, entries) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

fn pow_le_entries(base: u32, exp: u32, entries: u32) -> bool {
    let mut out = 1u128;
    let limit = entries as u128;
    for _ in 0..exp {
        out *= base as u128;
        if out > limit {
            return false;
        }
    }
    true
}
