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

    for y in &mut final_y {
        *y = (*y).clamp(0, range - 1);
    }

    let mut sorted: Vec<(usize, u16)> = cfg.x_list.iter().copied().enumerate().collect();
    sorted.sort_by(|a, b| a.1.cmp(&b.1));

    let mut floor_idx = Vec::<u32>::with_capacity(n2);
    let mut lx = 0u32;
    let mut hx = 0u32;
    let mut ly = (final_y[sorted[0].0] * i32::from(cfg.multiplier)) as u32;
    let mut hy = ly;

    for (idx, x) in sorted.into_iter().skip(1) {
        if !step2[idx] {
            continue;
        }
        hy = (final_y[idx] * i32::from(cfg.multiplier)) as u32;
        hx = u32::from(x);
        render_line(lx, ly, hx, hy, &mut floor_idx);
        lx = hx;
        ly = hy;
    }

    let n = n2 as u32;
    if hx < n {
        render_line(hx, hy, n, hy, &mut floor_idx);
    } else if hx > n {
        floor_idx.truncate(n2);
    }

    if floor_idx.len() < n2 {
        floor_idx.resize(n2, hy);
    } else if floor_idx.len() > n2 {
        floor_idx.truncate(n2);
    }

    Ok(floor_idx
        .into_iter()
        .map(|v| FLOOR1_INVERSE_DB_TABLE[v.min(255) as usize])
        .collect())
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
    let mut high_idx = 0usize;
    let mut low_x = 0u16;
    let mut high_x = u16::MAX;
    for (j, &xj) in x_list.iter().enumerate().take(i) {
        if xj < xi && xj >= low_x {
            low_x = xj;
            low_idx = j;
        }
        if xj > xi && xj <= high_x {
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
    let ady = dy.abs();
    let adx2 = x - x0;
    let err = ady * adx2;
    let off = err / adx;
    if dy < 0 { y0 - off } else { y0 + off }
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
        predicted - (v - highroom) - 1
    }
}

fn render_line(x0: u32, y0: u32, x1: u32, y1: u32, out: &mut Vec<u32>) {
    if x1 <= x0 {
        return;
    }
    let dy = y1 as i32 - y0 as i32;
    let adx = x1 as i32 - x0 as i32;
    let mut ady = dy.abs();
    let base = dy / adx;
    let mut y = y0 as i32;
    let mut err = 0i32;
    let sy = base + if dy < 0 { -1 } else { 1 };
    ady -= base.abs() * adx;
    out.push(y as u32);
    for _ in (x0 + 1)..x1 {
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base;
        }
        out.push(y as u32);
    }
}

#[allow(clippy::excessive_precision)]
static FLOOR1_INVERSE_DB_TABLE: &[f32; 256] = &[
    1.0649863e-07,
    1.1341951e-07,
    1.2079015e-07,
    1.2863978e-07,
    1.3699951e-07,
    1.4590251e-07,
    1.5538408e-07,
    1.6548181e-07,
    1.7623575e-07,
    1.8768855e-07,
    1.9988561e-07,
    2.1287530e-07,
    2.2670913e-07,
    2.4144197e-07,
    2.5713223e-07,
    2.7384213e-07,
    2.9163793e-07,
    3.1059021e-07,
    3.3077411e-07,
    3.5226968e-07,
    3.7516214e-07,
    3.9954229e-07,
    4.2550680e-07,
    4.5315863e-07,
    4.8260743e-07,
    5.1396998e-07,
    5.4737065e-07,
    5.8294187e-07,
    6.2082472e-07,
    6.6116941e-07,
    7.0413592e-07,
    7.4989464e-07,
    7.9862701e-07,
    8.5052630e-07,
    9.0579828e-07,
    9.6466216e-07,
    1.0273513e-06,
    1.0941144e-06,
    1.1652161e-06,
    1.2409384e-06,
    1.3215816e-06,
    1.4074654e-06,
    1.4989305e-06,
    1.5963394e-06,
    1.7000785e-06,
    1.8105592e-06,
    1.9282195e-06,
    2.0535261e-06,
    2.1869758e-06,
    2.3290978e-06,
    2.4804557e-06,
    2.6416497e-06,
    2.8133190e-06,
    2.9961443e-06,
    3.1908506e-06,
    3.3982101e-06,
    3.6190449e-06,
    3.8542308e-06,
    4.1047004e-06,
    4.3714470e-06,
    4.6555282e-06,
    4.9580707e-06,
    5.2802740e-06,
    5.6234160e-06,
    5.9888572e-06,
    6.3780469e-06,
    6.7925283e-06,
    7.2339451e-06,
    7.7040476e-06,
    8.2047000e-06,
    8.7378876e-06,
    9.3057248e-06,
    9.9104632e-06,
    1.0554501e-05,
    1.1240392e-05,
    1.1970856e-05,
    1.2748789e-05,
    1.3577278e-05,
    1.4459606e-05,
    1.5399272e-05,
    1.6400004e-05,
    1.7465768e-05,
    1.8600792e-05,
    1.9809576e-05,
    2.1096914e-05,
    2.2467911e-05,
    2.3928002e-05,
    2.5482978e-05,
    2.7139006e-05,
    2.8902651e-05,
    3.0780908e-05,
    3.2781225e-05,
    3.4911534e-05,
    3.7180282e-05,
    3.9596466e-05,
    4.2169667e-05,
    4.4910090e-05,
    4.7828601e-05,
    5.0936773e-05,
    5.4246931e-05,
    5.7772202e-05,
    6.1526565e-05,
    6.5524908e-05,
    6.9783085e-05,
    7.4317983e-05,
    7.9147585e-05,
    8.4291040e-05,
    8.9768747e-05,
    9.5602426e-05,
    1.0181521e-04,
    1.0843174e-04,
    1.1547824e-04,
    1.2298267e-04,
    1.3097477e-04,
    1.3948625e-04,
    1.4855085e-04,
    1.5820453e-04,
    1.6848555e-04,
    1.7943469e-04,
    1.9109536e-04,
    2.0351382e-04,
    2.1673929e-04,
    2.3082423e-04,
    2.4582449e-04,
    2.6179955e-04,
    2.7881275e-04,
    2.9693158e-04,
    3.1622787e-04,
    3.3677814e-04,
    3.5866388e-04,
    3.8197188e-04,
    4.0679456e-04,
    4.3323036e-04,
    4.6138411e-04,
    4.9136745e-04,
    5.2329927e-04,
    5.5730621e-04,
    5.9352311e-04,
    6.3209358e-04,
    6.7317058e-04,
    7.1691700e-04,
    7.6350630e-04,
    8.1312324e-04,
    8.6596457e-04,
    9.2223983e-04,
    9.8217216e-04,
    1.0459992e-03,
    1.1139742e-03,
    1.1863665e-03,
    1.2634633e-03,
    1.3455702e-03,
    1.4330129e-03,
    1.5261382e-03,
    1.6253153e-03,
    1.7309374e-03,
    1.8434235e-03,
    1.9632195e-03,
    2.0908006e-03,
    2.2266726e-03,
    2.3713743e-03,
    2.5254795e-03,
    2.6895994e-03,
    2.8643847e-03,
    3.0505286e-03,
    3.2487691e-03,
    3.4598925e-03,
    3.6847358e-03,
    3.9241906e-03,
    4.1792066e-03,
    4.4507950e-03,
    4.7400328e-03,
    5.0480668e-03,
    5.3761186e-03,
    5.7254891e-03,
    6.0975636e-03,
    6.4938176e-03,
    6.9158225e-03,
    7.3652516e-03,
    7.8438871e-03,
    8.3536271e-03,
    8.8964928e-03,
    9.4746370e-03,
    1.0090352e-02,
    1.0746080e-02,
    1.1444421e-02,
    1.2188144e-02,
    1.2980198e-02,
    1.3823725e-02,
    1.4722068e-02,
    1.5678791e-02,
    1.6697687e-02,
    1.7782797e-02,
    1.8938423e-02,
    2.0169149e-02,
    2.1479854e-02,
    2.2875735e-02,
    2.4362330e-02,
    2.5945531e-02,
    2.7631618e-02,
    2.9427276e-02,
    3.1339626e-02,
    3.3376252e-02,
    3.5545228e-02,
    3.7855157e-02,
    4.0315199e-02,
    4.2935108e-02,
    4.5725273e-02,
    4.8696758e-02,
    5.1861348e-02,
    5.5231591e-02,
    5.8820850e-02,
    6.2643361e-02,
    6.6714279e-02,
    7.1049749e-02,
    7.5666962e-02,
    8.0584227e-02,
    8.5821044e-02,
    9.1398179e-02,
    9.7337747e-02,
    1.0366330e-01,
    1.1039993e-01,
    1.1757434e-01,
    1.2521498e-01,
    1.3335215e-01,
    1.4201813e-01,
    1.5124727e-01,
    1.6107617e-01,
    1.7154380e-01,
    1.8269168e-01,
    1.9456402e-01,
    2.0720788e-01,
    2.2067342e-01,
    2.3501402e-01,
    2.5028656e-01,
    2.6655159e-01,
    2.8387361e-01,
    3.0232132e-01,
    3.2196786e-01,
    3.4289114e-01,
    3.6517414e-01,
    3.8890521e-01,
    4.1417847e-01,
    4.4109412e-01,
    4.6975890e-01,
    5.0028648e-01,
    5.3279791e-01,
    5.6742212e-01,
    6.0429640e-01,
    6.4356699e-01,
    6.8538959e-01,
    7.2993007e-01,
    7.7736504e-01,
    8.2788260e-01,
    8.8168307e-01,
    9.3897980e-01,
    1.0,
];
