import sys
import subprocess
import os

c_file = "libavcodec/h264_cabac.c"
with open(c_file, "r") as f:
    lines = f.readlines()

new_lines = []
for i, line in enumerate(lines):
    new_lines.append(line)
    if "skip = decode_cabac_mb_skip(h, sl, sl->mb_x, sl->mb_y );" in line:
        new_lines.append("""        if (h->poc.frame_num == 1 && sl->mb_xy >= 0 && sl->mb_xy <= 5) {
            fprintf(stderr, "[FFMPEG_TRACE] mb_xy=%d skip=%d\\n", sl->mb_xy, skip);
        }
""")
    if "mb_type         = ff_h264_p_mb_type_info[mb_type].type;" in line and "partition_count" in lines[i-1]:
        new_lines.append("""        if (h->poc.frame_num == 1 && sl->mb_xy >= 0 && sl->mb_xy <= 5) {
            fprintf(stderr, "[FFMPEG_TRACE] mb_xy=%d p_mb_type=%d\\n", sl->mb_xy, mb_type);
        }
""")
    if "mb_type = decode_cabac_intra_mb_type(sl, 17, 0);" in line:
        new_lines.append("""        if (h->poc.frame_num == 1 && sl->mb_xy >= 0 && sl->mb_xy <= 5) {
            fprintf(stderr, "[FFMPEG_TRACE] mb_xy=%d intra_mb_type=%d\\n", sl->mb_xy, mb_type);
        }
""")
    if "cbp  = decode_cabac_mb_cbp_luma(sl);" in line:
        new_lines.append("""        if (h->poc.frame_num == 1 && sl->mb_xy >= 0 && sl->mb_xy <= 5) {
            int trace_cbp = cbp;
            if(decode_chroma) trace_cbp |= decode_cabac_mb_cbp_chroma(sl) << 4;
            fprintf(stderr, "[FFMPEG_TRACE] mb_xy=%d cbp=%d\\n", sl->mb_xy, trace_cbp);
        }
""")
    if "int ff_h264_decode_mb_cabac(const H264Context *h, H264SliceContext *sl" in line:
        new_lines.insert(len(new_lines)-1, "int g_trace_cabac_bins = 0;\nuint8_t *g_cabac_state_base = NULL;\n")
    if "if (sl->slice_type_nos != AV_PICTURE_TYPE_I) {" in line:
        new_lines.insert(len(new_lines)-1, """    if (h->poc.frame_num == 1 && sl->mb_xy == 1) {
        g_trace_cabac_bins = 100;
        g_cabac_state_base = sl->cabac_state;
    }
""")
        
with open("libavcodec/h264_cabac.c", "w") as f:
    f.writelines(new_lines)


c_file2 = "libavcodec/cabac_functions.h"
with open(c_file2, "r") as f:
    lines = f.readlines()
new_lines2 = []
for line in lines:
    if "#ifndef get_cabac_inline" in line and "refill2" in "".join(lines[lines.index(line):lines.index(line)+3]):
        new_lines2.append("#include <stdio.h>\n")
        new_lines2.append("#undef get_cabac_inline\n")
    if "static av_always_inline int get_cabac_inline(CABACContext *c, uint8_t * const state){" in line:
        new_lines2.insert(len(new_lines2), "extern int g_trace_cabac_bins;\nextern uint8_t *g_cabac_state_base;\n")
    new_lines2.append(line)
    if "return bit;" in line:
        new_lines2.insert(len(new_lines2)-1, """
    if (g_trace_cabac_bins > 0) {
        int ctx_idx = state - g_cabac_state_base;
        fprintf(stderr, "[FFMPEG_CABAC_BIN] ctx=%d state=%d res=%d\\n", ctx_idx, *state, bit);
        g_trace_cabac_bins--;
    }
""")
with open("libavcodec/cabac_functions.h", "w") as f:
    f.writelines(new_lines2)

print("Patch applied to libavcodec/cabac_functions.h")

print("Patch applied to h264_cabac.c")
