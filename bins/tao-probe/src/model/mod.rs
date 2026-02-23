//! 统一输出模型.
//!
//! 用于驱动多个 writer 的一致输出.

use serde::Serialize;
use serde_json::Value as JsonValue;

/// 字段值类型.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ValueType {
    String,
    Integer,
    Unsigned,
    Float,
    Bool,
    Null,
}

/// 字段值.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
#[serde(untagged)]
pub enum ProbeValue {
    String(String),
    Integer(i64),
    Unsigned(u64),
    Float(f64),
    Bool(bool),
    Null,
}

impl ProbeValue {
    /// 推断值类型.
    pub fn value_type(&self) -> ValueType {
        match self {
            Self::String(_) => ValueType::String,
            Self::Integer(_) => ValueType::Integer,
            Self::Unsigned(_) => ValueType::Unsigned,
            Self::Float(_) => ValueType::Float,
            Self::Bool(_) => ValueType::Bool,
            Self::Null => ValueType::Null,
        }
    }

    /// 转为文本.
    pub fn as_text(&self) -> String {
        match self {
            Self::String(v) => v.clone(),
            Self::Integer(v) => v.to_string(),
            Self::Unsigned(v) => v.to_string(),
            Self::Float(v) => format_float(*v),
            Self::Bool(v) => {
                if *v {
                    "1".to_string()
                } else {
                    "0".to_string()
                }
            }
            Self::Null => "N/A".to_string(),
        }
    }

    /// 转为 JSON 值.
    pub fn to_json_value(&self) -> JsonValue {
        match self {
            Self::String(v) => JsonValue::String(v.clone()),
            Self::Integer(v) => JsonValue::Number((*v).into()),
            Self::Unsigned(v) => JsonValue::Number((*v).into()),
            Self::Float(v) => serde_json::Number::from_f64(*v)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null),
            Self::Bool(v) => JsonValue::Bool(*v),
            Self::Null => JsonValue::Null,
        }
    }
}

/// 单个字段.
#[derive(Debug, Clone, Serialize)]
pub struct ProbeField {
    /// 字段键.
    pub key: String,
    /// 字段值.
    pub value: ProbeValue,
    /// 值类型.
    pub value_type: ValueType,
    /// 是否可选字段.
    pub optional: bool,
    /// 是否私有字段.
    pub private: bool,
    /// 默认是否显示.
    pub display_by_default: bool,
    /// JSON 写出时是否强制字符串.
    pub json_force_string: bool,
}

impl ProbeField {
    /// 创建字段.
    pub fn new(key: impl Into<String>, value: ProbeValue) -> Self {
        let value_type = value.value_type();
        Self {
            key: key.into(),
            value,
            value_type,
            optional: false,
            private: false,
            display_by_default: true,
            json_force_string: false,
        }
    }

    /// 标记 JSON 强制字符串写出.
    pub fn with_json_string(mut self) -> Self {
        self.json_force_string = true;
        self
    }

    /// 转为 JSON 值（结合字段元信息）.
    pub fn to_json_value(&self) -> JsonValue {
        if self.json_force_string {
            return JsonValue::String(self.value.as_text());
        }
        self.value.to_json_value()
    }
}

/// section 节点.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ProbeSection {
    /// section 名称（通常使用 ffprobe 大写风格, 如 `FORMAT` / `STREAM`）.
    pub name: String,
    /// 字段列表.
    pub fields: Vec<ProbeField>,
    /// 子 section 列表.
    pub children: Vec<ProbeSection>,
}

impl ProbeSection {
    /// 创建空 section.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            fields: Vec::new(),
            children: Vec::new(),
        }
    }

    /// 追加字段.
    pub fn push_field(&mut self, field: ProbeField) {
        self.fields.push(field);
    }
}

/// 输出文档根.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ProbeDocument {
    pub sections: Vec<ProbeSection>,
}

impl ProbeDocument {
    /// 追加根 section.
    pub fn push_section(&mut self, section: ProbeSection) {
        self.sections.push(section);
    }
}

fn format_float(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.6}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}
