use tao_core::{TaoError, TaoResult};

#[derive(Clone)]
pub(crate) struct LsbBitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> LsbBitReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    fn bits_left(&self) -> usize {
        self.data
            .len()
            .saturating_mul(8)
            .saturating_sub(self.bit_pos)
    }

    pub(crate) fn read_flag(&mut self) -> TaoResult<bool> {
        Ok(self.read_bits(1)? != 0)
    }

    pub(crate) fn read_bits(&mut self, n: u8) -> TaoResult<u32> {
        if n == 0 {
            return Ok(0);
        }
        if n > 32 {
            return Err(TaoError::InvalidArgument(format!(
                "Vorbis read_bits 位数非法: {}",
                n,
            )));
        }
        if self.bits_left() < n as usize {
            return Err(TaoError::Eof);
        }

        let mut out = 0u32;
        for i in 0..n {
            let bit_idx = self.bit_pos + i as usize;
            let byte = self.data[bit_idx / 8];
            let bit = (byte >> (bit_idx % 8)) & 1;
            out |= u32::from(bit) << i;
        }
        self.bit_pos += n as usize;
        Ok(out)
    }

    pub(crate) fn read_bits_u64(&mut self, n: u8) -> TaoResult<u64> {
        if n == 0 {
            return Ok(0);
        }
        if n > 64 {
            return Err(TaoError::InvalidArgument(format!(
                "Vorbis read_bits_u64 位数非法: {}",
                n,
            )));
        }
        if self.bits_left() < n as usize {
            return Err(TaoError::Eof);
        }

        let mut out = 0u64;
        for i in 0..n {
            let bit_idx = self.bit_pos + i as usize;
            let byte = self.data[bit_idx / 8];
            let bit = (byte >> (bit_idx % 8)) & 1;
            out |= u64::from(bit) << i;
        }
        self.bit_pos += n as usize;
        Ok(out)
    }

    pub(crate) fn bit_position(&self) -> usize {
        self.bit_pos
    }

    pub(crate) fn read_bits_at(&self, bit_pos: usize, n: u8) -> TaoResult<u32> {
        if n == 0 {
            return Ok(0);
        }
        if n > 32 {
            return Err(TaoError::InvalidArgument(format!(
                "Vorbis read_bits_at 位数非法: {}",
                n,
            )));
        }
        let total_bits = self.data.len().saturating_mul(8);
        if bit_pos.saturating_add(n as usize) > total_bits {
            return Err(TaoError::Eof);
        }

        let mut out = 0u32;
        for i in 0..n {
            let idx = bit_pos + i as usize;
            let byte = self.data[idx / 8];
            let bit = (byte >> (idx % 8)) & 1;
            out |= u32::from(bit) << i;
        }
        Ok(out)
    }
}

pub(crate) fn ilog(v: u32) -> u8 {
    if v == 0 {
        return 0;
    }
    (32 - v.leading_zeros()) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsb_bit_reader_bit_order() {
        let data = [0b1011_0010];
        let mut br = LsbBitReader::new(&data);
        assert_eq!(br.read_bits(1).unwrap(), 0);
        assert_eq!(br.read_bits(3).unwrap(), 0b001);
        assert_eq!(br.read_bits(4).unwrap(), 0b1011);
    }
}
