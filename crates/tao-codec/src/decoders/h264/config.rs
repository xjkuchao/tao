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
        for sps_data in &config.sps_list {
            if let Ok(nalu) = NalUnit::parse(sps_data) {
                if nalu.nal_type == NalUnitType::Sps && parse_sps(&nalu.rbsp()).is_ok() {
                    seen_valid_sps = true;
                }
                self.handle_sps(&nalu);
            }
        }
        if seen_valid_sps && self.sps.is_none() {
            return Err(TaoError::NotImplemented(
                "H264: avcC 中未找到受支持的 SPS".into(),
            ));
        }
        for pps_data in &config.pps_list {
            if let Ok(nalu) = NalUnit::parse(pps_data) {
                self.handle_pps(&nalu);
            }
        }
        Ok(())
    }
}
