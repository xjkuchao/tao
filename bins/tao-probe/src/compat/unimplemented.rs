//! 未实现白名单.
//!
//! 规则:
//! - 参数接口先接入。
//! - 核心能力缺失时统一返回 `Function not implemented`。

/// 单条未实现项.
#[derive(Debug, Clone, Copy)]
pub struct UnimplementedEntry {
    /// 参数名（规范名）.
    pub option: &'static str,
    /// 缺失原因.
    pub reason: &'static str,
    /// 关联模块.
    pub module: &'static str,
    /// 清零条件.
    pub clear_condition: &'static str,
}

/// 未实现白名单.
pub const UNIMPLEMENTED_OPTIONS: &[UnimplementedEntry] = &[
    UnimplementedEntry {
        option: "devices",
        reason: "设备层枚举尚未接入",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "接入 tao 的 device 抽象并可枚举输入/输出设备",
    },
    UnimplementedEntry {
        option: "bsfs",
        reason: "比特流过滤器列表尚未接入",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "提供 bsf 注册表并导出查询接口",
    },
    UnimplementedEntry {
        option: "protocols",
        reason: "协议层能力清单未完整实现",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "统一协议能力注册并与 format/io 打通",
    },
    UnimplementedEntry {
        option: "filters",
        reason: "滤镜枚举接口尚未接入",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "tao-filter 暴露可查询的滤镜注册表",
    },
    UnimplementedEntry {
        option: "layouts",
        reason: "声道布局列表输出尚未接入",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "实现标准布局枚举输出",
    },
    UnimplementedEntry {
        option: "sample_fmts",
        reason: "采样格式完整枚举输出尚未接入",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "实现 sample format 枚举与描述输出",
    },
    UnimplementedEntry {
        option: "dispositions",
        reason: "stream disposition 枚举尚未接入",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "补齐 disposition 常量与 writer 输出",
    },
    UnimplementedEntry {
        option: "colors",
        reason: "颜色名称清单未接入",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "补齐 color name 列表输出",
    },
    UnimplementedEntry {
        option: "show_packets",
        reason: "packet 级 ffprobe 对拍字段尚未完整实现",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "补齐 packet section 字段与 writer 对拍",
    },
    UnimplementedEntry {
        option: "show_frames",
        reason: "frame 级输出尚未接入",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "接入 decoder 路径并输出 FRAME section",
    },
    UnimplementedEntry {
        option: "show_programs",
        reason: "program section 尚未接入 demuxer",
        module: "crates/tao-format/src/demuxer.rs",
        clear_condition: "demuxer 提供 programs 并完成 writer 输出",
    },
    UnimplementedEntry {
        option: "show_stream_groups",
        reason: "stream group section 尚未接入 demuxer",
        module: "crates/tao-format/src/demuxer.rs",
        clear_condition: "demuxer 提供 stream_groups 并完成 writer 输出",
    },
    UnimplementedEntry {
        option: "show_chapters",
        reason: "chapter section 尚未接入 demuxer",
        module: "crates/tao-format/src/demuxer.rs",
        clear_condition: "demuxer 提供 chapters 并完成 writer 输出",
    },
    UnimplementedEntry {
        option: "show_error",
        reason: "ERROR section 结构化输出尚未接入",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "实现 ERROR section 并与 writer 对拍",
    },
    UnimplementedEntry {
        option: "show_log",
        reason: "按流日志输出尚未接入",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "接入解码日志管线并输出 LOG section",
    },
    UnimplementedEntry {
        option: "show_data",
        reason: "packet/frame 原始数据输出尚未接入",
        module: "crates/tao-codec/src/packet.rs",
        clear_condition: "packet side-data 与十六进制转储输出完整实现",
    },
    UnimplementedEntry {
        option: "show_data_hash",
        reason: "packet/frame 数据哈希输出尚未接入",
        module: "crates/tao-codec/src/packet.rs",
        clear_condition: "接入哈希算法与数据域导出",
    },
    UnimplementedEntry {
        option: "read_intervals",
        reason: "区间读取语义尚未接入",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "实现 read_intervals 解析与 demux 读取窗口",
    },
    UnimplementedEntry {
        option: "find_stream_info",
        reason: "启发式补全流信息尚未接入",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "接入解码采样流程补全缺失信息",
    },
    UnimplementedEntry {
        option: "count_frames",
        reason: "逐流帧计数尚未接入",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "实现按流 frame 计数并对拍",
    },
    UnimplementedEntry {
        option: "sources",
        reason: "设备 source 枚举尚未接入",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "实现输入设备源查询",
    },
    UnimplementedEntry {
        option: "sinks",
        reason: "设备 sink 枚举尚未接入",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "实现输出设备汇查询",
    },
    UnimplementedEntry {
        option: "show_pixel_formats",
        reason: "像素格式描述输出尚未达到 ffprobe 对拍级",
        module: "bins/tao-probe/src/app.rs",
        clear_condition: "补齐 pixel format 字段并完成对拍",
    },
];

/// 按参数名查询未实现条目.
pub fn find_entry(option: &str) -> Option<&'static UnimplementedEntry> {
    UNIMPLEMENTED_OPTIONS
        .iter()
        .find(|entry| entry.option == option)
}
