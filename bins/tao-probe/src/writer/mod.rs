//! 多 writer 输出模块.

use std::collections::BTreeMap;
use std::io::Write;

use crate::model::ProbeDocument;

pub mod compact;
pub mod csv;
pub mod default;
pub mod flat;
pub mod ini;
pub mod json;
pub mod xml;

/// 输出格式类型.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Default,
    Compact,
    Csv,
    Flat,
    Ini,
    Json,
    Xml,
}

/// 输出格式配置.
#[derive(Debug, Clone)]
pub struct OutputFormatSpec {
    pub format: OutputFormat,
    pub options: BTreeMap<String, String>,
}

impl Default for OutputFormatSpec {
    fn default() -> Self {
        Self {
            format: OutputFormat::Default,
            options: BTreeMap::new(),
        }
    }
}

/// 解析 `-of` / `-output_format` 参数.
pub fn parse_output_format(raw: Option<&str>) -> Result<OutputFormatSpec, String> {
    let Some(raw) = raw else {
        return Ok(OutputFormatSpec::default());
    };

    let mut options = BTreeMap::new();
    let mut split = raw.split(':');
    let head = split.next().unwrap_or_default();

    let (name, first_option) = if let Some((name, option_expr)) = head.split_once('=') {
        (name, Some(option_expr))
    } else {
        (head, None)
    };

    if let Some(expr) = first_option {
        parse_option_expr(expr, &mut options);
    }

    for expr in split {
        parse_option_expr(expr, &mut options);
    }

    let format = match name {
        "default" => OutputFormat::Default,
        "compact" => OutputFormat::Compact,
        "csv" => OutputFormat::Csv,
        "flat" => OutputFormat::Flat,
        "ini" => OutputFormat::Ini,
        "json" => OutputFormat::Json,
        "xml" => OutputFormat::Xml,
        other => {
            return Err(format!("Unknown output format with name '{}'", other));
        }
    };

    Ok(OutputFormatSpec { format, options })
}

/// 根据格式写出文档.
pub fn write_document(
    spec: &OutputFormatSpec,
    doc: &ProbeDocument,
    output: &mut dyn Write,
) -> std::io::Result<()> {
    match spec.format {
        OutputFormat::Default => default::write(doc, output),
        OutputFormat::Compact => compact::write(doc, output),
        OutputFormat::Csv => csv::write(doc, output),
        OutputFormat::Flat => flat::write(doc, output),
        OutputFormat::Ini => ini::write(doc, output),
        OutputFormat::Json => json::write(doc, output, spec),
        OutputFormat::Xml => xml::write(doc, output),
    }
}

fn parse_option_expr(expr: &str, options: &mut BTreeMap<String, String>) {
    if expr.is_empty() {
        return;
    }
    if let Some((k, v)) = expr.split_once('=') {
        options.insert(k.to_string(), v.to_string());
    } else {
        options.insert(expr.to_string(), "1".to_string());
    }
}
