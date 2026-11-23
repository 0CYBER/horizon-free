//! A website bug report as a GitHub issue: the text made inert (no mentions, no cross-references, no HTML),
//! written as the template Kira's `/report` and `#horizon-bugs` use, and the call that opens it.
use std::time::Duration;

use serde_json::{Map, Value, json};

/// Opens an issue; its URL, or why not. The server's is [`Filer::github`]; tests give their own.
pub type Filer = dyn Fn(&Issue) -> Result<String, String> + Send + Sync;

pub struct Issue {
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
}

/// Labels every website report gets.
pub const LABELS: [&str; 2] = ["bug", "from-website"];

/// Text people wrote, safe in an issue: no HTML, no `@` mentions, no `#12`, `GH-12` or issue-URL
/// cross-references, no closing keywords, no code spans to break out of. The same rules as Kira's
/// `neutralise`.
pub fn neutralise(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let word_start = i == 0 || !chars[i - 1].is_alphanumeric();
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '`' => out.push('\''),
            // A zero-width space after `@` or `#` keeps the text and drops the mention or the link.
            '@' | '#' if next.is_some_and(|n| n.is_alphanumeric()) => {
                out.push(c);
                out.push('\u{200b}');
            }
            // GitHub also links `GH-12` (either case) to issue 12: a zero-width space after `GH`.
            'g' | 'G' if word_start && gh_reference(&chars[i..]) => {
                out.push(c);
                out.push(chars[i + 1]);
                out.push('\u{200b}');
                i += 1;
            }
            _ if word_start && closing_keyword(&chars[i..]) => {
                out.push(c);
                out.push('\u{200b}');
            }
            _ => out.push(c),
        }
        i += 1;
    }
    // github.com/owner/repo/issues/12 links cross-reference the issue: shown, not linked.
    let out = out.replace("github.com/", "github.com\u{200b}/");
    break_links(&out)
}

/// Markdown links and images (`[t](url)`, `![t](url)`, `[t][ref]` with `[ref]: url`, `[t]` with `[t]: url`) all
/// need `](`, `][` or `]:` right after the label. A zero-width space there keeps the text readable but makes it
/// plain text, so an anonymous report can't carry a link with a hidden target or an image that loads when a
/// maintainer opens the issue. (GitHub proxies images, so it would not leak the maintainer's IP; the point is
/// the content.) Bare URLs stay: they are shown as written and a server address is a normal thing to report.
/// Idempotent, as the title is neutralised twice.
fn break_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == ']' && matches!(chars.peek(), Some('(' | '[' | ':')) {
            out.push('\u{200b}');
        }
    }
    out
}

/// Whether `rest` starts with `GH-` and a digit, as GitHub's issue autolink reads it.
fn gh_reference(rest: &[char]) -> bool {
    matches!(rest, [_, 'h' | 'H', '-', d, ..] if d.is_ascii_digit())
}

fn closing_keyword(rest: &[char]) -> bool {
    let word: String = rest
        .iter()
        .take_while(|c| c.is_alphabetic())
        .flat_map(|c| c.to_lowercase())
        .collect();
    [
        "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
    ]
    .contains(&word.as_str())
}

/// Text as a markdown quote, every line prefixed.
fn quote(text: &str) -> String {
    text.lines()
        .map(|l| format!("> {l}").trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The issue for a report's fields (`hub::REPORT`'s names, already checked).
pub fn issue(fields: &Map<String, Value>) -> Issue {
    let field = |name: &str| {
        fields
            .get(name)
            .and_then(Value::as_str)
            .map(neutralise)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "not given".into())
    };
    let title = neutralise(&field("title").replace(['\n', '\r'], " "));
    let text = [
        format!("**What I did:**\n{}", field("did")),
        format!("**What happened:**\n{}", field("happened")),
        format!("**What I expected:**\n{}", field("expected")),
        format!("**Horizon version:** {}", field("version")),
        format!("**OS, CPU, GPU:** {}", field("system")),
    ]
    .join("\n");
    let by = fields
        .get("name")
        .and_then(Value::as_str)
        .map(|n| neutralise(n).replace(['*', '_', '~', '|', '[', ']'], ""))
        .filter(|n| !n.is_empty())
        .map(|n| format!(" by **{n}**"))
        .unwrap_or_default();
    let body = format!(
        "{}\n\nReported on playhorizon.dev{by}\n\n<sub>Opened by the bug form on playhorizon.dev/hub. The reporter has this issue's link; they don't get GitHub notifications.</sub>",
        quote(&text)
    );
    Issue {
        title,
        body,
        labels: LABELS.iter().map(|l| (*l).to_owned()).collect(),
    }
}

/// Opens issues in `repo` (`owner/name`) with `token` (a fine-grained token with Issues: write on that repo
/// only). Errors never carry the token or GitHub's answer body.
pub fn github(repo: String, token: String) -> Box<Filer> {
    Box::new(move |issue: &Issue| {
        let response = ureq::post(&format!("https://api.github.com/repos/{repo}/issues"))
            .set("Authorization", &format!("Bearer {token}"))
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28")
            .set("User-Agent", "horizon-telemetry-server")
            .timeout(Duration::from_secs(20))
            .send_json(json!({ "title": issue.title, "body": issue.body, "labels": issue.labels }));
        match response {
            Ok(response) => response
                .into_json::<Value>()
                .ok()
                .and_then(|v| v["html_url"].as_str().map(str::to_owned))
                .ok_or_else(|| "GitHub's answer had no issue URL".into()),
            Err(ureq::Error::Status(code, _)) => Err(format!("GitHub answered {code}")),
            Err(ureq::Error::Transport(e)) => Err(format!("could not reach GitHub: {}", e.kind())),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_and_images_are_not_live_markdown() {
        for text in [
            "[click](https://evil.example/login)",
            "![x](https://evil.example/p.png)",
            "[a][b]\n\n[b]: https://evil.example",
            "![a][b]\n\n[b]: https://evil.example",
            "[a]\n\n[a]: https://evil.example",
        ] {
            let out = neutralise(text);
            assert!(
                !out.contains("](") && !out.contains("][") && !out.contains("]:"),
                "{out:?}"
            );
            assert_eq!(neutralise(&out), out, "idempotent");
            assert_eq!(out.replace('\u{200b}', ""), text, "text stays readable");
        }
    }

    #[test]
    fn issue_body_has_no_live_link_and_keeps_bare_urls() {
        let mut fields = Map::new();
        fields.insert(
            "did".into(),
            json!("![x](https://evil.example/p.png) join play.example.org:25565"),
        );
        fields.insert("title".into(), json!("[t](https://evil.example)"));
        let issue = issue(&fields);
        assert!(!issue.body.contains("]("), "{}", issue.body);
        assert!(!issue.title.contains("]("), "{}", issue.title);
        assert!(issue.body.contains("play.example.org:25565"));
    }
}
