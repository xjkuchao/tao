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

        let mut count = vec![0u32; max_len as usize + 1];
        for &len in lengths {
            if len > 0 {
                count[len as usize] = count[len as usize].saturating_add(1);
            }
        }

        let mut next_code = vec![0u32; max_len as usize + 1];
        let mut code = 0u32;
        for len in 1..=max_len as usize {
            code = (code + count[len - 1]) << 1;
            next_code[len] = code;
        }

        let mut nodes = vec![HuffNode::new()];
        for (sym, &len) in lengths.iter().enumerate() {
            if len == 0 {
                continue;
            }
            let codeword = next_code[len as usize];
            next_code[len as usize] = next_code[len as usize].saturating_add(1);
            let rev = reverse_bits(codeword, len);
            insert_codeword(&mut nodes, rev, len, sym as u32)?;
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

fn insert_codeword(nodes: &mut Vec<HuffNode>, code: u32, len: u8, sym: u32) -> TaoResult<()> {
    let mut idx = 0usize;
    for i in 0..len {
        if nodes[idx].sym.is_some() {
            return Err(TaoError::InvalidData(
                "Vorbis codebook Huffman 码冲突".into(),
            ));
        }
        let bit = (code >> i) & 1;
        let next_idx = if bit == 0 {
            if let Some(v) = nodes[idx].left {
                v
            } else {
                let new_idx = nodes.len();
                nodes.push(HuffNode::new());
                nodes[idx].left = Some(new_idx);
                new_idx
            }
        } else if let Some(v) = nodes[idx].right {
            v
        } else {
            let new_idx = nodes.len();
            nodes.push(HuffNode::new());
            nodes[idx].right = Some(new_idx);
            new_idx
        };
        idx = next_idx;
    }
    if nodes[idx].sym.is_some() {
        return Err(TaoError::InvalidData(
            "Vorbis codebook Huffman 码冲突".into(),
        ));
    }
    nodes[idx].sym = Some(sym);
    Ok(())
}

pub(crate) fn decode_codebook_scalar(
    br: &mut LsbBitReader<'_>,
    book: &CodebookConfig,
    huffman: &CodebookHuffman,
) -> TaoResult<u32> {
    let sym = huffman.decode_symbol(br)?;
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
        out[..fill_dims].fill(0.0);
        let _ = decode_codebook_scalar(br, book, huffman)?;
        return Ok(fill_dims);
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
    let mut index_divisor = 1usize;
    for (i, slot) in out.iter_mut().enumerate().take(fill_dims) {
        let m_idx = if lookup_type == 1 {
            (sym / index_divisor) % lookup.lookup_values as usize
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
            index_divisor = index_divisor
                .checked_mul(lookup.lookup_values as usize)
                .ok_or_else(|| TaoError::InvalidData("Vorbis codebook divisor 溢出".into()))?;
        }
    }
    Ok(())
}

fn reverse_bits(mut v: u32, n: u8) -> u32 {
    let mut out = 0u32;
    for _ in 0..n {
        out = (out << 1) | (v & 1);
        v >>= 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_huffman_构建与解码() {
        let h = CodebookHuffman::from_lengths(&[1, 3, 3, 3]).expect("构建失败");
        // lengths 对应 canonical 码(按 symbol 升序):
        // sym0(len1)=0, sym1(len3)=100, sym2(len3)=101, sym3(len3)=110
        // LSB 位流拼接: sym0=0, sym1=001(LSB-first 为 100), sym2=101。
        let data = [0b0101_0010u8]; // bits(LSB-first): 0 | 100 | 101 ...
        let mut br = LsbBitReader::new(&data);
        let s0 = h.decode_symbol(&mut br).expect("sym0 解码失败");
        assert_eq!(s0, 0, "第一个符号应为 sym0");
        let s1 = h.decode_symbol(&mut br).expect("sym1 解码失败");
        assert_eq!(s1, 1, "第二个符号应为 sym1");
        let s2 = h.decode_symbol(&mut br).expect("sym2 解码失败");
        assert_eq!(s2, 2, "第三个符号应为 sym2");
    }
}
