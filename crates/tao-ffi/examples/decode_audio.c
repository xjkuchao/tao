/**
 * decode_audio.c - Tao C API 使用示例
 *
 * 演示如何使用 Tao 的 C FFI 接口打开媒体文件、读取数据包、解码音频.
 *
 * 编译 (假设 tao_ffi 动态库已构建):
 *   Windows: cl decode_audio.c /I.. /link tao_ffi.dll.lib
 *   Linux:   gcc decode_audio.c -L../../target/release -ltao_ffi -o decode_audio
 *   macOS:   gcc decode_audio.c -L../../target/release -ltao_ffi -o decode_audio
 *
 * 用法:
 *   decode_audio <输入文件>
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>

/* ========================================
 * Tao C API 声明 (手动声明, 或使用 cbindgen 生成的 tao.h)
 * ======================================== */

/* 错误码 */
#define TAO_OK              0
#define TAO_ERROR          -1
#define TAO_EOF            -2
#define TAO_NEED_MORE_DATA -3

/* 媒体类型 */
#define TAO_MEDIA_TYPE_AUDIO 1
#define TAO_MEDIA_TYPE_VIDEO 2

/* 不透明指针类型 */
typedef struct TaoFormatContext TaoFormatContext;
typedef struct TaoCodecContext TaoCodecContext;
typedef struct TaoPacket TaoPacket;
typedef struct TaoFrame TaoFrame;

/* 版本信息 */
extern const char* tao_version(void);
extern uint32_t tao_version_int(void);
extern const char* tao_build_info(void);

/* 初始化 */
extern void tao_init(void);
extern void tao_shutdown(void);

/* 格式 (解封装) */
extern TaoFormatContext* tao_format_open_input(const char* filename);
extern int tao_format_read_packet(TaoFormatContext* ctx, TaoPacket** packet);
extern int tao_format_get_stream_count(const TaoFormatContext* ctx);
extern int tao_format_get_stream_codec_id(const TaoFormatContext* ctx, int stream_index);
extern int tao_format_get_stream_media_type(const TaoFormatContext* ctx, int stream_index);
extern void tao_format_close(TaoFormatContext* ctx);

/* 编解码器 */
extern TaoCodecContext* tao_codec_create_decoder(int codec_id);
extern int tao_codec_open_decoder(TaoCodecContext* ctx, int sample_rate, int channels,
                                   const uint8_t* extra_data, int extra_data_size);
extern int tao_codec_send_packet(TaoCodecContext* ctx, const TaoPacket* packet);
extern int tao_codec_receive_frame(TaoCodecContext* ctx, TaoFrame** frame);
extern void tao_codec_close(TaoCodecContext* ctx);

/* 数据包 */
extern const uint8_t* tao_packet_data(const TaoPacket* pkt);
extern int tao_packet_size(const TaoPacket* pkt);
extern int64_t tao_packet_pts(const TaoPacket* pkt);
extern int tao_packet_stream_index(const TaoPacket* pkt);
extern void tao_packet_free(TaoPacket* pkt);

/* 帧 */
extern int tao_frame_is_audio(const TaoFrame* frame);
extern int tao_frame_is_video(const TaoFrame* frame);
extern int tao_frame_nb_samples(const TaoFrame* frame);
extern int tao_frame_sample_rate(const TaoFrame* frame);
extern int tao_frame_width(const TaoFrame* frame);
extern int tao_frame_height(const TaoFrame* frame);
extern const uint8_t* tao_frame_data(const TaoFrame* frame, int plane);
extern int tao_frame_linesize(const TaoFrame* frame, int plane);
extern void tao_frame_free(TaoFrame* frame);

/* ======================================== */

int main(int argc, char* argv[]) {
    if (argc < 2) {
        printf("用法: %s <输入文件>\n", argv[0]);
        printf("\n");
        printf("示例: %s input.wav\n", argv[0]);
        return 1;
    }

    const char* input_file = argv[1];

    /* 初始化 */
    tao_init();
    printf("Tao 版本: %s\n", tao_version());
    printf("构建信息: %s\n", tao_build_info());
    printf("\n");

    /* 打开输入文件 */
    printf("打开文件: %s\n", input_file);
    TaoFormatContext* fmt_ctx = tao_format_open_input(input_file);
    if (!fmt_ctx) {
        fprintf(stderr, "错误: 无法打开输入文件\n");
        tao_shutdown();
        return 1;
    }

    /* 获取流信息 */
    int stream_count = tao_format_get_stream_count(fmt_ctx);
    printf("流数量: %d\n", stream_count);

    /* 查找第一个音频流 */
    int audio_stream = -1;
    int audio_codec_id = -1;
    for (int i = 0; i < stream_count; i++) {
        int media_type = tao_format_get_stream_media_type(fmt_ctx, i);
        int codec_id = tao_format_get_stream_codec_id(fmt_ctx, i);
        printf("  流 #%d: 媒体类型=%d, 编解码器ID=%d\n", i, media_type, codec_id);

        if (media_type == TAO_MEDIA_TYPE_AUDIO && audio_stream < 0) {
            audio_stream = i;
            audio_codec_id = codec_id;
        }
    }

    if (audio_stream < 0) {
        fprintf(stderr, "错误: 未找到音频流\n");
        tao_format_close(fmt_ctx);
        tao_shutdown();
        return 1;
    }

    printf("使用音频流 #%d (编解码器ID=%d)\n\n", audio_stream, audio_codec_id);

    /* 创建解码器 */
    TaoCodecContext* dec_ctx = tao_codec_create_decoder(audio_codec_id);
    if (!dec_ctx) {
        fprintf(stderr, "错误: 无法创建解码器\n");
        tao_format_close(fmt_ctx);
        tao_shutdown();
        return 1;
    }

    /* 打开解码器 (使用默认参数) */
    int ret = tao_codec_open_decoder(dec_ctx, 44100, 2, NULL, 0);
    if (ret != TAO_OK) {
        fprintf(stderr, "错误: 无法打开解码器\n");
        tao_codec_close(dec_ctx);
        tao_format_close(fmt_ctx);
        tao_shutdown();
        return 1;
    }

    /* 解码循环 */
    int packet_count = 0;
    int frame_count = 0;
    int64_t total_samples = 0;

    while (1) {
        TaoPacket* pkt = NULL;
        ret = tao_format_read_packet(fmt_ctx, &pkt);

        if (ret == TAO_EOF) {
            printf("到达文件末尾\n");
            break;
        }
        if (ret != TAO_OK || !pkt) {
            fprintf(stderr, "读取数据包错误: %d\n", ret);
            break;
        }

        /* 只处理目标音频流 */
        if (tao_packet_stream_index(pkt) != audio_stream) {
            tao_packet_free(pkt);
            continue;
        }

        packet_count++;

        /* 发送数据包到解码器 */
        ret = tao_codec_send_packet(dec_ctx, pkt);
        tao_packet_free(pkt);

        if (ret != TAO_OK) {
            continue;
        }

        /* 接收解码帧 */
        while (1) {
            TaoFrame* frame = NULL;
            ret = tao_codec_receive_frame(dec_ctx, &frame);

            if (ret == TAO_NEED_MORE_DATA || ret == TAO_EOF) {
                break;
            }
            if (ret != TAO_OK || !frame) {
                break;
            }

            if (tao_frame_is_audio(frame)) {
                int nb_samples = tao_frame_nb_samples(frame);
                int sample_rate = tao_frame_sample_rate(frame);
                total_samples += nb_samples;
                frame_count++;

                /* 打印前几帧的信息 */
                if (frame_count <= 5) {
                    printf("  帧 #%d: %d 采样 @ %d Hz (PTS: -)\n",
                           frame_count, nb_samples, sample_rate);
                }
            }

            tao_frame_free(frame);
        }
    }

    /* 统计 */
    printf("\n解码完成:\n");
    printf("  数据包: %d\n", packet_count);
    printf("  帧: %d\n", frame_count);
    printf("  总采样: %lld\n", (long long)total_samples);

    /* 清理 */
    tao_codec_close(dec_ctx);
    tao_format_close(fmt_ctx);
    tao_shutdown();

    return 0;
}
