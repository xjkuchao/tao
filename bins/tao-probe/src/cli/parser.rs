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
    /// 是否 AVOption 名称接口.
    pub is_avoption: bool,
}

/// 已解析命令行.
#[derive(Debug, Clone)]
pub struct ParsedArgs {
    /// 调用名.
    pub invocation_name: String,
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
    let mut i = 1usize;
    let mut stop_option_scan = false;
    let mut hide_banner = false;

    while i < argv.len() {
        let token = &argv[i];

        if stop_option_scan {
            positionals.push(token.clone());
            i += 1;
            continue;
        }

        if token == "--" {
            stop_option_scan = true;
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
            i += 1;
            continue;
        };

        let (name, inline_value) = split_name_value(raw);

        if let Some((mapped_canonical, mapped_value)) = find_legacy_alias(name) {
            let value = mapped_value.map(ToString::to_string);
            options.push(ParsedOption {
                canonical: mapped_canonical.to_string(),
                value,
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
                                i += 1;
                            } else {
                                value = Some("1".to_string());
                            }
                        } else {
                            value = Some("1".to_string());
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
                is_avoption: false,
            });
            i += 1;
            continue;
        }

        if is_avoption_name(name) {
            let mut value = inline_value.map(ToString::to_string);
            if value.is_none() {
                if let Some(next) = argv.get(i + 1) {
                    value = Some(next.clone());
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
                is_avoption: true,
            });
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
