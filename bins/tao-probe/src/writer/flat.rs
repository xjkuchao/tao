//! flat writer.

use std::io::Write;

use crate::model::{ProbeDocument, ProbeSection};

pub fn write(doc: &ProbeDocument, output: &mut dyn Write) -> std::io::Result<()> {
    let mut counters = std::collections::BTreeMap::<String, usize>::new();
    for section in &doc.sections {
        let key = section.name.to_lowercase();
        let idx = counters.entry(key.clone()).or_insert(0);
        write_section(section, &format!("{}.{}", key, *idx), output)?;
        *idx += 1;
    }
    Ok(())
}

fn write_section(
    section: &ProbeSection,
    path: &str,
    output: &mut dyn Write,
) -> std::io::Result<()> {
    for field in &section.fields {
        writeln!(
            output,
            "{}.{}={}",
            path,
            field.key,
            quote(&field.value.as_text())
        )?;
    }

    let mut counters = std::collections::BTreeMap::<String, usize>::new();
    for child in &section.children {
        let key = child.name.to_lowercase();
        let idx = counters.entry(key.clone()).or_insert(0);
        let child_path = format!("{}.{}.{}", path, key, *idx);
        write_section(child, &child_path, output)?;
        *idx += 1;
    }

    Ok(())
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}
