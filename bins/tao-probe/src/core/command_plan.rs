//! 命令归一化模型.
//!
//! 将原始参数解析结果统一成可执行的 `CommandPlan`.

use crate::cli::parser::{CliError, OptionValueSource, ParsedArgs, ParsedOption};
use crate::compat::unimplemented;

/// 全局命令.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalCommand {
    Help(Option<String>),
    Version,
    BuildConf,
    License,
    Formats,
    Muxers,
    Demuxers,
    Devices,
    Codecs,
    Decoders,
    Encoders,
    Bsfs,
    Protocols,
    Filters,
    PixFmts,
    Layouts,
    SampleFmts,
    Dispositions,
    Colors,
    Sections,
}

/// 展示修饰链.
#[derive(Debug, Clone, Default)]
pub struct DisplayModifiers {
    pub unit: bool,
    pub prefix: bool,
    pub byte_binary_prefix: bool,
    pub sexagesimal: bool,
    pub pretty: bool,
    pub show_optional_fields: Option<String>,
    pub show_optional_fields_state: OptionalValueState,
    pub show_private_data: bool,
    pub show_private_data_state: OptionalValueState,
}

/// 探测输出开关.
#[derive(Debug, Clone, Default)]
pub struct ShowSwitches {
    pub show_format: bool,
    pub show_streams: bool,
    pub show_packets: bool,
    pub show_frames: bool,
    pub show_programs: bool,
    pub show_stream_groups: bool,
    pub show_chapters: bool,
    pub show_error: bool,
    pub show_log: bool,
    pub show_data: bool,
    pub show_data_hash: Option<String>,
    pub show_data_hash_state: OptionalValueState,
    pub count_frames: bool,
    pub count_packets: bool,
    pub show_program_version: bool,
    pub show_library_versions: bool,
    pub show_versions: bool,
    pub show_pixel_formats: bool,
}

/// 可选参数值状态（三态）.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum OptionalValueState {
    /// 参数未出现.
    #[default]
    Absent,
    /// 参数出现但未显式给值.
    PresentImplicit,
    /// 参数出现且显式给值.
    PresentExplicit(String),
}

/// 有序执行项（用于 passthrough 保序执行）.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedExecutionItem {
    pub token: String,
}

/// 统一执行计划.
#[derive(Debug, Clone, Default)]
pub struct CommandPlan {
    pub invocation_name: String,
    pub hide_banner: bool,
    pub loglevel: Option<String>,
    pub global_command: Option<GlobalCommand>,

    pub input: Option<String>,
    pub force_format: Option<String>,
    pub output_path: Option<String>,
    pub output_format: Option<String>,
    pub print_filename: Option<String>,
    pub select_streams: Option<String>,
    pub show_entries: Option<String>,
    pub read_intervals: Option<String>,
    pub find_stream_info: bool,

    pub show: ShowSwitches,
    pub display: DisplayModifiers,

    pub avoptions: Vec<(String, String)>,
    pub ordered_execution: Vec<OrderedExecutionItem>,
    pub unimplemented_hits: Vec<&'static unimplemented::UnimplementedEntry>,
}

/// 构建执行计划.
pub fn build_command_plan(parsed: &ParsedArgs) -> Result<CommandPlan, CliError> {
    let mut plan = CommandPlan {
        invocation_name: parsed.invocation_name.clone(),
        ..CommandPlan::default()
    };

    plan.hide_banner = parse_optional_bool(parsed.last_value("hide_banner")).unwrap_or(false);
    plan.loglevel = parsed.last_value("loglevel").map(ToString::to_string);
    plan.force_format = parsed.last_value("f").map(ToString::to_string);
    plan.output_path = parsed.last_value("o").map(ToString::to_string);
    plan.output_format = parsed.last_value("output_format").map(ToString::to_string);
    plan.print_filename = parsed.last_value("print_filename").map(ToString::to_string);
    plan.select_streams = parsed.last_value("select_streams").map(ToString::to_string);
    plan.show_entries = parsed.last_value("show_entries").map(ToString::to_string);
    plan.read_intervals = parsed.last_value("read_intervals").map(ToString::to_string);
    plan.find_stream_info = parsed.has("find_stream_info");

    plan.display.unit = parsed.has("unit");
    plan.display.prefix = parsed.has("prefix");
    plan.display.byte_binary_prefix = parsed.has("byte_binary_prefix");
    plan.display.sexagesimal = parsed.has("sexagesimal");
    plan.display.pretty = parsed.has("pretty");
    plan.display.show_optional_fields_state =
        optional_value_state(parsed.last_option("show_optional_fields"));
    plan.display.show_optional_fields = parsed
        .last_value("show_optional_fields")
        .map(ToString::to_string);
    plan.display.show_private_data_state =
        optional_value_state(parsed.last_option("show_private_data"));
    plan.display.show_private_data = match &plan.display.show_private_data_state {
        OptionalValueState::Absent => false,
        OptionalValueState::PresentImplicit => true,
        OptionalValueState::PresentExplicit(value) => {
            parse_optional_bool(Some(value.as_str())).unwrap_or(true)
        }
    };

    if plan.display.pretty {
        plan.display.unit = true;
        plan.display.prefix = true;
        plan.display.byte_binary_prefix = true;
        plan.display.sexagesimal = true;
    }

    plan.show.show_format = parsed.has("show_format");
    plan.show.show_streams = parsed.has("show_streams");
    plan.show.show_packets = parsed.has("show_packets");
    plan.show.show_frames = parsed.has("show_frames");
    plan.show.show_programs = parsed.has("show_programs");
    plan.show.show_stream_groups = parsed.has("show_stream_groups");
    plan.show.show_chapters = parsed.has("show_chapters");
    plan.show.show_error = parsed.has("show_error");
    plan.show.show_log = parsed.has("show_log");
    plan.show.show_data = parsed.has("show_data");
    plan.show.show_data_hash_state = optional_value_state(parsed.last_option("show_data_hash"));
    plan.show.show_data_hash = parsed
        .last_value("show_data_hash")
        .map(ToString::to_string)
        .or_else(|| {
            matches!(
                plan.show.show_data_hash_state,
                OptionalValueState::PresentImplicit
            )
            .then(|| "md5".to_string())
        });
    plan.show.count_frames = parsed.has("count_frames");
    plan.show.count_packets = parsed.has("count_packets");
    plan.show.show_program_version = parsed.has("show_program_version");
    plan.show.show_library_versions = parsed.has("show_library_versions");
    plan.show.show_versions = parsed.has("show_versions");
    plan.show.show_pixel_formats = parsed.has("show_pixel_formats");

    for option in &parsed.options {
        if plan.global_command.is_none() {
            plan.global_command = map_global_command(&option.canonical, option.value.clone());
        }

        if option.is_avoption {
            if let Some(value) = option.value.as_deref() {
                plan.avoptions
                    .push((option.canonical.clone(), value.to_string()));
            }
        }

        if let Some(hit) = unimplemented::find_entry(&option.canonical) {
            plan.unimplemented_hits.push(hit);
        }
    }

    plan.ordered_execution = parsed
        .passthrough_tokens
        .iter()
        .cloned()
        .map(|token| OrderedExecutionItem { token })
        .collect();

    let mut input_candidates: Vec<String> =
        parsed.values("i").iter().map(|s| s.to_string()).collect();
    input_candidates.extend(parsed.positionals.iter().cloned());

    if input_candidates.len() > 1 {
        return Err(CliError {
            message: format!(
                "Argument '{}' provided as input filename, but '{}' was already specified.",
                input_candidates[1], input_candidates[0]
            ),
            hide_banner: plan.hide_banner,
        });
    }

    plan.input = input_candidates.into_iter().next();

    Ok(plan)
}

fn map_global_command(canonical: &str, value: Option<String>) -> Option<GlobalCommand> {
    match canonical {
        "L" => Some(GlobalCommand::License),
        "help" => Some(GlobalCommand::Help(value)),
        "version" => Some(GlobalCommand::Version),
        "buildconf" => Some(GlobalCommand::BuildConf),
        "formats" => Some(GlobalCommand::Formats),
        "muxers" => Some(GlobalCommand::Muxers),
        "demuxers" => Some(GlobalCommand::Demuxers),
        "devices" => Some(GlobalCommand::Devices),
        "codecs" => Some(GlobalCommand::Codecs),
        "decoders" => Some(GlobalCommand::Decoders),
        "encoders" => Some(GlobalCommand::Encoders),
        "bsfs" => Some(GlobalCommand::Bsfs),
        "protocols" => Some(GlobalCommand::Protocols),
        "filters" => Some(GlobalCommand::Filters),
        "pix_fmts" => Some(GlobalCommand::PixFmts),
        "layouts" => Some(GlobalCommand::Layouts),
        "sample_fmts" => Some(GlobalCommand::SampleFmts),
        "dispositions" => Some(GlobalCommand::Dispositions),
        "colors" => Some(GlobalCommand::Colors),
        "sections" => Some(GlobalCommand::Sections),
        _ => None,
    }
}

fn parse_optional_bool(value: Option<&str>) -> Option<bool> {
    let raw = value?;
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "1" | "true" | "yes" | "on" | "always" => Some(true),
        "0" | "false" | "no" | "off" | "never" => Some(false),
        _ => Some(true),
    }
}

fn optional_value_state(option: Option<&ParsedOption>) -> OptionalValueState {
    let Some(option) = option else {
        return OptionalValueState::Absent;
    };

    match option.value_source {
        OptionValueSource::None => OptionalValueState::PresentImplicit,
        OptionValueSource::ImplicitDefault => OptionalValueState::PresentImplicit,
        OptionValueSource::Inline
        | OptionValueSource::Separate
        | OptionValueSource::LegacyAlias => option
            .value
            .clone()
            .map(OptionalValueState::PresentExplicit)
            .unwrap_or(OptionalValueState::PresentImplicit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::parser::parse_argv;

    fn vec_of(items: &[&str]) -> Vec<String> {
        items.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn test_build_plan_with_mixed_input_order() {
        let argv = vec_of(&["ffprobe", "-show_format", "a.mp4"]);
        let parsed = parse_argv(&argv, "ffprobe").expect("解析失败");
        let plan = build_command_plan(&parsed).expect("构建计划失败");
        assert_eq!(plan.input.as_deref(), Some("a.mp4"), "应识别位置输入");
        assert!(plan.show.show_format, "应开启 show_format");
    }

    #[test]
    fn test_build_plan_multiple_inputs_error() {
        let argv = vec_of(&["ffprobe", "a.mp4", "b.mp4"]);
        let parsed = parse_argv(&argv, "ffprobe").expect("解析失败");
        let err = build_command_plan(&parsed).expect_err("应因多输入失败");
        assert!(
            err.message
                .contains("Argument 'b.mp4' provided as input filename"),
            "错误文案应包含多输入冲突"
        );
    }
}
