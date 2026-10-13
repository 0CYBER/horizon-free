//! Release notes and the roadmap: pulldown-cmark events into gpui elements.
use crate::theme::Theme;
use gpui::{
    div, prelude::*, px, AnyElement, FontStyle, FontWeight, HighlightStyle, SharedString,
    StyledText,
};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::ops::Range;

/// A run of text's styled ranges.
type Spans = Vec<(Range<usize>, Span)>;
/// A table row: each cell's text and its spans.
type Row = Vec<(String, Spans)>;

enum Block {
    Heading(u8, String, Spans),
    Paragraph(String, Spans),
    Item(usize, Option<u64>, String, Spans),
    Code(String),
    Rule,
    Table(Vec<Row>),
}

#[derive(Clone, Copy)]
enum Span {
    Strong,
    Emphasis,
    Code,
    Link,
}

pub fn render(source: &str, t: &Theme) -> AnyElement {
    let mut blocks = Vec::new();
    let (mut text, mut spans) = (String::new(), Vec::new());
    let mut open: Vec<(Span, usize)> = Vec::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut item: Option<(usize, Option<u64>)> = None;
    let mut heading = None;
    let mut code: Option<String> = None;
    let mut table: Option<Vec<Row>> = None;
    let flush = |blocks: &mut Vec<Block>,
                 text: &mut String,
                 spans: &mut Vec<_>,
                 item: &mut Option<(usize, Option<u64>)>,
                 heading: Option<u8>| {
        if text.trim().is_empty() {
            text.clear();
            spans.clear();
            return;
        }
        let (t, s) = (std::mem::take(text), std::mem::take(spans));
        blocks.push(match (heading, item.take()) {
            (Some(level), _) => Block::Heading(level, t, s),
            (None, Some((depth, n))) => Block::Item(depth, n, t, s),
            _ => Block::Paragraph(t, s),
        });
    };
    for event in Parser::new_ext(
        source,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES,
    ) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    _ => 3,
                })
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut blocks, &mut text, &mut spans, &mut item, heading);
                heading = None;
            }
            Event::End(TagEnd::Paragraph) => {
                flush(&mut blocks, &mut text, &mut spans, &mut item, None)
            }
            Event::Start(Tag::List(start)) => {
                flush(&mut blocks, &mut text, &mut spans, &mut item, None);
                lists.push(start)
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut blocks, &mut text, &mut spans, &mut item, None);
                let depth = lists.len().saturating_sub(1);
                let number = lists.last_mut().and_then(|n| {
                    n.as_mut().map(|n| {
                        *n += 1;
                        *n - 1
                    })
                });
                item = Some((depth, number));
            }
            Event::End(TagEnd::Item) => flush(&mut blocks, &mut text, &mut spans, &mut item, None),
            Event::Start(Tag::CodeBlock(_)) => code = Some(String::new()),
            Event::End(TagEnd::CodeBlock) => {
                blocks.push(Block::Code(code.take().unwrap_or_default()))
            }
            Event::Start(Tag::Strong) => open.push((Span::Strong, text.len())),
            Event::Start(Tag::Emphasis) => open.push((Span::Emphasis, text.len())),
            Event::Start(Tag::Link { .. }) => open.push((Span::Link, text.len())),
            Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Link) => {
                if let Some((span, start)) = open.pop() {
                    spans.push((start..text.len(), span));
                }
            }
            Event::Code(c) => {
                let start = text.len();
                text.push_str(&c);
                spans.push((start..text.len(), Span::Code));
            }
            Event::Text(s) => match &mut code {
                Some(c) => c.push_str(&s),
                None => text.push_str(&s),
            },
            Event::SoftBreak => text.push(' '),
            Event::HardBreak => text.push('\n'),
            Event::Rule => blocks.push(Block::Rule),
            Event::Start(Tag::Table(_)) => {
                flush(&mut blocks, &mut text, &mut spans, &mut item, None);
                table = Some(Vec::new())
            }
            Event::Start(Tag::TableHead | Tag::TableRow) => {
                if let Some(t) = &mut table {
                    t.push(Vec::new())
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(row) = table.as_mut().and_then(|t| t.last_mut()) {
                    row.push((std::mem::take(&mut text), std::mem::take(&mut spans)));
                }
            }
            Event::End(TagEnd::Table) => {
                blocks.push(Block::Table(table.take().unwrap_or_default()))
            }
            _ => {}
        }
    }
    flush(&mut blocks, &mut text, &mut spans, &mut item, None);

    let styled = |text: String, spans: Spans| {
        let highlights = spans.into_iter().map(|(range, span)| {
            let style = match span {
                Span::Strong => HighlightStyle {
                    font_weight: Some(FontWeight::SEMIBOLD),
                    color: Some(t.text),
                    ..Default::default()
                },
                Span::Emphasis => HighlightStyle {
                    font_style: Some(FontStyle::Italic),
                    ..Default::default()
                },
                Span::Code => HighlightStyle {
                    background_color: Some(t.raised),
                    color: Some(t.text),
                    ..Default::default()
                },
                Span::Link => HighlightStyle {
                    color: Some(t.text),
                    ..Default::default()
                },
            };
            (range, style)
        });
        StyledText::new(SharedString::from(text)).with_highlights(highlights.collect::<Vec<_>>())
    };
    let mut column = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .text_size(px(13.5))
        .line_height(px(21.))
        .text_color(t.muted);
    for block in blocks {
        column = column.child(match block {
            Block::Heading(level, text, spans) => div()
                .pt(px(if level <= 2 { 12. } else { 6. }))
                .text_size(px(if level <= 2 { 14.5 } else { 13.5 }))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text)
                .child(styled(text, spans))
                .into_any_element(),
            Block::Paragraph(text, spans) => div().child(styled(text, spans)).into_any_element(),
            Block::Item(depth, number, text, spans) => div()
                .flex()
                .gap(px(8.))
                .pl(px(depth as f32 * 16.))
                .child(
                    div().flex_none().w(px(16.)).text_color(t.faint).child(
                        number
                            .map(|n| format!("{n}."))
                            .unwrap_or_else(|| "–".into()),
                    ),
                )
                .child(div().flex_1().min_w_0().child(styled(text, spans)))
                .into_any_element(),
            Block::Code(code) => div()
                .p(px(12.))
                .rounded(px(6.))
                .border_1()
                .border_color(t.border)
                .bg(t.raised)
                .font_family(crate::theme::MONO)
                .text_size(px(12.))
                .line_height(px(18.))
                .text_color(t.text)
                .child(code.trim_end().to_string())
                .into_any_element(),
            Block::Rule => div().h(px(1.)).bg(t.border).my(px(8.)).into_any_element(),
            Block::Table(rows) => {
                let mut grid = div()
                    .flex()
                    .flex_col()
                    .my(px(4.))
                    .border_t_1()
                    .border_color(t.border);
                for (i, row) in rows.into_iter().enumerate() {
                    let mut line = div()
                        .flex()
                        .gap(px(16.))
                        .py(px(8.))
                        .border_b_1()
                        .border_color(t.border);
                    let columns = row.len();
                    for (j, (text, spans)) in row.into_iter().enumerate() {
                        // Two-column tables keep a narrow first column; wider ones share the width.
                        let cell = if j == 0 && columns > 1 {
                            div().w(px(180.)).flex_none()
                        } else if j + 1 < columns && columns > 2 {
                            div().w(px(56.)).flex_none()
                        } else {
                            div().flex_1().min_w_0()
                        };
                        line = line.child(
                            cell.when(i == 0, |d| {
                                d.text_color(t.text)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(12.5))
                            })
                            .child(styled(text, spans)),
                        );
                    }
                    grid = grid.child(line);
                }
                grid.into_any_element()
            }
        });
    }
    column.into_any_element()
}
