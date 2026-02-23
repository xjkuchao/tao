//! xml writer.

use std::io::Write;

use crate::model::{ProbeDocument, ProbeSection};

pub fn write(doc: &ProbeDocument, output: &mut dyn Write) -> std::io::Result<()> {
    writeln!(output, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>")?;
    writeln!(output, "<ffprobe>")?;

    for section in &doc.sections {
        write_section(section, 1, output)?;
    }

    writeln!(output, "</ffprobe>")?;
    Ok(())
}

fn write_section(
    section: &ProbeSection,
    indent: usize,
    output: &mut dyn Write,
) -> std::io::Result<()> {
    let tag = section.name.to_lowercase();
    let pad = "  ".repeat(indent);
    writeln!(output, "{}<{}>", pad, tag)?;

    for field in &section.fields {
        writeln!(
            output,
            "{}  <{}>{}</{}>",
            pad,
            field.key,
            escape_xml(&field.value.as_text()),
            field.key
        )?;
    }

    for child in &section.children {
        write_section(child, indent + 1, output)?;
    }

    writeln!(output, "{}</{}>", pad, tag)?;
    Ok(())
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
