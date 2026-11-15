//! Release notes: pulldown-cmark events into gpui elements.
use crate::cull::Cull;
use crate::theme::Theme;
use gpui::{
    div, prelude::*, px, AnyElement, Div, FontStyle, FontWeight, HighlightStyle, ScrollHandle,
    SharedString, StyledText,
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

/// Every block of `source`: for notes short enough to draw whole.
pub fn render(source: &str, t: &Theme) -> AnyElement {
    let notes = Notes::parse(source);
    frame(t)
        .gap(px(gap(t)))
        .children(notes.0.iter().map(|b| block(b, t)))
        .into_any_element()
}

/// Notes parsed once. A Nightly's notes run to some 1,500 items, too many to parse and lay out
/// on every frame, so [`Notes::column`] builds only the blocks near the view.
pub struct Notes(Vec<Block>);

impl Notes {
    pub fn parse(source: &str) -> Notes {
        Notes(parse(source))
    }

    /// The blocks near `scroll`'s view; `cull` keeps the others' heights as spacers.
    pub fn column(&self, t: &Theme, cull: &Cull, scroll: &ScrollHandle) -> Div {
        frame(t).child(cull.column(
            scroll,
            self.0.len(),
            gap(t),
            |_| false,
            |i| block(&self.0[i], t),
        ))
    }
}

fn parse(source: &str) -> Vec<Block> {
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
    blocks
}

fn styled(text: &str, spans: &Spans, t: &Theme) -> StyledText {
    let highlights = spans.iter().map(|(range, span)| {
        let style = match span {
            Span::Strong => HighlightStyle {
                font_weight: Some(FontWeight::SEMIBOLD),
                color: Some(t.ink),
                ..Default::default()
            },
            Span::Emphasis => HighlightStyle {
                font_style: Some(FontStyle::Italic),
                ..Default::default()
            },
            Span::Code => HighlightStyle {
                background_color: Some(t.plate_soft),
                color: Some(t.ink),
                ..Default::default()
            },
            Span::Link => HighlightStyle {
                color: Some(t.ink),
                ..Default::default()
            },
        };
        (range.clone(), style)
    });
    StyledText::new(SharedString::from(text.to_owned()))
        .with_highlights(highlights.collect::<Vec<_>>())
}

/// The space between blocks. The glass look sets notes as the mockup's: 14 px on 1.55 at 75 %,
/// lists with square bullets, 6 px apart.
fn gap(t: &Theme) -> f32 {
    if t.glass {
        6.
    } else {
        8.
    }
}

/// The notes' column, without the space between its blocks.
fn frame(t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .text_size(px(crate::theme::UI.px))
        .line_height(px(if t.glass { 21.7 } else { 22. }))
        .text_color(t.ink_soft)
}

fn block(block: &Block, t: &Theme) -> AnyElement {
    let glass = t.glass;
    match block {
        Block::Heading(level, text, spans) => div()
            .pt(px(if *level <= 2 { 12. } else { 6. }))
            .text_size(px(if *level <= 2 { 16. } else { 14. }))
            .font_weight(if glass {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::MEDIUM
            })
            .text_color(t.ink)
            .child(styled(text, spans, t))
            .into_any_element(),
        Block::Paragraph(text, spans) => div().child(styled(text, spans, t)).into_any_element(),
        Block::Item(depth, number, text, spans) => div()
            .flex()
            .gap(px(if glass { 12. } else { 8. }))
            .pl(px(*depth as f32 * 16.))
            .child(match number {
                // The mockup's bullet: a 5 px square, white at 40 %.
                None if glass => div()
                    .flex_none()
                    .mt(px(8.5))
                    .size(px(5.))
                    .bg(crate::glass_ui::white(0.4)),
                _ => div().flex_none().w(px(16.)).text_color(t.muted).child(
                    number
                        .map(|n| format!("{n}."))
                        .unwrap_or_else(|| "–".into()),
                ),
            })
            .child(div().flex_1().min_w_0().child(styled(text, spans, t)))
            .into_any_element(),
        Block::Code(code) => div()
            .p(px(16.))
            .rounded(px(if glass {
                12.
            } else {
                crate::theme::RADIUS_PLATE
            }))
            .bg(t.plate_soft)
            .font_family(crate::theme::MONO)
            .text_size(px(12.))
            .line_height(px(18.))
            .text_color(t.ink)
            .child(code.trim_end().to_string())
            .into_any_element(),
        Block::Rule => div().h(px(1.)).bg(t.rule).my(px(8.)).into_any_element(),
        Block::Table(rows) => {
            let mut grid = div()
                .flex()
                .flex_col()
                .my(px(4.))
                .border_t_1()
                .border_color(t.rule);
            for (i, row) in rows.iter().enumerate() {
                let mut line = div()
                    .flex()
                    .gap(px(16.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(t.rule);
                let columns = row.len();
                for (j, (text, spans)) in row.iter().enumerate() {
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
                            d.text_color(t.ink)
                                .font_weight(FontWeight::MEDIUM)
                                .text_size(px(12.5))
                        })
                        .child(styled(text, spans, t)),
                    );
                }
                grid = grid.child(line);
            }
            grid.into_any_element()
        }
    }
}
