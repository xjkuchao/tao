//! ini writer.

use std::io::Write;

use crate::model::{ProbeDocument, ProbeSection};

pub fn write(doc: &ProbeDocument, output: &mut dyn Write) -> std::io::Result<()> {
    for section in &doc.sections {
        write_section(section, output)?;
    }
    Ok(())
}

fn write_section(section: &ProbeSection, output: &mut dyn Write) -> std::io::Result<()> {
    writeln!(output, "[{}]", section.name.to_lowercase())?;
    for field in &section.fields {
        writeln!(output, "{}={}", field.key, field.value.as_text())?;
    }
    writeln!(output)?;

    for child in &section.children {
        write_section(child, output)?;
    }

    Ok(())
}
