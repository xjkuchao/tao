//! MP3 Huffman 解码器
//!
//! 实现基于静态表的 Huffman 解码 (VLC)

use std::sync::OnceLock;
use super::tables::{MPA_HUFF_LENS, MPA_HUFF_SYMS, MPA_HUFF_OFFSET};
use super::bitreader::BitReader;
use tao_core::{TaoError, TaoResult};

/// VLC 表项
#[derive(Debug, Clone)]
struct VlcEntry {
    /// 码长 (bits)
    len: u8,
    /// 码字 (value)
    code: u32,
    /// 符号 (decoded value)
    symbol: u8,
}

/// VLC 表
#[derive(Debug, Clone, Default)]
struct VlcTable {
    entries: Vec<VlcEntry>,
    // 优化: 可以添加查找表 (Lookup Table) 以加速短码
}

/// 全局 VLC 表缓存 (34 张表)
static VLC_TABLES: OnceLock<Vec<VlcTable>> = OnceLock::new();

/// 初始化并获取 VLC 表
fn get_vlc_tables() -> &'static Vec<VlcTable> {
    VLC_TABLES.get_or_init(|| {
        let mut tables = Vec::with_capacity(34);
        for i in 0..34 {
            tables.push(build_vlc_table(i as u8));
        }
        tables
    })
}

/// 构建单张 VLC 表 (Canonical Huffman)
fn build_vlc_table(table_id: u8) -> VlcTable {
    if table_id == 0 || table_id == 4 || table_id == 14 {
        return VlcTable::default();
    }

    // 获取该表的长度和符号切片
    let offset = MPA_HUFF_OFFSET[table_id as usize];
    
    // 确定该表的条目数 (根据 offset 和下一个 offset, 或者硬编码)
    // 由于我们没有存储每个表的大小, 我们需要推断.
    // 简单方法: 假设 MPA_HUFF_OFFSET 是递增的, 可以通过 next_offset - curr_offset 计算.
    // 但 16-23 共享, 24-31 共享.
    // 我们可以硬编码大小或逻辑判断.
    
    let count = match table_id {
        1 => 4,
        2..=3 => 9,
        5..=6 => 16,
        7..=9 => 36,
        10..=12 => 64,
        13 | 15 => 256,
        16..=23 => 256, // Table 16
        24..=31 => 256, // Table 24
        32 | 33 => 16, // Count1
        _ => 0,
    };

    if count == 0 {
        return VlcTable::default();
    }

    let lens = &MPA_HUFF_LENS[offset..offset + count];
    let syms = &MPA_HUFF_SYMS[offset..offset + count];

    // 1. 统计每个长度的码字数量 (bl_count)
    let mut bl_count = [0u32; 33]; // max len 32
    let mut max_len = 0;
    for &len in lens {
        if len > 0 {
            bl_count[len as usize] += 1;
            if len > max_len {
                max_len = len;
            }
        }
    }

    // 2. 计算每个长度的起始码字 (next_code)
    let mut next_code = [0u32; 33];
    let mut code = 0u32;
    // Canonical Huffman: code for length L is (code for L-1 + count[L-1]) << 1
    // 注意: 这里假设长度是连续增加的? 不, Canonical Huffman 规则:
    // 码字按长度排序, 相同长度按符号排序?
    // 通常: code[len] = (code[len-1] + bl_count[len-1]) << 1
    for len in 1..=max_len as usize {
        code = (code + bl_count[len - 1]) << 1;
        next_code[len] = code;
    }

    // 3. 分配码字
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let len = lens[i];
        if len > 0 {
            let code_val = next_code[len as usize];
            next_code[len as usize] += 1;
            
            entries.push(VlcEntry {
                len,
                code: code_val,
                symbol: syms[i],
            });
        }
    }

    VlcTable { entries }
}

pub struct HuffmanDecoder {
    // 无需状态, 使用全局静态表
}

impl HuffmanDecoder {
    pub fn new() -> Self {
        Self {}
    }

    /// 解码 (通用)
    /// 返回 (symbol, len)
    fn decode_vlc(&self, br: &mut BitReader, table_id: u8) -> TaoResult<u8> {
        let tables = get_vlc_tables();
        if table_id as usize >= tables.len() {
            return Err(TaoError::InvalidData(format!("Invalid Huffman table id: {}", table_id)));
        }
        let table = &tables[table_id as usize];
        if table.entries.is_empty() {
            return Ok(0);
        }

        // 线性搜索 (简单实现, 性能较低)
        // 优化: 可以一次 peek 很多位
        // 这里为了正确性先 peek 逐个尝试
        // 实际上, 我们应该 peek max_len, 然后查表.
        // 但这里 entries 没有按 code 排序(虽然 build 时是按 index).
        
        // 更好的方法: 逐位读取直到匹配?
        // Canonical codes have prefix property.
        // 我们可以遍历 entries, 看哪个匹配 peek 的位.
        // 由于是前缀码, 唯一匹配.
        
        // 优化: 按长度排序 entries, 然后 peek max bits.
        // 但为了简单:
        // peek 32 bits (or max possible), compare with entries.
        // entry.code matches peeked bits (high bits)?
        // code is usually aligned to MSB of the length.
        
        let val = br.peek_bits(32).unwrap_or(0); // 假设 max len <= 32
        
        for entry in &table.entries {
            // 检查前 entry.len 位是否等于 entry.code
            // val 的高 entry.len 位
            if entry.len == 0 { continue; }
            let shift = 32 - entry.len;
            let peeked_code = val >> shift;
            
            if peeked_code == entry.code {
                br.skip_bits(entry.len as usize);
                return Ok(entry.symbol);
            }
        }

        Err(TaoError::InvalidData("Huffman decode failed".to_string()))
    }

    /// 解码 Big Values (x, y)
    /// table_id: 1..31
    pub fn decode_big_values(&self, br: &mut BitReader, table_id: u8, linbits: u8) -> TaoResult<(i32, i32)> {
        let symbol = self.decode_vlc(br, table_id)?;
        
        let mut x = (symbol >> 4) as i32;
        let mut y = (symbol & 0x0F) as i32;

        if table_id > 15 {
            // Escaped values (linbits)
            if x == 15 && linbits > 0 {
                x += br.read_bits(linbits).ok_or(TaoError::Eof)? as i32;
            }
            if y == 15 && linbits > 0 {
                y += br.read_bits(linbits).ok_or(TaoError::Eof)? as i32;
            }
        }

        if x > 0 {
            if br.read_bool().ok_or(TaoError::Eof)? {
                x = -x;
            }
        }
        if y > 0 {
            if br.read_bool().ok_or(TaoError::Eof)? {
                y = -y;
            }
        }

        Ok((x, y))
    }

    /// 解码 Count1 (v, w, x, y)
    /// table_id: 32 or 33
    pub fn decode_count1(&self, br: &mut BitReader, table_id: u8) -> TaoResult<(i32, i32, i32, i32)> {
        let symbol = self.decode_vlc(br, table_id)?;
        
        // Count1 symbol mapping: v << 3 | w << 2 | x << 1 | y
        // values are 0 or 1.
        let mut v = ((symbol >> 3) & 1) as i32;
        let mut w = ((symbol >> 2) & 1) as i32;
        let mut x = ((symbol >> 1) & 1) as i32;
        let mut y = (symbol & 1) as i32;

        // Apply signs
        if v > 0 && br.read_bool().ok_or(TaoError::Eof)? { v = -v; }
        if w > 0 && br.read_bool().ok_or(TaoError::Eof)? { w = -w; }
        if x > 0 && br.read_bool().ok_or(TaoError::Eof)? { x = -x; }
        if y > 0 && br.read_bool().ok_or(TaoError::Eof)? { y = -y; }

        Ok((v, w, x, y))
    }
}
