use tao_core::{ChannelLayout, TaoError, TaoResult};

#[derive(Debug, Clone)]
pub(crate) struct VorbisHeaders {
    pub(crate) channels: u8,
    pub(crate) blocksize0: u16,
    pub(crate) blocksize1: u16,
}

pub(crate) fn parse_identification_header(
    packet: &[u8],
) -> TaoResult<(VorbisHeaders, u32, ChannelLayout)> {
    if packet.len() < 30 {
        return Err(TaoError::InvalidData(format!(
            "Vorbis identification 头包长度不足: {}",
            packet.len(),
        )));
    }
    if packet[0] != 0x01 || &packet[1..7] != b"vorbis" {
        return Err(TaoError::InvalidData(
            "Vorbis identification 头包标识无效".into(),
        ));
    }

    let version = u32::from_le_bytes([packet[7], packet[8], packet[9], packet[10]]);
    if version != 0 {
        return Err(TaoError::InvalidData(format!(
            "Vorbis 版本不支持: {}",
            version,
        )));
    }

    let channels = packet[11];
    if channels == 0 {
        return Err(TaoError::InvalidData("Vorbis 声道数不能为 0".into()));
    }

    let sample_rate = u32::from_le_bytes([packet[12], packet[13], packet[14], packet[15]]);
    if sample_rate == 0 {
        return Err(TaoError::InvalidData("Vorbis 采样率不能为 0".into()));
    }

    let bs = packet[28];
    let bs0_exp = bs & 0x0F;
    let bs1_exp = bs >> 4;
    let blocksize0 = 1u16 << bs0_exp;
    let blocksize1 = 1u16 << bs1_exp;
    if bs0_exp < 6 || bs1_exp < bs0_exp {
        return Err(TaoError::InvalidData(format!(
            "Vorbis blocksize 非法: bs0_exp={}, bs1_exp={}",
            bs0_exp, bs1_exp,
        )));
    }

    if packet[29] & 0x01 == 0 {
        return Err(TaoError::InvalidData(
            "Vorbis identification 头包 framing_flag 非法".into(),
        ));
    }

    Ok((
        VorbisHeaders {
            channels,
            blocksize0,
            blocksize1,
        },
        sample_rate,
        ChannelLayout::from_channels(u32::from(channels)),
    ))
}

pub(crate) fn parse_comment_header(packet: &[u8]) -> TaoResult<()> {
    if packet.len() < 8 {
        return Err(TaoError::InvalidData("Vorbis comment 头包长度不足".into()));
    }
    if packet[0] != 0x03 || &packet[1..7] != b"vorbis" {
        return Err(TaoError::InvalidData("Vorbis comment 头包标识无效".into()));
    }

    let mut pos = 7usize;
    let vendor_len = read_le_u32(packet, &mut pos)? as usize;
    ensure_left(packet, pos, vendor_len, "Vorbis vendor 字段")?;
    pos += vendor_len;

    let comment_count = read_le_u32(packet, &mut pos)? as usize;
    for _ in 0..comment_count {
        let comment_len = read_le_u32(packet, &mut pos)? as usize;
        ensure_left(packet, pos, comment_len, "Vorbis comment 项")?;
        pos += comment_len;
    }

    ensure_left(packet, pos, 1, "Vorbis comment framing_flag")?;
    if packet[pos] & 0x01 == 0 {
        return Err(TaoError::InvalidData(
            "Vorbis comment 头包 framing_flag 非法".into(),
        ));
    }

    Ok(())
}

fn ensure_left(data: &[u8], pos: usize, need: usize, what: &str) -> TaoResult<()> {
    if pos.saturating_add(need) > data.len() {
        return Err(TaoError::InvalidData(format!(
            "{} 超读取越界: pos={}, need={}, len={}",
            what,
            pos,
            need,
            data.len(),
        )));
    }
    Ok(())
}

fn read_le_u32(data: &[u8], pos: &mut usize) -> TaoResult<u32> {
    ensure_left(data, *pos, 4, "Vorbis u32")?;
    let v = u32::from_le_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
    *pos += 4;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_ident_header() -> Vec<u8> {
        let mut v = Vec::new();
        v.push(0x01);
        v.extend_from_slice(b"vorbis");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.push(2);
        v.extend_from_slice(&44100u32.to_le_bytes());
        v.extend_from_slice(&0i32.to_le_bytes());
        v.extend_from_slice(&128000i32.to_le_bytes());
        v.extend_from_slice(&0i32.to_le_bytes());
        v.push((11 << 4) | 8);
        v.push(1);
        v
    }

    fn build_comment_header() -> Vec<u8> {
        let mut v = Vec::new();
        v.push(0x03);
        v.extend_from_slice(b"vorbis");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.push(1);
        v
    }

    #[test]
    fn test_parse_identification_success() {
        let (h, sample_rate, layout) = parse_identification_header(&build_ident_header()).unwrap();
        assert_eq!(sample_rate, 44100);
        assert_eq!(layout.channels, 2);
        assert_eq!(h.channels, 2);
        assert_eq!(h.blocksize0, 256);
        assert_eq!(h.blocksize1, 2048);
    }

    #[test]
    fn test_parse_comment_success() {
        parse_comment_header(&build_comment_header()).unwrap();
    }
}
