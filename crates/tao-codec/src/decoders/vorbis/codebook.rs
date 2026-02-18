use std::collections::HashMap;

use tao_core::{TaoError, TaoResult};

use super::bitreader::LsbBitReader;

#[derive(Debug, Clone)]
pub(crate) struct CodebookHuffman {
    table: HashMap<(u8, u32), u32>,
    max_len: u8,
}

impl CodebookHuffman {
    pub(crate) fn from_lengths(lengths: &[u8]) -> TaoResult<Self> {
        let mut entries: Vec<(u32, u8)> = lengths
            .iter()
            .enumerate()
            .filter_map(|(sym, &len)| (len > 0).then_some((sym as u32, len)))
            .collect();
        entries.sort_by_key(|&(sym, len)| (len, sym));

        let max_len = entries.iter().map(|(_, len)| *len).max().unwrap_or(0);
        let mut table = HashMap::new();
        let mut code = 0u32;
        let mut cur_len = 1u8;

        for (sym, len) in entries {
            while cur_len < len {
                code <<= 1;
                cur_len += 1;
            }
            let rev = reverse_bits(code, len);
            if table.insert((len, rev), sym).is_some() {
                return Err(TaoError::InvalidData(
                    "Vorbis codebook Huffman 码冲突".into(),
                ));
            }
            code = code.saturating_add(1);
        }

        Ok(Self { table, max_len })
    }

    pub(crate) fn decode_symbol(&self, br: &mut LsbBitReader<'_>) -> TaoResult<u32> {
        let mut code = 0u32;
        for len in 1..=self.max_len {
            let bit = br.read_bits(1)?;
            code |= bit << (len - 1);
            if let Some(&sym) = self.table.get(&(len, code)) {
                return Ok(sym);
            }
        }
        Err(TaoError::InvalidData(
            "Vorbis codebook Huffman 解码失败".into(),
        ))
    }
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
        // LSB 读取时写入反转位序。
        let data = [0b0110_0100u8]; // bits: 0 | 001 | 101 ...
        let mut br = LsbBitReader::new(&data);
        let s0 = h.decode_symbol(&mut br).expect("sym0 解码失败");
        assert_eq!(s0, 0, "第一个符号应为 sym0");
        let s1 = h.decode_symbol(&mut br).expect("sym1 解码失败");
        assert_eq!(s1, 1, "第二个符号应为 sym1");
        let s2 = h.decode_symbol(&mut br).expect("sym2 解码失败");
        assert_eq!(s2, 2, "第三个符号应为 sym2");
    }
}
