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

        let mut nonzero_count = 0usize;
        let mut last_nonzero_sym = 0usize;
        for (sym, &len) in lengths.iter().enumerate() {
            if len > 0 {
                nonzero_count += 1;
                last_nonzero_sym = sym;
            }
        }

        if nonzero_count == 1 {
            let only_len = lengths[last_nonzero_sym];
            if only_len != 1 {
                return Err(TaoError::InvalidData(
                    "Vorbis 单项 codebook 的码长必须为 1".into(),
                ));
            }
            let mut nodes = vec![HuffNode::new(), HuffNode::new(), HuffNode::new()];
            nodes[0].left = Some(1);
            nodes[0].right = Some(2);
            nodes[1].sym = Some(last_nonzero_sym as u32);
            nodes[2].sym = Some(last_nonzero_sym as u32);
            return Ok(Self { max_len: 1, nodes });
        }

        let mut root = BuildNode::new();
        for (sym, &len) in lengths.iter().enumerate() {
            if len == 0 {
                continue;
            }
            if !root.insert_rec(sym as u32, len) {
                return Err(TaoError::InvalidData(
                    "Vorbis codebook Huffman 长度表过度指定".into(),
                ));
            }
        }
        if !root.even_children {
            return Err(TaoError::InvalidData(
                "Vorbis codebook Huffman 长度表欠指定".into(),
            ));
        }

        let mut nodes = Vec::new();
        flatten_build_tree(&root, &mut nodes);
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

#[derive(Debug, Clone)]
struct BuildNode {
    even_children: bool,
    sym: Option<u32>,
    left: Option<Box<BuildNode>>,
    right: Option<Box<BuildNode>>,
}

impl BuildNode {
    fn new() -> Self {
        Self {
            even_children: true,
            sym: None,
            left: None,
            right: None,
        }
    }

    fn insert_rec(&mut self, payload: u32, depth: u8) -> bool {
        if self.sym.is_some() {
            return false;
        }
        if depth == 0 {
            if self.left.is_some() || self.right.is_some() {
                return false;
            }
            self.sym = Some(payload);
            return true;
        }

        if self.even_children {
            if self.left.is_some() {
                return false;
            }
            let mut new_node = BuildNode::new();
            let success = new_node.insert_rec(payload, depth - 1);
            self.left = Some(Box::new(new_node));
            self.even_children = false;
            return success;
        }

        let left = self
            .left
            .as_mut()
            .expect("Vorbis Huffman 构建内部状态非法: left 缺失");
        if !left.even_children && left.insert_rec(payload, depth - 1) {
            let right_even = self
                .right
                .as_ref()
                .map(|r| r.even_children)
                .unwrap_or(false);
            self.even_children = left.even_children && right_even;
            return true;
        }

        match self.right.as_mut() {
            Some(right) => {
                let success = right.insert_rec(payload, depth - 1);
                self.even_children = left.even_children && right.even_children;
                success
            }
            None => {
                let mut new_node = BuildNode::new();
                let success = new_node.insert_rec(payload, depth - 1);
                self.even_children = left.even_children && new_node.even_children;
                self.right = Some(Box::new(new_node));
                success
            }
        }
    }
}

fn flatten_build_tree(root: &BuildNode, out: &mut Vec<HuffNode>) -> usize {
    let idx = out.len();
    out.push(HuffNode::new());
    out[idx].sym = root.sym;
    if let Some(left) = &root.left {
        let left_idx = flatten_build_tree(left, out);
        out[idx].left = Some(left_idx);
    }
    if let Some(right) = &root.right {
        let right_idx = flatten_build_tree(right, out);
        out[idx].right = Some(right_idx);
    }
    idx
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_huffman_构建与解码() {
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
    fn test_huffman_官方示例映射() {
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
