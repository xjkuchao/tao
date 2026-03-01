use super::*;

// ============================================================
// avcC 配置解析
// ============================================================

impl H264Decoder {
    pub(super) fn parse_sps_pps_from_config(
        &mut self,
        config: &crate::parsers::h264::AvccConfig,
    ) -> TaoResult<()> {
        let mut seen_valid_sps = false;
        for (idx, sps_data) in config.sps_list.iter().enumerate() {
            match NalUnit::parse(sps_data) {
                Ok(nalu) => {
                    if nalu.nal_type != NalUnitType::Sps {
                        warn!(
                            "H264: avcC SPS 条目类型异常, index={}, nal_type={:?}",
                            idx, nalu.nal_type
                        );
                        continue;
                    }
                    if let Err(err) = parse_sps(&nalu.rbsp()) {
                        warn!("H264: avcC SPS 解析失败, index={}, err={}", idx, err);
                    } else {
                        seen_valid_sps = true;
                    }
                    self.handle_sps(&nalu);
                }
                Err(err) => {
                    warn!("H264: avcC SPS NAL 解析失败, index={}, err={}", idx, err);
                }
            }
        }
        if seen_valid_sps && self.sps.is_none() {
            return Err(TaoError::NotImplemented(
                "H264: avcC 中未找到受支持的 SPS".into(),
            ));
        }
        for (idx, pps_data) in config.pps_list.iter().enumerate() {
            match NalUnit::parse(pps_data) {
                Ok(nalu) => {
                    if nalu.nal_type != NalUnitType::Pps {
                        warn!(
                            "H264: avcC PPS 条目类型异常, index={}, nal_type={:?}",
                            idx, nalu.nal_type
                        );
                        continue;
                    }
                    self.handle_pps(&nalu);
                }
                Err(err) => {
                    warn!("H264: avcC PPS NAL 解析失败, index={}, err={}", idx, err);
                }
            }
        }
        Ok(())
    }
}
