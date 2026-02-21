use super::*;

const SEI_BUFFERING_PERIOD: u32 = 0;
const SEI_PIC_TIMING: u32 = 1;
const SEI_USER_DATA_UNREGISTERED: u32 = 5;
const SEI_RECOVERY_POINT: u32 = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SeiPayload {
    pub payload_type: u32,
    pub payload_size: usize,
    pub message: SeiMessage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SeiMessage {
    BufferingPeriod(SeiBufferingPeriod),
    PicTiming(SeiPicTiming),
    UserDataUnregistered(SeiUserDataUnregistered),
    RecoveryPoint(SeiRecoveryPoint),
    Unknown { data: Vec<u8> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SeiBufferingPeriod {
    pub seq_parameter_set_id: u32,
    pub raw: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SeiPicTiming {
    pub raw: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SeiUserDataUnregistered {
    pub uuid_iso_iec_11578: [u8; 16],
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SeiRecoveryPoint {
    pub recovery_frame_cnt: u32,
    pub exact_match_flag: bool,
    pub broken_link_flag: bool,
    pub changing_slice_group_idc: u8,
}

pub(super) fn parse_sei_rbsp(rbsp: &[u8]) -> TaoResult<Vec<SeiPayload>> {
    let mut payloads = Vec::new();
    let mut offset = 0usize;

    while offset < rbsp.len() {
        if is_rbsp_trailing_bits(&rbsp[offset..]) {
            break;
        }

        let payload_type = read_sei_ff_coded_value(rbsp, &mut offset, "payload_type")?;
        let payload_size_raw = read_sei_ff_coded_value(rbsp, &mut offset, "payload_size")?;
        let payload_size = usize::try_from(payload_size_raw).map_err(|_| {
            TaoError::InvalidData(format!(
                "H264: SEI payload_size 超范围, value={payload_size_raw}"
            ))
        })?;
        let payload_end = offset.checked_add(payload_size).ok_or_else(|| {
            TaoError::InvalidData(format!("H264: SEI payload_size 溢出, value={payload_size}"))
        })?;
        if payload_end > rbsp.len() {
            return Err(TaoError::InvalidData(format!(
                "H264: SEI payload 截断, type={payload_type}, size={payload_size}, remain={}",
                rbsp.len().saturating_sub(offset)
            )));
        }
        let payload = &rbsp[offset..payload_end];
        offset = payload_end;

        payloads.push(SeiPayload {
            payload_type,
            payload_size,
            message: parse_sei_payload(payload_type, payload)?,
        });
    }

    Ok(payloads)
}

fn is_rbsp_trailing_bits(rest: &[u8]) -> bool {
    if rest.is_empty() {
        return true;
    }
    rest[0] == 0x80 && rest[1..].iter().all(|v| *v == 0)
}

fn read_sei_ff_coded_value(data: &[u8], offset: &mut usize, name: &str) -> TaoResult<u32> {
    let mut value = 0u32;
    loop {
        let byte = *data
            .get(*offset)
            .ok_or_else(|| TaoError::InvalidData(format!("H264: SEI {name} 截断")))?;
        *offset += 1;
        value = value
            .checked_add(u32::from(byte))
            .ok_or_else(|| TaoError::InvalidData(format!("H264: SEI {name} 溢出")))?;
        if byte != 0xFF {
            break;
        }
    }
    Ok(value)
}

fn parse_sei_payload(payload_type: u32, payload: &[u8]) -> TaoResult<SeiMessage> {
    match payload_type {
        SEI_BUFFERING_PERIOD => Ok(SeiMessage::BufferingPeriod(parse_buffering_period(
            payload,
        )?)),
        SEI_PIC_TIMING => Ok(SeiMessage::PicTiming(SeiPicTiming {
            raw: payload.to_vec(),
        })),
        SEI_USER_DATA_UNREGISTERED => Ok(SeiMessage::UserDataUnregistered(
            parse_user_data_unregistered(payload)?,
        )),
        SEI_RECOVERY_POINT => Ok(SeiMessage::RecoveryPoint(parse_recovery_point(payload)?)),
        _ => Ok(SeiMessage::Unknown {
            data: payload.to_vec(),
        }),
    }
}

fn parse_buffering_period(payload: &[u8]) -> TaoResult<SeiBufferingPeriod> {
    let mut br = BitReader::new(payload);
    let seq_parameter_set_id = read_ue(&mut br)?;
    Ok(SeiBufferingPeriod {
        seq_parameter_set_id,
        raw: payload.to_vec(),
    })
}

fn parse_user_data_unregistered(payload: &[u8]) -> TaoResult<SeiUserDataUnregistered> {
    if payload.len() < 16 {
        return Err(TaoError::InvalidData(format!(
            "H264: SEI user_data_unregistered 截断, len={}",
            payload.len()
        )));
    }
    let mut uuid_iso_iec_11578 = [0u8; 16];
    uuid_iso_iec_11578.copy_from_slice(&payload[..16]);
    Ok(SeiUserDataUnregistered {
        uuid_iso_iec_11578,
        payload: payload[16..].to_vec(),
    })
}

fn parse_recovery_point(payload: &[u8]) -> TaoResult<SeiRecoveryPoint> {
    let mut br = BitReader::new(payload);
    let recovery_frame_cnt = read_ue(&mut br)?;
    let exact_match_flag = br.read_bit()? != 0;
    let broken_link_flag = br.read_bit()? != 0;
    let changing_slice_group_idc = br.read_bits(2)? as u8;
    Ok(SeiRecoveryPoint {
        recovery_frame_cnt,
        exact_match_flag,
        broken_link_flag,
        changing_slice_group_idc,
    })
}
