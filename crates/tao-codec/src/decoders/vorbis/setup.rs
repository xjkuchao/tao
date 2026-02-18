use tao_core::{TaoError, TaoResult};

use super::bitreader::{LsbBitReader, ilog};

#[derive(Debug, Clone)]
pub(crate) struct ParsedSetup {
    pub(crate) mode_block_flags: Vec<bool>,
    pub(crate) mode_mappings: Vec<u8>,
    pub(crate) mappings: Vec<MappingConfig>,
    pub(crate) codebooks: Vec<CodebookConfig>,
    pub(crate) floors: Vec<FloorConfig>,
    pub(crate) residues: Vec<ResidueConfig>,
    pub(crate) floor_count: u32,
    pub(crate) residue_count: u32,
    pub(crate) mapping_count: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct CodebookConfig {
    pub(crate) dimensions: u16,
    pub(crate) entries: u32,
    pub(crate) lengths: Vec<u8>,
    pub(crate) lookup_type: u8,
    pub(crate) lookup: Option<CodebookLookupConfig>,
}

#[derive(Debug, Clone)]
pub(crate) struct CodebookLookupConfig {
    pub(crate) minimum_value: f32,
    pub(crate) delta_value: f32,
    pub(crate) value_bits: u8,
    pub(crate) sequence_p: bool,
    pub(crate) lookup_values: u32,
    pub(crate) multiplicands: Vec<u32>,
}

#[derive(Debug, Clone)]
pub(crate) enum FloorConfig {
    Floor0,
    Floor1(Floor1Config),
}

#[derive(Debug, Clone)]
pub(crate) struct Floor1Class {
    pub(crate) dimensions: u8,
    pub(crate) subclasses: u8,
    pub(crate) masterbook: Option<u8>,
    pub(crate) subclass_books: Vec<Option<u8>>,
}

#[derive(Debug, Clone)]
pub(crate) struct Floor1Config {
    pub(crate) partitions: u8,
    pub(crate) partition_classes: Vec<u8>,
    pub(crate) classes: Vec<Floor1Class>,
    pub(crate) multiplier: u8,
    pub(crate) range_bits: u8,
    pub(crate) x_list: Vec<u16>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResidueConfig {
    pub(crate) residue_type: u8,
    pub(crate) begin: u32,
    pub(crate) end: u32,
    pub(crate) partition_size: u32,
    pub(crate) classifications: u8,
    pub(crate) classbook: u8,
    pub(crate) cascades: Vec<u8>,
    pub(crate) books: Vec<[Option<u8>; 8]>,
}

#[derive(Debug, Clone)]
pub(crate) struct MappingConfig {
    pub(crate) coupling_steps: Vec<CouplingStep>,
    pub(crate) channel_mux: Vec<u8>,
    pub(crate) submap_floor: Vec<u8>,
    pub(crate) submap_residue: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct CouplingStep {
    pub(crate) magnitude: u8,
    pub(crate) angle: u8,
}

pub(crate) fn parse_setup_packet(packet: &[u8], channels: u8) -> TaoResult<ParsedSetup> {
    if packet.len() < 8 {
        return Err(TaoError::InvalidData("Vorbis setup 头包长度不足".into()));
    }
    if packet[0] != 0x05 || &packet[1..7] != b"vorbis" {
        return Err(TaoError::InvalidData("Vorbis setup 头包标识无效".into()));
    }

    let mut br = LsbBitReader::new(&packet[7..]);
    let codebooks = parse_codebooks(&mut br).map_err(|e| {
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
    let floors = parse_floors(&mut br).map_err(|e| {
        TaoError::InvalidData(format!(
            "Vorbis setup floors 解析失败(bit={}): {}",
            br.bit_position(),
            e
        ))
    })?;
    let residues = parse_residues(&mut br).map_err(|e| {
        TaoError::InvalidData(format!(
            "Vorbis setup residues 解析失败(bit={}): {}",
            br.bit_position(),
            e
        ))
    })?;

    let floor_count = floors.len() as u32;
    let residue_count = residues.len() as u32;
    let mappings = parse_mappings(&mut br, channels, floor_count, residue_count).map_err(|e| {
        TaoError::InvalidData(format!(
            "Vorbis setup mappings 解析失败(bit={}): {}",
            br.bit_position(),
            e
        ))
    })?;
    let (mode_block_flags, mode_mappings) =
        parse_modes(&mut br, mappings.len() as u32).map_err(|e| {
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
        mode_block_flags,
        mode_mappings,
        mapping_count: mappings.len() as u32,
        mappings,
        codebooks,
        floors,
        residues,
        floor_count,
        residue_count,
    })
}

fn parse_codebooks(br: &mut LsbBitReader<'_>) -> TaoResult<Vec<CodebookConfig>> {
    let codebook_count = br.read_bits(8)? + 1;
    let mut codebooks = Vec::with_capacity(codebook_count as usize);
    for _ in 0..codebook_count {
        let sync = br.read_bits(24)?;
        if sync != 0x564342 {
            return Err(TaoError::InvalidData(format!(
                "Vorbis codebook 同步字错误: 0x{sync:06X}",
            )));
        }

        let dimensions = br.read_bits(16)? as u16;
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
        let mut lengths = vec![0u8; entries as usize];
        if ordered {
            let mut current_entry = 0u32;
            let mut current_length = br.read_bits(5)? + 1;
            while current_entry < entries {
                let left = entries - current_entry;
                let bits = ilog(left);
                let number = br.read_bits(bits)?;
                if number == 0 || number > left {
                    return Err(TaoError::InvalidData(
                        "Vorbis codebook ordered 长度组无效".into(),
                    ));
                }
                for _ in 0..number {
                    lengths[current_entry as usize] = current_length as u8;
                    current_entry += 1;
                }
                current_length += 1;
            }
        } else {
            let sparse = br.read_flag()?;
            for i in 0..entries {
                let used = if sparse { br.read_flag()? } else { true };
                if used {
                    let length = br.read_bits(5)? + 1;
                    lengths[i as usize] = length as u8;
                }
            }
        }

        let lookup_type = br.read_bits(4)? as u8;
        if lookup_type > 2 {
            return Err(TaoError::InvalidData(format!(
                "Vorbis codebook lookup_type 非法: {}",
                lookup_type,
            )));
        }
        let lookup = if lookup_type == 1 || lookup_type == 2 {
            let minimum_raw = br.read_bits(32)?;
            let maximum_raw = br.read_bits(32)?;
            let value_bits = (br.read_bits(4)? + 1) as u8;
            let sequence_p = br.read_flag()?;

            let lookup_values = if lookup_type == 1 {
                lookup1_values(entries, dimensions as u32)
            } else {
                entries
                    .checked_mul(dimensions as u32)
                    .ok_or_else(|| TaoError::InvalidData("Vorbis quant_values 溢出".into()))?
            };
            let mut multiplicands = Vec::with_capacity(lookup_values as usize);
            for _ in 0..lookup_values {
                multiplicands.push(br.read_bits(value_bits)?);
            }
            Some(CodebookLookupConfig {
                minimum_value: vorbis_float32_unpack(minimum_raw),
                delta_value: vorbis_float32_unpack(maximum_raw),
                value_bits,
                sequence_p,
                lookup_values,
                multiplicands,
            })
        } else {
            None
        };

        codebooks.push(CodebookConfig {
            dimensions,
            entries,
            lengths,
            lookup_type,
            lookup,
        });
    }
    Ok(codebooks)
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

fn parse_floors(br: &mut LsbBitReader<'_>) -> TaoResult<Vec<FloorConfig>> {
    let floor_count = br.read_bits(6)? + 1;
    let mut floors = Vec::with_capacity(floor_count as usize);
    for floor_idx in 0..floor_count {
        let floor_type_pos = br.bit_position();
        let floor_type = br.read_bits(16)?;
        match floor_type {
            0 => {
                parse_floor0(br)?;
                floors.push(FloorConfig::Floor0);
            }
            1 => floors.push(FloorConfig::Floor1(parse_floor1(br)?)),
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
    Ok(floors)
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

fn parse_floor1(br: &mut LsbBitReader<'_>) -> TaoResult<Floor1Config> {
    let partitions = br.read_bits(5)?;
    let mut partition_classes = Vec::with_capacity(partitions as usize);
    let mut maximum_class = 0u32;
    for _ in 0..partitions {
        let class_num = br.read_bits(4)?;
        maximum_class = maximum_class.max(class_num);
        partition_classes.push(class_num);
    }

    let class_count = maximum_class + 1;
    let mut classes = Vec::with_capacity(class_count as usize);
    for _ in 0..class_count {
        let dim = br.read_bits(3)? + 1;

        let subclass = br.read_bits(2)?;
        let masterbook = if subclass > 0 {
            Some(br.read_bits(8)? as u8)
        } else {
            None
        };
        let count = 1u32 << subclass;
        let mut subclass_books = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let v = br.read_bits(8)? as i32 - 1;
            if v >= 0 {
                subclass_books.push(Some(v as u8));
            } else {
                subclass_books.push(None);
            }
        }
        classes.push(Floor1Class {
            dimensions: dim as u8,
            subclasses: subclass as u8,
            masterbook,
            subclass_books,
        });
    }

    let multiplier = br.read_bits(2)? + 1;
    if multiplier > 4 {
        return Err(TaoError::InvalidData(
            "Vorbis floor1 multiplier 非法".into(),
        ));
    }

    let range_bits = br.read_bits(4)?;
    let mut x_list = vec![0u16, (1u32 << range_bits) as u16];
    for class_num in &partition_classes {
        let class_idx = *class_num as usize;
        let dim = classes[class_idx].dimensions as u32;
        for _ in 0..dim {
            let x = br.read_bits(range_bits as u8)? as u16;
            x_list.push(x);
        }
    }

    Ok(Floor1Config {
        partitions: partitions as u8,
        partition_classes: partition_classes.into_iter().map(|v| v as u8).collect(),
        classes,
        multiplier: multiplier as u8,
        range_bits: range_bits as u8,
        x_list,
    })
}

fn parse_residues(br: &mut LsbBitReader<'_>) -> TaoResult<Vec<ResidueConfig>> {
    let residue_count = br.read_bits(6)? + 1;
    let mut residues = Vec::with_capacity(residue_count as usize);
    for _ in 0..residue_count {
        let residue_type = br.read_bits(16)?;
        if residue_type > 2 {
            return Err(TaoError::InvalidData(format!(
                "Vorbis residue_type 不支持: {}",
                residue_type,
            )));
        }
        let begin = br.read_bits(24)?;
        let end = br.read_bits(24)?;
        let partition_size = br.read_bits(24)? + 1;
        let classifications = br.read_bits(6)? + 1;
        let classbook = br.read_bits(8)? as u8;

        let mut cascades = vec![0u8; classifications as usize];
        for cascade in &mut cascades {
            let low_bits = br.read_bits(3)?;
            let bitflag = br.read_flag()?;
            let high_bits = if bitflag { br.read_bits(5)? } else { 0 };
            *cascade = ((high_bits << 3) | low_bits) as u8;
        }

        let mut books = vec![[None; 8]; classifications as usize];
        for (ci, cascade) in cascades.iter().copied().enumerate() {
            for (bit, slot) in books[ci].iter_mut().enumerate().take(8) {
                if (cascade & (1 << bit)) != 0 {
                    *slot = Some(br.read_bits(8)? as u8);
                }
            }
        }

        residues.push(ResidueConfig {
            residue_type: residue_type as u8,
            begin,
            end,
            partition_size,
            classifications: classifications as u8,
            classbook,
            cascades,
            books,
        });
    }
    Ok(residues)
}

fn parse_mappings(
    br: &mut LsbBitReader<'_>,
    channels: u8,
    floor_count: u32,
    residue_count: u32,
) -> TaoResult<Vec<MappingConfig>> {
    let mapping_count = br.read_bits(6)? + 1;
    let mut mappings = Vec::with_capacity(mapping_count as usize);
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

        let mut coupling_steps_v = Vec::new();
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
                coupling_steps_v.push(CouplingStep {
                    magnitude: magnitude as u8,
                    angle: angle as u8,
                });
            }
        }

        let reserved = br.read_bits(2)?;
        if reserved != 0 {
            return Err(TaoError::InvalidData(
                "Vorbis mapping reserved bits 必须为 0".into(),
            ));
        }

        let mut channel_mux = vec![0u8; channels as usize];
        if submaps > 1 {
            for ch in 0..channels {
                let mux = br.read_bits(4)?;
                if mux >= submaps {
                    return Err(TaoError::InvalidData("Vorbis mapping mux 值越界".into()));
                }
                channel_mux[ch as usize] = mux as u8;
            }
        }

        let mut submap_floor = Vec::with_capacity(submaps as usize);
        let mut submap_residue = Vec::with_capacity(submaps as usize);
        for _ in 0..submaps {
            let _time_submap = br.read_bits(8)?;
            let floor = br.read_bits(8)?;
            let residue = br.read_bits(8)?;
            if floor >= floor_count || residue >= residue_count {
                return Err(TaoError::InvalidData(
                    "Vorbis mapping floor/residue 索引越界".into(),
                ));
            }
            submap_floor.push(floor as u8);
            submap_residue.push(residue as u8);
        }

        mappings.push(MappingConfig {
            coupling_steps: coupling_steps_v,
            channel_mux,
            submap_floor,
            submap_residue,
        });
    }
    Ok(mappings)
}

fn parse_modes(br: &mut LsbBitReader<'_>, mapping_count: u32) -> TaoResult<(Vec<bool>, Vec<u8>)> {
    let mode_count = br.read_bits(6)? + 1;
    let mut mode_flags = Vec::with_capacity(mode_count as usize);
    let mut mode_mappings = Vec::with_capacity(mode_count as usize);
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
        mode_mappings.push(mapping as u8);
    }
    Ok((mode_flags, mode_mappings))
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

fn vorbis_float32_unpack(v: u32) -> f32 {
    let sign = (v >> 31) & 0x01;
    let exponent = ((v >> 21) & 0x03ff) as i32;
    let mantissa = (v & 0x001f_ffff) as i32;
    let signed_mantissa = if sign != 0 { -mantissa } else { mantissa };
    (signed_mantissa as f32) * 2.0f32.powi(exponent - 788)
}
