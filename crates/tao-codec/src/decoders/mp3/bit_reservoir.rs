//! MP3 比特储备库 (Bit Reservoir)
//!
//! 负责缓存 main_data 并根据 main_data_begin 复用历史字节.

use tao_core::{TaoError, TaoResult};

/// MP3 main_data 最大缓冲大小
pub const MAX_MAIN_DATA_BYTES: usize = 2048;

/// 比特储备库
#[derive(Debug, Clone)]
pub struct BitReservoir {
    buf: Vec<u8>,
    len: usize,
    consumed: usize,
}

impl BitReservoir {
    pub fn new() -> Self {
        Self {
            buf: vec![0u8; MAX_MAIN_DATA_BYTES],
            len: 0,
            consumed: 0,
        }
    }

    /// 填充 main_data, 返回 underflow 字节数.
    pub fn fill(&mut self, pkt_main_data: &[u8], main_data_begin: usize) -> TaoResult<usize> {
        let main_data_len = pkt_main_data.len();
        let main_data_end = main_data_begin + main_data_len;

        if main_data_end > self.buf.len() {
            return Err(TaoError::InvalidData(
                "MP3 main_data 超出比特储备库容量".into(),
            ));
        }

        let unread = self.len.saturating_sub(self.consumed);

        let underflow = if main_data_begin <= unread {
            let start = self.len - main_data_begin;
            self.buf.copy_within(start..self.len, 0);
            self.buf[main_data_begin..main_data_end].copy_from_slice(pkt_main_data);
            self.len = main_data_end;
            0
        } else {
            let start = self.len - unread;
            self.buf.copy_within(start..self.len, 0);
            self.buf[unread..unread + main_data_len].copy_from_slice(pkt_main_data);
            self.len = unread + main_data_len;
            main_data_begin - unread
        };

        self.consumed = 0;
        Ok(underflow)
    }

    /// 消费已读取的字节数
    pub fn consume(&mut self, len: usize) {
        self.consumed = (self.consumed + len).min(self.len);
    }

    /// 当前可读 main_data 视图
    pub fn bytes_ref(&self) -> &[u8] {
        &self.buf[self.consumed..self.len]
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.consumed = 0;
    }
}
