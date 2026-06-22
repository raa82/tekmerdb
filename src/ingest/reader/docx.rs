use anyhow::Result;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::io::{Cursor, Read};

pub fn read(path: &str) -> Result<String> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| anyhow::anyhow!("not a valid DOCX (ZIP) archive: {}", e))?;

    let xml_bytes = {
        let mut entry = archive
            .by_name("word/document.xml")
            .map_err(|_| anyhow::anyhow!("word/document.xml not found — is this a .docx file?"))?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        buf
    };

    let mut reader = Reader::from_reader(Cursor::new(xml_bytes));
    reader.config_mut().trim_text(true);

    let mut output = String::new();
    let mut buf = Vec::new();
    let mut in_text = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => match e.name().as_ref() {
                b"w:t" => in_text = true,
                b"w:p" => {
                    if !output.is_empty() && !output.ends_with('\n') {
                        output.push('\n');
                    }
                }
                _ => {}
            },
            Ok(Event::End(ref e)) => {
                if e.name().as_ref() == b"w:t" {
                    in_text = false;
                    output.push(' ');
                }
            }
            Ok(Event::Text(e)) if in_text => {
                output.push_str(&e.unescape()?.into_owned());
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("XML parse error: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(output)
}
