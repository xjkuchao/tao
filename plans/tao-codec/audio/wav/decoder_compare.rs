use std::env;

#[test]
#[ignore]
fn test_wav_compare() {
    let input_url =
        env::var("TAO_WAV_COMPARE_INPUT").expect("缺少对比输入参数 TAO_WAV_COMPARE_INPUT");
    println!("测试输入: {}", input_url);
    // 此处预留调用 FFT 对比逻辑的框架
    println!("Tao对比样本=0, Tao=0, FFmpeg=0, Tao/FFmpeg: max_err=0.00, psnr=0.00dB, 精度=0.00%");
}
