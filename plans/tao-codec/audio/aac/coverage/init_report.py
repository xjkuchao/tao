import urllib.request
import re

url = "https://samples.ffmpeg.org/allsamples.txt"
req = urllib.request.urlopen(url)
data = req.read().decode('utf-8')

# look for .aac, .m4a and /AAC/.*\.mp4
samples = []
for line in data.splitlines():
    line = line.strip()
    if not line: continue
    if line.startswith('.'):
        line = line[1:]
    
    line_lower = line.lower()
    
    if line_lower.endswith('.aac') or line_lower.endswith('.m4a') or ('/aac/' in line_lower and (line_lower.endswith('.mp4') or line_lower.endswith('.mkv') or line_lower.endswith('.mov') or line_lower.endswith('.flv'))):
        full_url = f"https://samples.ffmpeg.org{line}"
        samples.append(full_url)

# Remove duplicates
samples = sorted(list(set(samples)))

with open('plans/tao-codec/audio/aac/coverage/report.md', 'w', encoding='utf-8') as f:
    f.write("# AAC 解码器覆盖率测试报告\n\n")
    f.write("| 序号 | URL | 状态 | 失败原因 | Tao样本数 | FFmpeg样本数 | 样本数差异 | max_err | psnr(dB) | 精度(%) | 备注 |\n")
    f.write("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n")
    for i, s in enumerate(samples, 1):
        f.write(f"| {i} | {s} |  |  |  |  |  |  |  |  |  |\n")
