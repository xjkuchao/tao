use tao_core::{TaoError, TaoResult};

use super::bitreader::LsbBitReader;
use super::setup::{CodebookConfig, CodebookLookupConfig};

#[derive(Debug, Clone)]
pub(crate) struct CodebookHuffman {
    max_len: u8,
    nodes: Vec<HuffNode>,
}

impl CodebookHuffman {
    pub(crate) fn from_lengths(lengths: &[u8]) -> TaoResult<Self> {
        let max_len = lengths.iter().copied().max().unwrap_or(0);
        if max_len == 0 {
            return Ok(Self {
                max_len,
                nodes: vec![HuffNode::new()],
            });
        }

        let codewords = synthesize_codewords(lengths)?;
        let mut nodes = vec![HuffNode::new()];

        for (sym, &len) in lengths.iter().enumerate() {
            if len == 0 {
                continue;
            }
            let code = reverse_bits_len(codewords[sym], len);
            let mut node_idx = 0usize;

            for bit_pos in 0..len {
                if nodes[node_idx].sym.is_some() {
                    return Err(TaoError::InvalidData(
                        "Vorbis codebook Huffman 长度表过度指定".into(),
                    ));
                }
                let bit = (code >> bit_pos) & 1;
                let existing = if bit == 0 {
                    nodes[node_idx].left
                } else {
                    nodes[node_idx].right
                };
                let next_idx = if let Some(idx) = existing {
                    idx
                } else {
                    let idx = nodes.len();
                    nodes.push(HuffNode::new());
                    if bit == 0 {
                        nodes[node_idx].left = Some(idx);
                    } else {
                        nodes[node_idx].right = Some(idx);
                    }
                    idx
                };
                node_idx = next_idx;
            }

            if nodes[node_idx].left.is_some() || nodes[node_idx].right.is_some() {
                return Err(TaoError::InvalidData(
                    "Vorbis codebook Huffman 长度表过度指定".into(),
                ));
            }
            if nodes[node_idx].sym.is_some() {
                return Err(TaoError::InvalidData(
                    "Vorbis codebook Huffman 长度表过度指定".into(),
                ));
            }
            nodes[node_idx].sym = Some(sym as u32);
        }

        if !nodes
            .iter()
            .any(|n| n.sym.is_some() || n.left.is_some() || n.right.is_some())
        {
            return Err(TaoError::InvalidData(
                "Vorbis codebook Huffman 长度表非法".into(),
            ));
        }

        Ok(Self { max_len, nodes })
    }

    pub(crate) fn decode_symbol(&self, br: &mut LsbBitReader<'_>) -> TaoResult<u32> {
        let mut node_idx = 0usize;
        for _ in 0..self.max_len {
            let bit = br.read_bits(1)?;
            let next = if bit == 0 {
                self.nodes
                    .get(node_idx)
                    .and_then(|n| n.left)
                    .ok_or_else(|| TaoError::InvalidData("Vorbis Huffman 解码失败".into()))?
            } else {
                self.nodes
                    .get(node_idx)
                    .and_then(|n| n.right)
                    .ok_or_else(|| TaoError::InvalidData("Vorbis Huffman 解码失败".into()))?
            };
            node_idx = next;
            if let Some(sym) = self.nodes[node_idx].sym {
                return Ok(sym);
            }
        }
        Err(TaoError::InvalidData(
            "Vorbis codebook Huffman 解码失败".into(),
        ))
    }
}

#[derive(Debug, Clone)]
struct HuffNode {
    left: Option<usize>,
    right: Option<usize>,
    sym: Option<u32>,
}

impl HuffNode {
    fn new() -> Self {
        Self {
            left: None,
            right: None,
            sym: None,
        }
    }
}

fn synthesize_codewords(lengths: &[u8]) -> TaoResult<Vec<u32>> {
    let mut codewords = Vec::with_capacity(lengths.len());
    let mut next_codeword = [0u32; 33];
    let mut sparse_count = 0usize;

    for &len in lengths {
        if len > 32 {
            return Err(TaoError::InvalidData(
                "Vorbis codebook Huffman 码长非法".into(),
            ));
        }
        if len == 0 {
            sparse_count += 1;
            codewords.push(0);
            continue;
        }

        let codeword_len = usize::from(len);
        let codeword = next_codeword[codeword_len];
        if len < 32 && (codeword >> len) > 0 {
            return Err(TaoError::InvalidData(
                "Vorbis codebook Huffman 长度表过度指定".into(),
            ));
        }

        for i in (0..(codeword_len + 1)).rev() {
            if next_codeword[i] & 1 == 1 {
                if i == 0 {
                    return Err(TaoError::InvalidData(
                        "Vorbis codebook Huffman 长度表过度指定".into(),
                    ));
                }
                next_codeword[i] = next_codeword[i - 1] << 1;
                break;
            }
            next_codeword[i] = next_codeword[i].saturating_add(1);
        }

        let branch = next_codeword[codeword_len];
        for (i, next) in next_codeword[codeword_len..].iter_mut().enumerate().skip(1) {
            if *next == codeword << i {
                *next = branch << i;
            } else {
                break;
            }
        }

        codewords.push(codeword);
    }

    let underspecified = next_codeword
        .iter()
        .enumerate()
        .skip(1)
        .any(|(i, &c)| c & (u32::MAX >> (32 - i)) != 0);
    let single_entry = lengths.len().saturating_sub(sparse_count) == 1;
    if underspecified && !single_entry {
        return Err(TaoError::InvalidData(
            "Vorbis codebook Huffman 长度表欠指定".into(),
        ));
    }

    Ok(codewords)
}

#[inline]
fn reverse_bits_len(v: u32, len: u8) -> u32 {
    if len == 0 {
        0
    } else {
        v.reverse_bits() >> (32 - len)
    }
}

pub(crate) fn decode_codebook_scalar(
    br: &mut LsbBitReader<'_>,
    book: &CodebookConfig,
    huffman: &CodebookHuffman,
) -> TaoResult<u32> {
    let sym = match huffman.decode_symbol(br) {
        Ok(sym) => sym,
        Err(TaoError::InvalidData(msg)) if msg.contains("Huffman 解码失败") => {
            // 损坏流容错: 在位流局部失步时尝试小范围跳位重同步.
            // 该路径仅在原始 Huffman 解码失败时触发, 不影响正常样本.
            const MAX_RESYNC_BITS: u8 = 32;
            let mut recovered = None;
            for shift in 1..=MAX_RESYNC_BITS {
                let mut trial = br.clone();
                if trial.read_bits(shift).is_err() {
                    break;
                }
                let sym_try = match huffman.decode_symbol(&mut trial) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if sym_try < book.entries {
                    recovered = Some((sym_try, trial));
                    break;
                }
            }
            if let Some((sym_ok, trial_reader)) = recovered {
                *br = trial_reader;
                sym_ok
            } else {
                return Err(TaoError::InvalidData(msg));
            }
        }
        Err(e) => return Err(e),
    };
    if sym >= book.entries {
        return Err(TaoError::InvalidData(
            "Vorbis codebook 符号超出 entries".into(),
        ));
    }
    Ok(sym)
}

pub(crate) fn decode_codebook_vector(
    br: &mut LsbBitReader<'_>,
    book: &CodebookConfig,
    huffman: &CodebookHuffman,
    out: &mut [f32],
) -> TaoResult<usize> {
    let dims = usize::from(book.dimensions);
    if dims == 0 || out.is_empty() {
        return Ok(0);
    }
    let fill_dims = dims.min(out.len());
    if book.lookup_type == 0 {
        return Err(TaoError::InvalidData(
            "Vorbis 向量 codebook 缺少 value mapping".into(),
        ));
    }

    let sym = decode_codebook_scalar(br, book, huffman)? as usize;
    let lookup = book
        .lookup
        .as_ref()
        .ok_or_else(|| TaoError::InvalidData("Vorbis codebook lookup 缺失".into()))?;
    decode_vector_from_lookup(sym, dims, book.lookup_type, lookup, out)?;
    Ok(fill_dims)
}

fn decode_vector_from_lookup(
    sym: usize,
    dims: usize,
    lookup_type: u8,
    lookup: &CodebookLookupConfig,
    out: &mut [f32],
) -> TaoResult<()> {
    if lookup.lookup_values == 0 || lookup.multiplicands.is_empty() {
        return Err(TaoError::InvalidData(
            "Vorbis codebook multiplicands 非法".into(),
        ));
    }

    let fill_dims = dims.min(out.len());
    let mut last = 0.0f32;
    let sym_u64 = sym as u64;
    let lookup_values_u64 = lookup.lookup_values as u64;
    let mut index_divisor = 1u64;
    for (i, slot) in out.iter_mut().enumerate().take(fill_dims) {
        let m_idx = if lookup_type == 1 {
            ((sym_u64 / index_divisor) % lookup_values_u64) as usize
        } else if lookup_type == 2 {
            sym.checked_mul(dims)
                .and_then(|base| base.checked_add(i))
                .ok_or_else(|| TaoError::InvalidData("Vorbis codebook 索引溢出".into()))?
        } else {
            return Err(TaoError::InvalidData(
                "Vorbis codebook lookup_type 不支持".into(),
            ));
        };
        let mul = lookup
            .multiplicands
            .get(m_idx)
            .copied()
            .ok_or_else(|| TaoError::InvalidData("Vorbis codebook multiplicand 越界".into()))?;
        let mut v = lookup.minimum_value + lookup.delta_value * mul as f32 + last;
        if !lookup.sequence_p {
            v = lookup.minimum_value + lookup.delta_value * mul as f32;
        } else {
            last = v;
        }
        *slot = v;
        if lookup_type == 1 {
            index_divisor = index_divisor.saturating_mul(lookup_values_u64.max(1));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_huffman_build_and_decode() {
        let h = CodebookHuffman::from_lengths(&[1, 2, 2]).expect("构建失败");
        let data = [0b0001_1010u8];
        let mut br = LsbBitReader::new(&data);
        let s0 = h.decode_symbol(&mut br).expect("sym0 解码失败");
        assert_eq!(s0, 0, "第一个符号应为 sym0");
        let s1 = h.decode_symbol(&mut br).expect("sym1 解码失败");
        assert_eq!(s1, 1, "第二个符号应为 sym1");
        let s2 = h.decode_symbol(&mut br).expect("sym2 解码失败");
        assert_eq!(s2, 2, "第三个符号应为 sym2");
    }

    #[test]
    fn test_synthesize_codewords_reference() {
        let lengths = [2u8, 4, 4, 4, 4, 2, 3, 3];
        let expected = [0u32, 0x4, 0x5, 0x6, 0x7, 0x2, 0x6, 0x7];
        let codewords = synthesize_codewords(&lengths).expect("codewords 生成失败");
        assert_eq!(
            codewords.as_slice(),
            expected,
            "canonical codeword 应与参考一致"
        );
    }

    #[test]
    fn test_huffman_official_example_mapping() {
        let h = CodebookHuffman::from_lengths(&[2, 4, 4, 4, 4, 2, 3, 3]).expect("构建失败");
        let decode = |bits: u32, len: u8| {
            let mut v = 0u8;
            for i in 0..len {
                if ((bits >> i) & 1) != 0 {
                    v |= 1 << i;
                }
            }
            let buf = [v];
            let mut br = LsbBitReader::new(&buf);
            h.decode_symbol(&mut br).expect("解码失败")
        };
        assert_eq!(decode(0b00, 2), 0);
        assert_eq!(decode(0b0010, 4), 1);
        assert_eq!(decode(0b1010, 4), 2);
        assert_eq!(decode(0b0110, 4), 3);
        assert_eq!(decode(0b1110, 4), 4);
        assert_eq!(decode(0b01, 2), 5);
        assert_eq!(decode(0b011, 3), 6);
        assert_eq!(decode(0b111, 3), 7);
    }
}
