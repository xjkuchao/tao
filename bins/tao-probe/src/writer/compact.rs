//! compact writer.

use std::io::Write;

use crate::model::{ProbeDocument, ProbeSection};

pub fn write(doc: &ProbeDocument, output: &mut dyn Write) -> std::io::Result<()> {
    for section in &doc.sections {
        write_line(section, output)?;
    }
    Ok(())
}

fn write_line(section: &ProbeSection, output: &mut dyn Write) -> std::io::Result<()> {
    write!(output, "{}", section.name.to_lowercase())?;
    for field in &section.fields {
        write!(output, "|{}={}", field.key, field.value.as_text())?;
    }
    writeln!(output)?;

    for child in &section.children {
        write_line(child, output)?;
    }
    Ok(())
}
