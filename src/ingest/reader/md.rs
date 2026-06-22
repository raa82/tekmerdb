use anyhow::Result;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

pub fn read(path: &str) -> Result<String> {
    let raw = std::fs::read_to_string(path)?;
    Ok(strip_markdown(&raw))
}

pub fn strip_markdown(raw: &str) -> String {
    let parser = Parser::new_ext(raw, Options::empty());
    let mut out = String::new();
    let mut in_code = false;

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(_)) => in_code = true,
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Event::Text(t) if !in_code => out.push_str(&t),
            Event::SoftBreak => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            Event::HardBreak => out.push('\n'),
            Event::End(TagEnd::Paragraph) => out.push_str("\n\n"),
            Event::End(TagEnd::Heading { .. }) => out.push('\n'),
            _ => {}
        }
    }
    out
}
