//! `DialogScreen` and its kinds (`SimpleDialogScreen`, `MultiButtonDialogScreen`):
//! where a server dialog's title, body, inputs and buttons go in a
//! `HeaderAndFooterLayout` (a 33 high header, the body in a `ScrollableLayout`,
//! the footer), and the `WaitingForResponseScreen` an answer can leave open.
use crate::{
    dialog::{Body, Control, Dialog, Kind},
    font::Font,
    text::{Style, Text},
};
use std::{cell::RefCell, sync::Arc};

/// `ActiveTextCollector.ClickableStyleFinder.accept`: the last style `wanted`
/// under the cursor, in the same visual order and advances as rendering.
/// A drawn glyph is hit where it is drawn (`BakedSheetGlyph.GlyphInstance`),
/// even far below the line when its font's `ascent` moves it; an empty one
/// covers the line's 9 pixels (`EmptyArea`).
pub fn line_style_at(
    font: &Font,
    text: &Text,
    x: f32,
    y: f32,
    cursor: (f32, f32),
    wanted: impl Fn(&Style) -> bool,
) -> Option<Style> {
    let text = text.resolved();
    let visual = crate::bidi::visual(&text);
    let mut pen = x;
    let mut found = None;
    for (c, style) in visual.chars() {
        let advance = font.advance(c, style);
        let object = c == '\u{fffc}' && style.inline_object.is_some();
        let glyph = (!object).then(|| font.glyph_styled(c, style));
        let (left, top, bottom) = match glyph {
            Some(g) if g.rect.is_some() => {
                let thick = if style.is_bold() { 0.1 } else { 0.0 };
                let shear = if style.is_italic() {
                    1.0 - 0.25 * g.bottom
                } else {
                    0.0
                };
                // Dialog text is drawn with a shadow, which extends the bottom.
                (
                    pen + g.left + shear - thick,
                    y + g.top - thick,
                    y + g.bottom + g.offset + thick,
                )
            }
            _ => (pen, y, y + 9.0),
        };
        let inside = (left..pen + advance).contains(&cursor.0) && (top..bottom).contains(&cursor.1);
        if inside && wanted(style) {
            found = Some(style.clone());
        }
        pen += advance;
    }
    found
}

/// `HeaderAndFooterLayout.DEFAULT_HEADER_AND_FOOTER_HEIGHT`.
const BAR: f32 = 33.0;
/// `Checkbox.getBoxSize`.
pub const BOX: f32 = 17.0;
/// `DialogScreen.WARNING_BUTTON_SPRITES`: normal, disabled, highlighted.
pub const WARNING_SPRITES: [&str; 3] = [
    "gui/sprites/dialog/warning_button",
    "gui/sprites/dialog/warning_button_disabled",
    "gui/sprites/dialog/warning_button_highlighted",
];

/// One drawn piece; `y` is from the top of the body (before it scrolls) for
/// the body's parts, from the top of the screen for the footer's.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    /// A line of text and where it starts; shared, so copying the layout or
    /// placing the body each frame doesn't copy every span of it.
    Line(Arc<Text>, f32, f32),
    /// A text input's box.
    Field { input: usize, rect: [f32; 4] },
    /// A boolean input: the box, its label right of it.
    Check { input: usize, rect: [f32; 4] },
    /// A button of `Dialog::buttons`.
    Button { index: usize, rect: [f32; 4] },
    /// A single option input's cycle button.
    Cycle { input: usize, rect: [f32; 4] },
    /// A number range input's slider.
    Slider { input: usize, rect: [f32; 4] },
    /// An item body's widget; `body` indexes `Dialog::body`.
    Item { body: usize, rect: [f32; 4] },
}
impl Part {
    /// The part moved by `(dx, dy)`.
    fn moved(self, dx: f32, dy: f32) -> Self {
        match self {
            Self::Line(t, x, y) => Self::Line(t, x + dx, y + dy),
            Self::Field { input, rect } => Self::Field {
                input,
                rect: shift(rect, dx, dy),
            },
            Self::Check { input, rect } => Self::Check {
                input,
                rect: shift(rect, dx, dy),
            },
            Self::Button { index, rect } => Self::Button {
                index,
                rect: shift(rect, dx, dy),
            },
            Self::Cycle { input, rect } => Self::Cycle {
                input,
                rect: shift(rect, dx, dy),
            },
            Self::Slider { input, rect } => Self::Slider {
                input,
                rect: shift(rect, dx, dy),
            },
            Self::Item { body, rect } => Self::Item {
                body,
                rect: shift(rect, dx, dy),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct Layout {
    /// The title and where it is drawn.
    pub title: (f32, f32),
    pub warning: [f32; 4],
    /// The body's visible area (`ScrollableLayout`'s container): x, y, w, h.
    pub area: [f32; 4],
    /// The body's full height: more than the area's scrolls.
    pub content: f32,
    pub body: Vec<Part>,
    pub footer: Vec<Part>,
}

/// A laid-out element of a `LinearLayout` before it is placed.
struct Element {
    w: f32,
    h: f32,
    parts: Vec<Part>,
}
impl Element {
    /// Its parts moved to `(x, y)`.
    fn at(self, x: f32, y: f32) -> impl Iterator<Item = Part> {
        self.parts.into_iter().map(move |p| p.moved(x, y))
    }
}
fn shift([x, y, w, h]: [f32; 4], dx: f32, dy: f32) -> [f32; 4] {
    [x + dx, y + dy, w, h]
}
/// `FrameLayout.alignInDimension` at 0.5: the offset that centres `inner`.
fn centre(outer: f32, inner: f32) -> f32 {
    ((outer - inner) * 0.5).trunc()
}

/// `FocusableTextWidget` with the body's settings: `width` wide, the text
/// wrapped to `width - 8` and centred, 4 of padding all round.
fn message(font: &Font, contents: &Text, width: f32) -> Element {
    let lines = font.split_lines(contents, width - 8.0);
    let mid = (width / 2.0).trunc();
    let parts = lines
        .into_iter()
        .enumerate()
        .map(|(i, (line, _))| {
            let x = mid - (font.width_text(&line) / 2.0).trunc();
            Part::Line(Arc::new(line), x, 4.0 + 9.0 * i as f32)
        })
        .collect::<Vec<_>>();
    Element {
        w: width,
        h: 9.0 * parts.len() as f32 + 8.0,
        parts,
    }
}

/// `DialogBodyHandlers.ItemHandler`: the item widget, with its description 2
/// to the right of it, both centred on each other vertically.
fn item(
    font: &Font,
    body: usize,
    description: Option<&(Text, i32)>,
    (w, h): (f32, f32),
) -> Element {
    let widget = |rect_h: f32, y: f32| Part::Item {
        body,
        rect: [0.0, y, w, rect_h],
    };
    let Some((contents, width)) = description else {
        return Element {
            w,
            h,
            parts: vec![widget(h, 0.0)],
        };
    };
    let text = message(font, contents, *width as f32);
    let height = h.max(text.h);
    let mut parts = vec![widget(h, centre(height, h))];
    parts.extend(
        text.parts
            .into_iter()
            .map(|p| p.moved(w + 2.0, centre(height, text.h))),
    );
    Element {
        w: w + 2.0 + text.w,
        h: height,
        parts,
    }
}

/// `TextInputHandler.addControl`: the 20 high `EditBox`, or the
/// `MultiLineEditBox` of the options' height, by default as tall as their most
/// lines (4 without a limit) need, at most 512.
pub fn text_height(multiline: Option<(Option<i32>, Option<i32>)>) -> f32 {
    match multiline {
        None => 20.0,
        Some((max_lines, height)) => {
            height.unwrap_or_else(|| (9 * max_lines.unwrap_or(4) + 8).min(512)) as f32
        }
    }
}

/// `InputControlHandlers`: the control for input `i`.
fn input(font: &Font, i: usize, control: &Control) -> Element {
    match control {
        Control::Text {
            width,
            label,
            label_visible,
            multiline,
            ..
        } => {
            let w = *width as f32;
            let h = text_height(*multiline);
            if !label_visible {
                return Element {
                    w,
                    h,
                    parts: vec![Part::Field {
                        input: i,
                        rect: [0.0, 0.0, w, h],
                    }],
                };
            }
            // CommonLayouts.labeledElement: the label, 4 below it the box,
            // both on the left. The label is a plain StringWidget: it has no
            // click handler, so a click event in it does nothing.
            let shown = label.map_styles(|style| style.click = None);
            Element {
                w: w.max(font.width_text(label)),
                h: 9.0 + 4.0 + h,
                parts: vec![
                    Part::Line(Arc::new(shown), 0.0, 0.0),
                    Part::Field {
                        input: i,
                        rect: [0.0, 13.0, w, h],
                    },
                ],
            }
        }
        Control::Boolean { label, .. } => {
            let w = BOX + 4.0 + font.width_text(label);
            Element {
                w,
                h: BOX,
                parts: vec![Part::Check {
                    input: i,
                    rect: [0.0, 0.0, w, BOX],
                }],
            }
        }
        // CycleButton and AbstractSliderButton: one 20 high button whose
        // message holds the label.
        Control::SingleOption { width, .. } => Element {
            w: *width as f32,
            h: 20.0,
            parts: vec![Part::Cycle {
                input: i,
                rect: [0.0, 0.0, *width as f32, 20.0],
            }],
        },
        Control::NumberRange { width, .. } => Element {
            w: *width as f32,
            h: 20.0,
            parts: vec![Part::Slider {
                input: i,
                rect: [0.0, 0.0, *width as f32, 20.0],
            }],
        },
    }
}

/// `DialogScreen.packControlsIntoColumns`: full rows in a grid 2 apart, what
/// is left in a centred row under them.
fn columns(widths: &[f32], columns: usize, first: usize) -> Element {
    let columns = columns.max(1);
    let full = widths.len() / columns * columns;
    let row_width = |ws: &[f32]| ws.iter().sum::<f32>() + 2.0 * (ws.len().max(1) - 1) as f32;
    let mut col = vec![0.0f32; columns];
    for (i, w) in widths[..full].iter().enumerate() {
        col[i % columns] = col[i % columns].max(*w);
    }
    let last = &widths[full..];
    if !last.is_empty() {
        // GridLayout spreads a cell over every column it spans (Divisor).
        let spread = row_width(last) - 2.0 * (columns - 1) as f32;
        let (each, extra) = ((spread / columns as f32).floor(), spread % columns as f32);
        for (c, w) in col.iter_mut().enumerate() {
            *w = w.max(each + f32::from(u8::from((c as f32) < extra)));
        }
    }
    let x_of = |c: usize| col[..c].iter().sum::<f32>() + 2.0 * c as f32;
    let mut parts = Vec::new();
    for (i, w) in widths[..full].iter().enumerate() {
        let (r, c) = (i / columns, i % columns);
        parts.push(Part::Button {
            index: first + i,
            rect: [x_of(c) + centre(col[c], *w), 22.0 * r as f32, *w, 20.0],
        });
    }
    let width = x_of(columns - 1) + col[columns - 1];
    if !last.is_empty() {
        let y = 22.0 * (full / columns) as f32;
        let mut x = centre(width, row_width(last));
        for (i, w) in last.iter().enumerate() {
            parts.push(Part::Button {
                index: first + full + i,
                rect: [x, y, *w, 20.0],
            });
            x += w + 2.0;
        }
    }
    let rows = full / columns + usize::from(!last.is_empty());
    Element {
        w: width,
        h: (22.0 * rows as f32 - 2.0).max(0.0),
        parts,
    }
}

impl Layout {
    /// `DialogScreen.init` and `repositionElements` on a `w` by `h` screen.
    pub fn new(dialog: &Dialog, font: &Font, (w, h): (f32, f32)) -> Self {
        // The header: the title, 10 right of it the warning button, both
        // centred together.
        let tw = font.width_text(&dialog.title);
        let (hx, hy) = (centre(w, tw + 30.0), centre(BAR, 20.0));
        let title = (hx, hy + centre(20.0, 9.0));
        let warning = [hx + tw + 10.0, hy, 20.0, 20.0];

        let mut elements: Vec<Element> = dialog
            .body
            .iter()
            .enumerate()
            .map(|(i, b)| match b {
                Body::Message { contents, width } => message(font, contents, *width as f32),
                Body::Item {
                    description,
                    width,
                    height,
                    ..
                } => item(
                    font,
                    i,
                    description.as_ref(),
                    (*width as f32, *height as f32),
                ),
            })
            .collect();
        elements.extend(
            dialog
                .inputs
                .iter()
                .enumerate()
                .map(|(i, input_)| input(font, i, &input_.control)),
        );
        let buttons = dialog.buttons();
        let (listed, footer_buttons, footer_h) = match &dialog.kind {
            Kind::MultiAction {
                exit, columns: n, ..
            }
            | Kind::DialogList {
                exit, columns: n, ..
            } => {
                let listed = buttons.len() - usize::from(exit.is_some());
                let widths: Vec<f32> = buttons[..listed].iter().map(|b| b.width as f32).collect();
                elements.push(columns(&widths, *n as usize, 0));
                // ButtonListDialogScreen: the exit button in the footer, else a 5 high one.
                let footer = exit.as_ref().map(|_| listed).into_iter().collect();
                (listed, footer, if exit.is_some() { BAR } else { 5.0 })
            }
            Kind::ServerLinks { exit, .. } => {
                let footer = exit.as_ref().map(|_| 0).into_iter().collect();
                (0, footer, if exit.is_some() { BAR } else { 5.0 })
            }
            // SimpleDialogScreen: the buttons side by side, 8 apart.
            Kind::Notice(_) | Kind::Confirmation { .. } => {
                (0, (0..buttons.len()).collect::<Vec<_>>(), BAR)
            }
        };
        debug_assert!(listed + footer_buttons.len() <= buttons.len());

        // The body: a column 10 apart, each element centred in it.
        let body_w = elements.iter().map(|e| e.w).fold(0.0, f32::max);
        let content = elements.iter().map(|e| e.h).sum::<f32>()
            + 10.0 * elements.len().saturating_sub(1) as f32;
        // ScrollableLayout: 10 each side for the scrollbar (ReserveStrategy.BOTH),
        // no higher than what the header and footer leave.
        let area_w = body_w + 20.0;
        let area_h = content.min(h - BAR - footer_h).max(0.0);
        let area_y = (BAR + 30.0).min(h - footer_h - area_h);
        let area_x = centre(w, area_w);
        let mut body = Vec::new();
        let mut y = 0.0;
        for e in elements {
            let (x, eh) = (area_x + 10.0 + centre(body_w, e.w), e.h);
            body.extend(e.at(x, y));
            y += eh + 10.0;
        }

        let widths: Vec<f32> = footer_buttons
            .iter()
            .map(|&i| buttons[i].width as f32)
            .collect();
        let row = widths.iter().sum::<f32>() + 8.0 * widths.len().saturating_sub(1) as f32;
        let (mut x, fy) = (centre(w, row), h - footer_h + centre(footer_h, 20.0));
        let mut footer = Vec::new();
        for (&i, bw) in footer_buttons.iter().zip(&widths) {
            footer.push(Part::Button {
                index: i,
                rect: [x, fy, *bw, 20.0],
            });
            x += bw + 8.0;
        }
        Self {
            title,
            warning: Self::in_bounds(warning, w, h),
            area: [area_x, area_y, area_w, area_h],
            content,
            body,
            footer,
        }
    }
    /// `makeSureWarningButtonIsInBounds`.
    fn in_bounds(r: [f32; 4], w: f32, h: f32) -> [f32; 4] {
        if r[0] < 0.0 || r[1] < 0.0 || r[0] > w - 20.0 || r[1] > h - 20.0 {
            [(w - 40.0).max(0.0), 5.0f32.min(h), 20.0, 20.0]
        } else {
            r
        }
    }
    /// Every part where it is drawn with the body scrolled by `scroll`, and
    /// whether it is in the body (clipped to its area).
    pub fn placed(&self, scroll: f32) -> impl Iterator<Item = (Part, bool)> + '_ {
        let dy = self.area[1] - scroll;
        let body = self
            .body
            .iter()
            .map(move |p| (p.clone().moved(0.0, dy), true));
        body.chain(self.footer.iter().map(|p| (p.clone(), false)))
    }
}

/// An open dialog's layout, kept until the screen's size, the font's glyphs
/// or the text direction change: `DialogScreen` builds its widgets once and
/// only repositions them on a resize (`MultiLineTextWidget` keeps its split
/// lines), so a body of many lines isn't wrapped again every frame (#19102).
/// One per open dialog.
#[derive(Debug, Default)]
pub struct LayoutCache(RefCell<Option<(LayoutKey, Arc<Layout>)>>);
/// What a layout depends on beside its dialog: the screen's size, the font's
/// epoch and whether the language reads right to left.
type LayoutKey = ((f32, f32), u64, bool);
impl LayoutCache {
    /// `dialog`'s layout on a `size` screen.
    pub fn get(&self, dialog: &Dialog, font: &Font, size: (f32, f32)) -> Arc<Layout> {
        let key = (size, font.epoch(), crate::lang::bidirectional());
        let mut cache = self.0.borrow_mut();
        if let Some((kept, layout)) = &*cache
            && *kept == key
        {
            return layout.clone();
        }
        let layout = Arc::new(Layout::new(dialog, font, size));
        *cache = Some((key, layout.clone()));
        layout
    }
}

/// `WaitingForResponseScreen`: its Back button's label after `ticks`, and
/// whether it shows and works (it shows after a second, works after five).
pub fn waiting_button(ticks: u32) -> (Text, bool, bool) {
    let seconds = (ticks / 20).min(5);
    let label = match seconds {
        0 => Text::literal(""),
        5 => Text::translate("gui.back", &[]),
        s => Text::translate(
            "gui.waitingForResponse.button.inactive",
            &[Text::literal((5 - s).to_string())],
        ),
    };
    (label, seconds >= 1, seconds == 5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_hit_testing_respects_sprite_advances_and_glyph_bounds() {
        use crate::{component, hud::Rect, text::ClickEvent};
        let font = Font::load(
            &horizon_assets::resource_source::ResourceSource::default(),
            &std::collections::HashMap::new(),
            &[],
            (1, 1),
            (0, 0),
            Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 1,
                h: 1,
            },
        );
        let text = component::to_text(
            &serde_json::json!([
                {"object": "atlas", "sprite": "minecraft:item/potion"},
                {"object": "atlas", "sprite": "minecraft:item/potion", "click_event": {
                    "action": "custom", "id": "test:shop"
                }},
                {"object": "atlas", "sprite": "minecraft:item/potion"},
            ]),
            &mut |_| Text::empty(),
        );
        let style = |x, y| line_style_at(&font, &text, 10.0, 20.0, (x, y), |_| true);
        assert!(style(9.9, 20.0).is_none());
        assert_eq!(style(17.9, 20.0).unwrap().click, None);
        for x in [18.0, 21.0, 25.9] {
            assert_eq!(
                style(x, 24.0).unwrap().click,
                Some(ClickEvent::Custom {
                    id: "test:shop".into(),
                    payload: None,
                })
            );
        }
        assert_eq!(style(26.0, 20.0).unwrap().click, None);
        assert!(style(34.0, 20.0).is_none());
        assert!(style(21.0, 19.9).is_none());
        assert!(style(21.0, 29.0).is_none());
    }

    #[test]
    fn a_glyph_moved_down_by_its_ascent_is_clicked_where_it_is_drawn() {
        use crate::{component, hud::Rect, text::ClickEvent};
        let root =
            std::env::temp_dir().join(format!("horizon-dialog-ascent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut sheet = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut sheet, 8, 8);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[255; 8 * 8 * 4])
                .unwrap();
        }
        for (path, bytes) in [
            (
                "assets/minecraft/font/low.json",
                br#"{"providers":[{"type":"bitmap","file":"minecraft:font/box.png","height":8,"ascent":-28,"chars":["a"]}]}"#
                    .as_slice(),
            ),
            ("assets/minecraft/textures/font/box.png", sheet.as_slice()),
        ] {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let font = Font::load(
            &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
            &std::collections::HashMap::new(),
            &[],
            (4096, 1024),
            (512, 512),
            Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 0.0,
                v1: 0.0,
                w: 1,
                h: 1,
            },
        );
        let _ = std::fs::remove_dir_all(&root);
        let text = component::to_text(
            &serde_json::json!([
                {"text": "a", "font": "minecraft:low"},
                {"text": "a", "font": "minecraft:low", "click_event": {
                    "action": "custom", "id": "test:requests"
                }},
            ]),
            &mut |_| Text::empty(),
        );
        let click = |x, y| {
            line_style_at(&font, &text, 10.0, 20.0, (x, y), |s| s.click.is_some())
                .and_then(|s| s.click)
        };
        let requests = Some(ClickEvent::Custom {
            id: "test:requests".into(),
            payload: None,
        });
        for (x, y) in [(19.0, 55.0), (22.0, 60.0), (27.9, 63.9)] {
            assert_eq!(click(x, y), requests, "drawn 35 below the line at {x}, {y}");
        }
        assert_eq!(click(22.0, 24.0), None, "the line's own row holds nothing");
        assert_eq!(click(22.0, 54.9), None, "above the glyph");
        assert_eq!(click(22.0, 65.0), None, "below the glyph and its shadow");
        assert_eq!(click(15.0, 60.0), None, "the plain glyph left of it");
    }

    /// #19102: a dialog of many lines whose every character has its own click
    /// event is laid out once, not again every frame, and laid out again when
    /// the screen changes size; each character still answers with its own click.
    #[test]
    fn a_dialog_is_laid_out_once_until_the_screen_changes() {
        use crate::{component, hud::Rect, text::ClickEvent};
        use horizon_core::nbt::Tag;
        let font = Font::load(
            &horizon_assets::resource_source::ResourceSource::default(),
            &std::collections::HashMap::new(),
            &[],
            (1, 1),
            (0, 0),
            Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 1,
                h: 1,
            },
        );
        let mut dialog = Dialog::parse(&Tag::compound([
            ("type", Tag::String("minecraft:notice".into())),
            ("title", Tag::String("Many lines".into())),
        ]))
        .unwrap();
        let id = |line: usize, letter: usize| format!("test:{line}/{letter}");
        dialog.body = (0..50)
            .map(|line| {
                let letters = format!("Line {line}: click me!")
                    .chars()
                    .enumerate()
                    .map(|(letter, c)| {
                        serde_json::json!({"text": c.to_string(), "click_event": {
                            "action": "custom", "id": id(line, letter)
                        }})
                    })
                    .collect();
                Body::Message {
                    contents: component::to_text(&serde_json::Value::Array(letters), &mut |_| {
                        Text::empty()
                    }),
                    width: 300,
                }
            })
            .collect();
        let cache = LayoutCache::default();
        let layout = cache.get(&dialog, &font, (640.0, 480.0));
        assert!(
            Arc::ptr_eq(&layout, &cache.get(&dialog, &font, (640.0, 480.0))),
            "the next frame keeps the layout"
        );
        let lines: Vec<_> = layout
            .body
            .iter()
            .filter_map(|p| match p {
                Part::Line(text, x, y) => Some((text, *x, *y)),
                _ => None,
            })
            .collect();
        assert_eq!(lines.len(), 50);
        for (line, (text, x, y)) in lines.into_iter().enumerate() {
            // Every glyph is the 6 wide missing one.
            for letter in 0..text.chars().count() {
                let at = (x + 6.0 * letter as f32 + 3.0, y + 4.0);
                let click = line_style_at(&font, text, x, y, at, |s| s.click.is_some())
                    .and_then(|s| s.click);
                let own = ClickEvent::Custom {
                    id: id(line, letter),
                    payload: None,
                };
                assert_eq!(click, Some(own), "line {line}, letter {letter}");
            }
        }
        let resized = cache.get(&dialog, &font, (320.0, 240.0));
        assert!(
            !Arc::ptr_eq(&layout, &resized),
            "a resize lays it out again"
        );
        assert_eq!(
            resized.area,
            Layout::new(&dialog, &font, (320.0, 240.0)).area
        );
    }

    /// A font with no glyphs: every character is the missing one.
    fn glyphless_font() -> Font {
        Font::load(
            &horizon_assets::resource_source::ResourceSource::default(),
            &std::collections::HashMap::new(),
            &[],
            (1, 1),
            (0, 0),
            crate::hud::Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 1,
                h: 1,
            },
        )
    }

    /// A notice whose body is one message.
    fn notice() -> Dialog {
        use horizon_core::nbt::Tag;
        let mut dialog = Dialog::parse(&Tag::compound([
            ("type", Tag::String("minecraft:notice".into())),
            ("title", Tag::String("Notice".into())),
        ]))
        .unwrap();
        dialog.body = vec![Body::Message {
            contents: Text::literal("A line of text"),
            width: 200,
        }];
        dialog
    }

    /// Whether two layouts put everything in the same place.
    fn same(a: &Layout, b: &Layout) -> bool {
        (a.title, a.warning, a.area, a.content, &a.body, &a.footer)
            == (b.title, b.warning, b.area, b.content, &b.body, &b.footer)
    }

    /// #19102: the kept layout goes when the font's glyphs may come out
    /// different (here a glyph cache reset, as a full glyph region at a frame's
    /// start or other font options do), and a frame that resets nothing keeps it.
    #[test]
    fn a_dialog_is_laid_out_again_when_the_font_changes() {
        let (font, dialog, cache) = (glyphless_font(), notice(), LayoutCache::default());
        let before = cache.get(&dialog, &font, (640.0, 480.0));
        font.begin_frame();
        assert!(
            Arc::ptr_eq(&before, &cache.get(&dialog, &font, (640.0, 480.0))),
            "a frame that resets no glyphs keeps the layout"
        );
        font.reset_cache();
        let after = cache.get(&dialog, &font, (640.0, 480.0));
        assert!(
            !Arc::ptr_eq(&before, &after),
            "a glyph reset lays it out again"
        );
        assert!(same(&after, &Layout::new(&dialog, &font, (640.0, 480.0))));
        assert!(
            Arc::ptr_eq(&after, &cache.get(&dialog, &font, (640.0, 480.0))),
            "and keeps the new layout"
        );
    }

    /// #19102: the kept layout goes when the language's text direction
    /// changes, either way.
    #[test]
    fn a_dialog_is_laid_out_again_when_the_text_direction_changes() {
        use crate::lang::{self, Language};
        /// A pack declaring one right-to-left language.
        struct RightToLeft;
        impl lang::ResourceSource for RightToLeft {
            fn read(&self, path: &str) -> Option<Vec<u8>> {
                match path {
                    "pack.mcmeta" => {
                        Some(br#"{"language":{"ar_sa":{"bidirectional":true}}}"#.to_vec())
                    }
                    "assets/minecraft/lang/ar_sa.json" => Some(b"{}".to_vec()),
                    _ => None,
                }
            }
        }
        let (font, dialog, cache) = (glyphless_font(), notice(), LayoutCache::default());
        // Languages only this thread sees, put back when dropped, so the tests
        // beside it keep the process-wide one.
        let _left_to_right = lang::isolate_publication(Language::of("en_us", []));
        assert!(!lang::bidirectional());
        let left = cache.get(&dialog, &font, (640.0, 480.0));
        let right = {
            let _right_to_left =
                lang::isolate_publication(Language::load(&[&RightToLeft], "ar_sa"));
            assert!(lang::bidirectional(), "ar_sa reads right to left");
            let right = cache.get(&dialog, &font, (640.0, 480.0));
            assert!(
                !Arc::ptr_eq(&left, &right),
                "a right-to-left language lays it out again"
            );
            assert!(same(&right, &Layout::new(&dialog, &font, (640.0, 480.0))));
            assert!(
                Arc::ptr_eq(&right, &cache.get(&dialog, &font, (640.0, 480.0))),
                "and keeps the new layout"
            );
            right
        };
        assert!(!lang::bidirectional());
        let back = cache.get(&dialog, &font, (640.0, 480.0));
        assert!(
            !Arc::ptr_eq(&right, &back),
            "back to left to right lays it out again"
        );
        assert!(same(&back, &Layout::new(&dialog, &font, (640.0, 480.0))));
    }

    #[test]
    fn grids_pack_full_rows_and_centre_the_rest() {
        // AuthMe's two buttons: one row, 2 apart.
        let two = columns(&[150.0, 150.0], 2, 0);
        assert_eq!((two.w, two.h), (302.0, 20.0));
        // Three in two columns: the third alone, centred under them.
        let three = columns(&[150.0, 150.0, 150.0], 2, 0);
        assert_eq!((three.w, three.h), (302.0, 42.0));
        assert_eq!(
            three.parts[2],
            Part::Button {
                index: 2,
                rect: [76.0, 22.0, 150.0, 20.0]
            }
        );
        // One in two columns spreads over both.
        let one = columns(&[100.0], 2, 0);
        assert_eq!(one.w, 100.0);
    }

    #[test]
    fn a_multiline_input_is_as_tall_as_its_lines_or_its_height() {
        assert_eq!(text_height(None), 20.0);
        // No options: four lines and the padding.
        assert_eq!(text_height(Some((None, None))), 44.0);
        assert_eq!(text_height(Some((Some(10), None))), 98.0);
        // Never over 512; a height wins over the lines.
        assert_eq!(text_height(Some((Some(100), None))), 512.0);
        assert_eq!(text_height(Some((Some(2), Some(70)))), 70.0);
    }

    #[test]
    fn the_waiting_screens_button_counts_down() {
        assert!(!waiting_button(0).1);
        assert_eq!(
            waiting_button(20).0,
            Text::translate(
                "gui.waitingForResponse.button.inactive",
                &[Text::literal("4")]
            )
        );
        assert_eq!(
            waiting_button(100),
            (Text::translate("gui.back", &[]), true, true)
        );
    }
}
