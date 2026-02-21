use super::*;

// ============================================================
// avcC 配置解析
// ============================================================

impl H264Decoder {
    pub(super) fn parse_sps_pps_from_config(
        &mut self,
        config: &crate::parsers::h264::AvccConfig,
    ) -> TaoResult<()> {
        for sps_data in &config.sps_list {
            if let Ok(nalu) = NalUnit::parse(sps_data) {
                self.handle_sps(&nalu);
            }
        }
        for pps_data in &config.pps_list {
            if let Ok(nalu) = NalUnit::parse(pps_data) {
                self.handle_pps(&nalu);
            }
        }
        Ok(())
    }
}
