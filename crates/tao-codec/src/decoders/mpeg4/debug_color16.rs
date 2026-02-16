//! Debug utilities for color16.avi decoding analysis

#[cfg(test)]
mod tests {
    use crate::codec_parameters::CodecParameters;
    use crate::decoder::Decoder;
    use crate::packet::Packet;

    #[test]
    #[ignore]
    fn debug_color16_avi_decoding() {
        // 目标: 测试 color16.avi 从第2帧开始的解码
        // 以查看何时/为何 CBPY 开始失败

        println!("色调16 AVI 调试");
        println!("已禁用此测试 - 需要网络处理");

        // 这个测试的目的是:
        // 1. 解码前10帧(应正常工作)
        // 2. 收集第2帧VOP头信息
        // 3. 分析为什么第2帧的MB开头的CBPY失败
        // 4. 比对FFmpeg的解析
    }
}
