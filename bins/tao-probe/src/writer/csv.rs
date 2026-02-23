//! csv writer.

use std::io::Write;

use crate::model::{ProbeDocument, ProbeSection};

pub fn write(doc: &ProbeDocument, output: &mut dyn Write) -> std::io::Result<()> {
    writeln!(output, "section,key,value")?;
    for section in &doc.sections {
        write_section(section, output)?;
    }
    Ok(())
}

fn write_section(section: &ProbeSection, output: &mut dyn Write) -> std::io::Result<()> {
    let section_name = section.name.to_lowercase();
    for field in &section.fields {
        writeln!(
            output,
            "{},{},{}",
            escape_csv(&section_name),
            escape_csv(&field.key),
            escape_csv(&field.value.as_text())
        )?;
    }

    for child in &section.children {
        write_section(child, output)?;
    }
    Ok(())
}

fn escape_csv(input: &str) -> String {
    let need_quote = input.contains(',') || input.contains('"') || input.contains('\n');
    if need_quote {
        format!("\"{}\"", input.replace('"', "\"\""))
    } else {
        input.to_string()
    }
}
