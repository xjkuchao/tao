//! json writer.

use std::collections::BTreeMap;
use std::io::Write;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::model::{ProbeDocument, ProbeField, ProbeSection};
use crate::writer::OutputFormatSpec;

pub fn write(
    doc: &ProbeDocument,
    output: &mut dyn Write,
    spec: &OutputFormatSpec,
) -> std::io::Result<()> {
    let mut root = Map::new();
    let mut grouped: BTreeMap<String, (bool, Vec<Map<String, Value>>)> = BTreeMap::new();

    for section in &doc.sections {
        let (key, always_array) = json_key_for_section(&section.name);
        grouped
            .entry(key)
            .or_insert_with(|| (always_array, Vec::new()))
            .1
            .push(section_to_object(section));
    }

    for (key, (always_array, values)) in grouped {
        if !always_array && values.len() == 1 {
            root.insert(
                key,
                Value::Object(values.into_iter().next().unwrap_or_default()),
            );
        } else {
            root.insert(
                key,
                Value::Array(values.into_iter().map(Value::Object).collect()),
            );
        }
    }

    let compact = spec
        .options
        .get("compact")
        .map(|v| v == "1")
        .unwrap_or(false);
    let json_value = Value::Object(root);
    if compact {
        serde_json::to_writer(&mut *output, &json_value)?;
    } else {
        let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
        let mut serializer = serde_json::Serializer::with_formatter(&mut *output, formatter);
        json_value.serialize(&mut serializer)?;
    }
    writeln!(output)?;
    Ok(())
}

fn section_to_object(section: &ProbeSection) -> Map<String, Value> {
    let mut object = Map::new();

    for field in &section.fields {
        insert_field(&mut object, field);
    }

    let mut grouped: BTreeMap<String, (bool, Vec<Map<String, Value>>)> = BTreeMap::new();
    for child in &section.children {
        let (key, always_array) = json_key_for_section(&child.name);
        grouped
            .entry(key)
            .or_insert_with(|| (always_array, Vec::new()))
            .1
            .push(section_to_object(child));
    }

    for (key, (always_array, values)) in grouped {
        if !always_array && values.len() == 1 {
            object.insert(
                key,
                Value::Object(values.into_iter().next().unwrap_or_default()),
            );
        } else {
            object.insert(
                key,
                Value::Array(values.into_iter().map(Value::Object).collect()),
            );
        }
    }

    object
}

fn insert_field(object: &mut Map<String, Value>, field: &ProbeField) {
    object.insert(field.key.clone(), field.to_json_value());
}

fn json_key_for_section(section_name: &str) -> (String, bool) {
    match section_name {
        "FORMAT" => ("format".to_string(), false),
        "STREAM" => ("streams".to_string(), true),
        "PACKET" => ("packets".to_string(), true),
        "FRAME" => ("frames".to_string(), true),
        "PROGRAM" => ("programs".to_string(), true),
        "STREAM_GROUP" => ("stream_groups".to_string(), true),
        "CHAPTER" => ("chapters".to_string(), true),
        "PROGRAM_VERSION" => ("program_version".to_string(), false),
        "LIBRARY_VERSION" => ("library_versions".to_string(), true),
        "ERROR" => ("error".to_string(), false),
        "LOG" => ("log".to_string(), true),
        "DISPOSITION" => ("disposition".to_string(), false),
        "TAGS" => ("tags".to_string(), false),
        other => (other.to_ascii_lowercase(), false),
    }
}
