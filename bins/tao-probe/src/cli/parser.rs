//! ffprobe 风格 argv 解析器.
//!
//! 支持:
//! - `-opt value`
//! - `-opt=value`
//! - 重复参数
//! - 输入参数前后混排
//! - 旧版 tao-probe 隐藏别名

use crate::cli::ffprobe_7_1_3_options::{
    LEGACY_ALIAS_OPTIONS, OptionValueKind, find_main_option, is_avoption_name,
};

/// 解析后的参数错误.
#[derive(Debug, Clone)]
pub struct CliError {
    /// 错误文本（对齐 ffprobe 风格）.
    pub message: String,
    /// 是否隐藏 banner.
    pub hide_banner: bool,
}

/// 单个已解析参数.
#[derive(Debug, Clone)]
pub struct ParsedOption {
    /// 规范名.
    pub canonical: String,
    /// 参数值（如存在）.
    pub value: Option<String>,
    /// 原始参数名（不含前导短横线）.
    #[allow(dead_code)]
    pub raw_name: String,
    /// 原始参数 token（保留用户写法）.
    #[allow(dead_code)]
    pub raw_token: String,
    /// 原始值 token（仅当值来自独立 token 或 inline 时存在）.
    #[allow(dead_code)]
    pub raw_value: Option<String>,
    /// 值来源形态.
    pub value_source: OptionValueSource,
    /// 是否来自隐藏别名映射.
    #[allow(dead_code)]
    pub is_legacy_alias: bool,
    /// 是否 AVOption 名称接口.
    pub is_avoption: bool,
}

/// 参数值来源形态.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionValueSource {
    /// 无值参数.
    None,
    /// `-opt value`.
    Separate,
    /// `-opt=value`.
    Inline,
    /// 可选值参数未显式提供时的隐式默认值.
    ImplicitDefault,
    /// 隐藏别名映射生成的值.
    LegacyAlias,
}

/// 已解析命令行.
#[derive(Debug, Clone)]
pub struct ParsedArgs {
    /// 调用名.
    pub invocation_name: String,
    /// 原始参数序列（不含 argv[0]）.
    #[allow(dead_code)]
    pub raw_tokens: Vec<String>,
    /// passthrough 参数序列（保序, 且已展开隐藏别名）.
    pub passthrough_tokens: Vec<String>,
    /// 选项（按出现顺序）.
    pub options: Vec<ParsedOption>,
    /// 位置参数（通常是输入文件）.
    pub positionals: Vec<String>,
}

impl ParsedArgs {
    /// 判断是否包含指定规范名.
    pub fn has(&self, canonical: &str) -> bool {
        self.options.iter().any(|opt| opt.canonical == canonical)
    }

    /// 读取最后一次出现的值.
    pub fn last_value(&self, canonical: &str) -> Option<&str> {
        self.options
            .iter()
            .rev()
            .find(|opt| opt.canonical == canonical)
            .and_then(|opt| opt.value.as_deref())
    }

    /// 读取最后一次出现的完整参数.
    pub fn last_option(&self, canonical: &str) -> Option<&ParsedOption> {
        self.options
            .iter()
            .rev()
            .find(|opt| opt.canonical == canonical)
    }

    /// 读取所有值.
    pub fn values(&self, canonical: &str) -> Vec<&str> {
        self.options
            .iter()
            .filter(|opt| opt.canonical == canonical)
            .filter_map(|opt| opt.value.as_deref())
            .collect()
    }
}

/// 解析 argv（`argv[0]` 为程序名）.
pub fn parse_argv(argv: &[String], invocation_name: &str) -> Result<ParsedArgs, CliError> {
    let mut options = Vec::new();
    let mut positionals = Vec::new();
    let mut passthrough_tokens = Vec::new();
    let mut i = 1usize;
    let mut stop_option_scan = false;
    let mut hide_banner = false;

    while i < argv.len() {
        let token = &argv[i];

        if stop_option_scan {
            positionals.push(token.clone());
            passthrough_tokens.push(token.clone());
            i += 1;
            continue;
        }

        if token == "--" {
            stop_option_scan = true;
            passthrough_tokens.push(token.clone());
            i += 1;
            continue;
        }

        let stripped = if token.starts_with("--") && token.len() > 2 {
            Some((&token[2..], true))
        } else if token.starts_with('-') && token.len() > 1 {
            Some((&token[1..], false))
        } else {
            None
        };

        let Some((raw, was_double_dash)) = stripped else {
            positionals.push(token.clone());
            passthrough_tokens.push(token.clone());
            i += 1;
            continue;
        };

        let (name, inline_value) = split_name_value(raw);

        if let Some((mapped_canonical, mapped_value)) = find_legacy_alias(name) {
            let value = mapped_value.map(ToString::to_string);
            passthrough_tokens.push(canonical_flag(mapped_canonical));
            if let Some(mapped_value) = mapped_value {
                passthrough_tokens.push(mapped_value.to_string());
            }
            options.push(ParsedOption {
                canonical: mapped_canonical.to_string(),
                value,
                raw_name: name.to_string(),
                raw_token: token.clone(),
                raw_value: mapped_value.map(ToString::to_string),
                value_source: OptionValueSource::LegacyAlias,
                is_legacy_alias: true,
                is_avoption: false,
            });
            if mapped_canonical == "hide_banner" {
                hide_banner = true;
            }
            i += 1;
            continue;
        }

        if let Some(spec) = find_main_option(name) {
            let mut value = inline_value.map(ToString::to_string);
            let mut raw_value = inline_value.map(ToString::to_string);
            let mut value_source = if inline_value.is_some() {
                OptionValueSource::Inline
            } else {
                OptionValueSource::None
            };
            match spec.value_kind {
                OptionValueKind::None => {
                    if value.is_some() {
                        return Err(CliError {
                            message: format!(
                                "Failed to set value '{}' for option '{}': Option does not take argument",
                                value.unwrap_or_default(),
                                name
                            ),
                            hide_banner,
                        });
                    }
                }
                OptionValueKind::Required => {
                    if value.is_none() {
                        if let Some(next) = argv.get(i + 1) {
                            value = Some(next.clone());
                            raw_value = Some(next.clone());
                            value_source = OptionValueSource::Separate;
                            i += 1;
                        } else {
                            return Err(CliError {
                                message: format!("Missing argument for option '{}'", name),
                                hide_banner,
                            });
                        }
                    }
                }
                OptionValueKind::Optional => {
                    if value.is_none() {
                        if let Some(next) = argv.get(i + 1) {
                            if !looks_like_option(next) {
                                value = Some(next.clone());
                                raw_value = Some(next.clone());
                                value_source = OptionValueSource::Separate;
                                i += 1;
                            } else {
                                value = Some("1".to_string());
                                raw_value = None;
                                value_source = OptionValueSource::ImplicitDefault;
                            }
                        } else {
                            value = Some("1".to_string());
                            raw_value = None;
                            value_source = OptionValueSource::ImplicitDefault;
                        }
                    }
                }
            }

            if spec.canonical == "hide_banner" {
                hide_banner = value.as_deref() != Some("0") && value.as_deref() != Some("false");
            }

            options.push(ParsedOption {
                canonical: spec.canonical.to_string(),
                value,
                raw_name: name.to_string(),
                raw_token: token.clone(),
                raw_value: raw_value.clone(),
                value_source,
                is_legacy_alias: false,
                is_avoption: false,
            });
            passthrough_tokens.push(token.clone());
            if value_source == OptionValueSource::Separate
                && let Some(raw_value) = raw_value
            {
                passthrough_tokens.push(raw_value);
            }
            i += 1;
            continue;
        }

        if is_avoption_name(name) {
            let mut value = inline_value.map(ToString::to_string);
            let mut raw_value = inline_value.map(ToString::to_string);
            let mut value_source = if inline_value.is_some() {
                OptionValueSource::Inline
            } else {
                OptionValueSource::None
            };
            if value.is_none() {
                if let Some(next) = argv.get(i + 1) {
                    value = Some(next.clone());
                    raw_value = Some(next.clone());
                    value_source = OptionValueSource::Separate;
                    i += 1;
                } else {
                    return Err(CliError {
                        message: format!("Missing argument for option '{}'", name),
                        hide_banner,
                    });
                }
            }

            options.push(ParsedOption {
                canonical: name.to_string(),
                value,
                raw_name: name.to_string(),
                raw_token: token.clone(),
                raw_value: raw_value.clone(),
                value_source,
                is_legacy_alias: false,
                is_avoption: true,
            });
            passthrough_tokens.push(token.clone());
            if value_source == OptionValueSource::Separate
                && let Some(raw_value) = raw_value
            {
                passthrough_tokens.push(raw_value);
            }
            i += 1;
            continue;
        }

        // 未知参数: 对齐 ffprobe 常见错误风格.
        if inline_value.is_some() {
            let unknown = if was_double_dash {
                format!("-{}", raw)
            } else {
                raw.to_string()
            };
            return Err(CliError {
                message: format!("Missing argument for option '{}'", unknown),
                hide_banner,
            });
        }

        if let Some(next) = argv.get(i + 1) {
            if !looks_like_option(next) {
                return Err(CliError {
                    message: format!(
                        "Failed to set value '{}' for option '{}': Option not found",
                        next, name
                    ),
                    hide_banner,
                });
            }
        }

        return Err(CliError {
            message: format!("Missing argument for option '{}'", name),
            hide_banner,
        });
    }

    Ok(ParsedArgs {
        invocation_name: invocation_name.to_string(),
        raw_tokens: argv[1..].to_vec(),
        passthrough_tokens,
        options,
        positionals,
    })
}

fn split_name_value(raw: &str) -> (&str, Option<&str>) {
    if let Some((name, value)) = raw.split_once('=') {
        (name, Some(value))
    } else {
        (raw, None)
    }
}

fn looks_like_option(token: &str) -> bool {
    token.starts_with('-') && token != "-"
}

fn find_legacy_alias(alias: &str) -> Option<(&'static str, Option<&'static str>)> {
    LEGACY_ALIAS_OPTIONS
        .iter()
        .find(|(legacy, _, _)| *legacy == alias)
        .map(|(_, canonical, value)| (*canonical, *value))
}

fn canonical_flag(canonical: &str) -> String {
    format!("-{}", canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec_of(items: &[&str]) -> Vec<String> {
        items.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn test_parse_single_dash_long_and_value_eq() {
        let argv = vec_of(&[
            "ffprobe",
            "-show_streams",
            "-of=json",
            "-select_streams",
            "v:0",
            "input.mp4",
        ]);
        let parsed = parse_argv(&argv, "ffprobe").expect("解析失败");
        assert!(parsed.has("show_streams"), "应识别 -show_streams");
        assert_eq!(
            parsed.last_value("output_format"),
            Some("json"),
            "应识别 -of=json"
        );
        assert_eq!(
            parsed
                .last_option("output_format")
                .map(|opt| opt.value_source),
            Some(OptionValueSource::Inline),
            "应记录 inline 值来源"
        );
        assert_eq!(
            parsed.last_value("select_streams"),
            Some("v:0"),
            "应识别 -select_streams v:0"
        );
        assert_eq!(parsed.positionals, vec!["input.mp4"], "应识别输入参数");
    }

    #[test]
    fn test_parse_legacy_aliases() {
        let argv = vec_of(&["tao-probe", "--json", "--show-format", "a.mp4"]);
        let parsed = parse_argv(&argv, "tao-probe").expect("解析失败");
        assert_eq!(
            parsed.last_value("output_format"),
            Some("json"),
            "--json 应映射到 -of json"
        );
        assert!(parsed.has("show_format"), "--show-format 应映射成功");
        assert_eq!(
            parsed.passthrough_tokens,
            vec!["-output_format", "json", "-show_format", "a.mp4"],
            "passthrough 应保序展开隐藏别名"
        );
    }

    #[test]
    fn test_parse_unknown_with_value_error_style() {
        let argv = vec_of(&["ffprobe", "-foo", "bar"]);
        let err = parse_argv(&argv, "ffprobe").expect_err("应报错");
        assert!(
            err.message
                .contains("Failed to set value 'bar' for option 'foo': Option not found"),
            "错误文案应对齐 ffprobe 风格"
        );
    }

    #[test]
    fn test_parse_unknown_inline_value_error_style() {
        let argv = vec_of(&["ffprobe", "-foo=bar"]);
        let err = parse_argv(&argv, "ffprobe").expect_err("应报错");
        assert_eq!(
            err.message, "Missing argument for option 'foo=bar'",
            "inline 未知参数错误文案应与 ffprobe 对齐"
        );
    }

    #[test]
    fn test_parse_double_dash_stop_option_scan() {
        let argv = vec_of(&["ffprobe", "-show_streams", "--", "-input.mp4"]);
        let parsed = parse_argv(&argv, "ffprobe").expect("解析失败");
        assert!(parsed.has("show_streams"), "应识别 -- 前选项");
        assert_eq!(
            parsed.positionals,
            vec!["-input.mp4"],
            "-- 后参数必须作为位置参数"
        );
    }

    #[test]
    fn test_parse_optional_value_default_to_one() {
        let argv = vec_of(&["ffprobe", "-hide_banner"]);
        let parsed = parse_argv(&argv, "ffprobe").expect("解析失败");
        assert_eq!(
            parsed.last_value("hide_banner"),
            Some("1"),
            "可选参数未给值时应默认 1"
        );
        assert_eq!(
            parsed
                .last_option("hide_banner")
                .map(|opt| opt.value_source),
            Some(OptionValueSource::ImplicitDefault),
            "应区分隐式默认值来源"
        );
        assert_eq!(
            parsed.passthrough_tokens,
            vec!["-hide_banner"],
            "隐式默认值不应追加伪造值 token"
        );
    }

    #[test]
    fn test_parse_repeated_options_last_wins() {
        let argv = vec_of(&["ffprobe", "-of", "json", "-of", "xml"]);
        let parsed = parse_argv(&argv, "ffprobe").expect("解析失败");
        assert_eq!(
            parsed.last_value("output_format"),
            Some("xml"),
            "重复参数应以后者为准"
        );
        assert_eq!(
            parsed.values("output_format"),
            vec!["json", "xml"],
            "应保留重复参数历史顺序"
        );
    }
}
