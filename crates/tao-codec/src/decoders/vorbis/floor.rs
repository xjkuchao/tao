use std::cmp::Ordering;

use tao_core::{TaoError, TaoResult};

use super::bitreader::LsbBitReader;
use super::codebook::CodebookHuffman;
use super::setup::{Floor1Config, FloorConfig, MappingConfig, ParsedSetup};

/// floor 恢复阶段上下文.
#[derive(Debug, Clone)]
pub(crate) struct FloorContext {
    pub(crate) channel_count: usize,
    pub(crate) floor_index_per_channel: Vec<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct FloorCurves {
    pub(crate) channel_curves: Vec<Vec<f32>>,
    pub(crate) nonzero: Vec<bool>,
}

/// 基于 setup 与当前包头信息构建 floor 阶段上下文.
pub(crate) fn build_floor_context(
    setup: &ParsedSetup,
    mapping: &MappingConfig,
    channel_count: usize,
) -> TaoResult<FloorContext> {
    if setup.floor_count == 0 {
        return Err(tao_core::TaoError::InvalidData(
            "Vorbis floor_count 非法".into(),
        ));
    }
    let mut floor_index_per_channel = vec![0usize; channel_count];
    for (ch, slot) in floor_index_per_channel.iter_mut().enumerate() {
        let mux = mapping.channel_mux.get(ch).copied().unwrap_or(0) as usize;
        let floor_idx = mapping.submap_floor.get(mux).copied().ok_or_else(|| {
            tao_core::TaoError::InvalidData("Vorbis mapping floor 子映射索引越界".into())
        })? as usize;
        if floor_idx >= setup.floors.len() {
            return Err(tao_core::TaoError::InvalidData(
                "Vorbis floor 索引越界".into(),
            ));
        }
        *slot = floor_idx;
    }
    Ok(FloorContext {
        channel_count,
        floor_index_per_channel,
    })
}

pub(crate) fn decode_floor_curves(
    br: &mut LsbBitReader<'_>,
    setup: &ParsedSetup,
    floor_ctx: &FloorContext,
    huffmans: &[CodebookHuffman],
    n2: usize,
) -> TaoResult<FloorCurves> {
    let mut channel_curves = vec![vec![1.0f32; n2]; floor_ctx.channel_count];
    let mut nonzero = vec![false; floor_ctx.channel_count];

    for ch in 0..floor_ctx.channel_count {
        let floor_idx = floor_ctx.floor_index_per_channel[ch];
        let floor = setup
            .floors
            .get(floor_idx)
            .ok_or_else(|| TaoError::InvalidData("Vorbis floor 索引越界".into()))?;
        let used = br.read_flag()?;
        nonzero[ch] = used;
        if !used {
            channel_curves[ch].fill(0.0);
            continue;
        }

        match floor {
            FloorConfig::Floor0 => {
                channel_curves[ch].fill(1.0);
            }
            FloorConfig::Floor1(cfg) => {
                channel_curves[ch] = decode_floor1_curve(br, setup, cfg, huffmans, n2)?;
            }
        }
    }

    Ok(FloorCurves {
        channel_curves,
        nonzero,
    })
}

fn decode_floor1_curve(
    br: &mut LsbBitReader<'_>,
    setup: &ParsedSetup,
    cfg: &Floor1Config,
    huffmans: &[CodebookHuffman],
    n2: usize,
) -> TaoResult<Vec<f32>> {
    let range = match cfg.multiplier {
        1 => 256i32,
        2 => 128,
        3 => 86,
        4 => 64,
        _ => {
            return Err(TaoError::InvalidData(
                "Vorbis floor1 multiplier 非法".into(),
            ));
        }
    };

    let mut y_list = Vec::<i32>::with_capacity(cfg.x_list.len());
    let y0 = br.read_bits(cfg.range_bits)? as i32;
    let y1 = br.read_bits(cfg.range_bits)? as i32;
    y_list.push(y0);
    y_list.push(y1);

    for &class_num in &cfg.partition_classes {
        let class = cfg
            .classes
            .get(class_num as usize)
            .ok_or_else(|| TaoError::InvalidData("Vorbis floor1 class 索引越界".into()))?;
        let cbits = class.subclasses;
        let mut cval = 0u32;
        if cbits > 0 {
            let masterbook = class
                .masterbook
                .ok_or_else(|| TaoError::InvalidData("Vorbis floor1 masterbook 缺失".into()))?;
            cval = decode_codebook_scalar(br, setup, huffmans, masterbook as usize)?;
        }
        let csub = (1u32 << cbits) - 1;
        for _ in 0..class.dimensions {
            let book_opt = class
                .subclass_books
                .get((cval & csub) as usize)
                .ok_or_else(|| TaoError::InvalidData("Vorbis floor1 subclass book 越界".into()))?;
            cval >>= cbits;
            let yv = if let Some(book) = *book_opt {
                decode_codebook_scalar(br, setup, huffmans, book as usize)? as i32
            } else {
                0
            };
            y_list.push(yv);
        }
    }

    if y_list.len() != cfg.x_list.len() {
        return Err(TaoError::InvalidData("Vorbis floor1 点数量不匹配".into()));
    }

    let mut step2 = vec![false; y_list.len()];
    step2[0] = true;
    step2[1] = true;
    let mut final_y = vec![0i32; y_list.len()];
    final_y[0] = y_list[0];
    final_y[1] = y_list[1];

    for i in 2..y_list.len() {
        let (low, high) = find_neighbors(&cfg.x_list, i);
        let predicted = render_point(
            cfg.x_list[low] as i32,
            final_y[low],
            cfg.x_list[high] as i32,
            final_y[high],
            cfg.x_list[i] as i32,
        );
        let val = y_list[i];
        if val != 0 {
            step2[i] = true;
            step2[low] = true;
            step2[high] = true;
            final_y[i] = dequantize_floor1_y(val, predicted, range);
        } else {
            final_y[i] = predicted;
        }
    }

    let mut points: Vec<(i32, i32)> = (0..cfg.x_list.len())
        .filter(|&i| step2[i])
        .map(|i| (cfg.x_list[i] as i32, final_y[i]))
        .collect();
    points.sort_by(|a, b| {
        if a.0 == b.0 {
            Ordering::Equal
        } else if a.0 < b.0 {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    });

    let mut curve = vec![0.0f32; n2];
    if points.is_empty() {
        return Ok(curve);
    }

    for w in points.windows(2) {
        let (x0, y0) = w[0];
        let (x1, y1) = w[1];
        if x1 <= x0 {
            continue;
        }
        let start = x0.max(0) as usize;
        let end = x1.min(n2 as i32 - 1) as usize;
        if start >= n2 {
            continue;
        }
        for (x, v) in curve
            .iter_mut()
            .enumerate()
            .take(end.saturating_add(1))
            .skip(start)
        {
            let t = (x as f32 - x0 as f32) / (x1 as f32 - x0 as f32);
            let y = y0 as f32 + (y1 as f32 - y0 as f32) * t;
            *v = approx_floor_gain(y, range as f32);
        }
    }

    Ok(curve)
}

fn decode_codebook_scalar(
    br: &mut LsbBitReader<'_>,
    setup: &ParsedSetup,
    huffmans: &[CodebookHuffman],
    book_idx: usize,
) -> TaoResult<u32> {
    let book = setup
        .codebooks
        .get(book_idx)
        .ok_or_else(|| TaoError::InvalidData("Vorbis codebook 索引越界".into()))?;
    let h = huffmans
        .get(book_idx)
        .ok_or_else(|| TaoError::InvalidData("Vorbis Huffman 表索引越界".into()))?;
    let sym = h.decode_symbol(br)?;
    if sym >= book.entries {
        return Err(TaoError::InvalidData(
            "Vorbis codebook 符号超出 entries".into(),
        ));
    }
    Ok(sym)
}

fn find_neighbors(x_list: &[u16], i: usize) -> (usize, usize) {
    let xi = x_list[i];
    let mut low_idx = 0usize;
    let mut high_idx = 1usize;
    let mut low_x = 0u16;
    let mut high_x = u16::MAX;
    for (j, &xj) in x_list.iter().enumerate().take(i) {
        if xj <= xi && xj >= low_x {
            low_x = xj;
            low_idx = j;
        }
        if xj >= xi && xj <= high_x {
            high_x = xj;
            high_idx = j;
        }
    }
    (low_idx, high_idx)
}

fn render_point(x0: i32, y0: i32, x1: i32, y1: i32, x: i32) -> i32 {
    if x1 == x0 {
        return y0;
    }
    let dy = y1 - y0;
    let adx = x1 - x0;
    y0 + dy * (x - x0) / adx
}

fn dequantize_floor1_y(v: i32, predicted: i32, range: i32) -> i32 {
    let highroom = range - predicted;
    let lowroom = predicted;
    let room = 2 * highroom.min(lowroom);
    if v < room {
        if (v & 1) == 1 {
            predicted - ((v + 1) >> 1)
        } else {
            predicted + (v >> 1)
        }
    } else if highroom > lowroom {
        predicted + (v - lowroom)
    } else {
        predicted - (v - highroom) + 1
    }
}

fn approx_floor_gain(y: f32, range: f32) -> f32 {
    if range <= 0.0 {
        return 1.0;
    }
    let norm = (y / range).clamp(0.0, 1.0);
    (1.0 - norm).powf(2.0)
}
