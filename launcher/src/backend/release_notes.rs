//! Release notes as plain text: markdown for terminals and the one-line summary.

/// Plain text from markdown, for terminals: headings on their own lines, bullets as "- ",
/// links followed by their address.
pub fn markdown_to_text(md: &str) -> String {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let mut out = String::new();
    let mut links = Vec::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    for event in Parser::new(md) {
        match event {
            Event::Start(Tag::Item) => {
                let depth = lists.len().saturating_sub(1);
                out.push_str(&"  ".repeat(depth));
                match lists.last_mut() {
                    Some(Some(n)) => {
                        out.push_str(&format!("{n}. "));
                        *n += 1;
                    }
                    _ => out.push_str("- "),
                }
            }
            Event::Start(Tag::List(start)) => {
                if !out.ends_with('\n') && !out.is_empty() {
                    out.push('\n');
                }
                lists.push(start)
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                if lists.is_empty() {
                    out.push('\n');
                }
            }
            Event::Start(Tag::Link { dest_url, .. }) => links.push(dest_url.to_string()),
            Event::End(TagEnd::Link) => {
                if let Some(url) = links.pop() {
                    out.push_str(&format!(" ({url})"));
                }
            }
            Event::End(TagEnd::Heading(_)) => out.push_str("\n\n"),
            Event::End(TagEnd::Paragraph) => {
                out.push_str(if lists.is_empty() { "\n\n" } else { "\n" })
            }
            Event::End(TagEnd::Item) if !out.ends_with('\n') => out.push('\n'),
            Event::End(TagEnd::CodeBlock) => out.push('\n'),
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak => out.push('\n'),
            Event::Rule => out.push_str("---\n\n"),
            _ => {}
        }
    }
    out.trim_end().to_string() + "\n"
}

/// The first sentence-like line of a release's notes, for a one-line summary.
pub fn notes_summary(body: &str) -> Option<String> {
    let text = markdown_to_text(body);
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| l.len() > 24 && !l.starts_with("---"))?;
    let line = line.trim_start_matches("- ");
    Some(match line.char_indices().nth(140) {
        Some((i, _)) => format!("{}…", line[..i].trim_end()),
        None => line.to_string(),
    })
}
