//! Styled text (a flattened `Component`): spans with ChatFormatting colours
//! and decorations, translatable messages formatted like
//! `TranslatableContents` (`%s`, `%1$s`, `%%`), and legacy `§` codes.
use crate::lang;

/// ChatFormatting colours by legacy code index 0..=15.
pub const COLORS: [(char, &str, u32); 16] = [
    ('0', "black", 0x000000),
    ('1', "dark_blue", 0x0000AA),
    ('2', "dark_green", 0x00AA00),
    ('3', "dark_aqua", 0x00AAAA),
    ('4', "dark_red", 0xAA0000),
    ('5', "dark_purple", 0xAA00AA),
    ('6', "gold", 0xFFAA00),
    ('7', "gray", 0xAAAAAA),
    ('8', "dark_gray", 0x555555),
    ('9', "blue", 0x5555FF),
    ('a', "green", 0x55FF55),
    ('b', "aqua", 0x55FFFF),
    ('c', "red", 0xFF5555),
    ('d', "light_purple", 0xFF55FF),
    ('e', "yellow", 0xFFFF55),
    ('f', "white", 0xFFFFFF),
];
pub const BLACK: u32 = 0x000000;
pub const GRAY: u32 = 0xAAAAAA;
pub const DARK_GRAY: u32 = 0x555555;
pub const GREEN: u32 = 0x55FF55;
pub const AQUA: u32 = 0x55FFFF;
pub const RED: u32 = 0xFF5555;
pub const YELLOW: u32 = 0xFFFF55;
pub const GOLD: u32 = 0xFFAA00;
pub const WHITE: u32 = 0xFFFFFF;
pub const LIGHT_PURPLE: u32 = 0xFF55FF;

pub fn named_color(name: &str) -> Option<u32> {
    COLORS.iter().find(|c| c.1 == name).map(|c| c.2)
}

/// Text style; `None` fields inherit from the enclosing style.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub color: Option<u32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underlined: Option<bool>,
    pub strikethrough: Option<bool>,
    pub obfuscated: Option<bool>,
    /// ClickEvent.SuggestCommand: clicking puts this into the chat box.
    pub suggest: Option<String>,
    /// Hover text.
    pub hover: Option<Box<Text>>,
}
impl Style {
    pub fn color(c: u32) -> Self {
        Self {
            color: Some(c),
            ..Self::default()
        }
    }
    /// Fill unset fields from `base` (Style.applyTo).
    pub fn inherit(&mut self, base: &Style) {
        self.color = self.color.or(base.color);
        self.bold = self.bold.or(base.bold);
        self.italic = self.italic.or(base.italic);
        self.underlined = self.underlined.or(base.underlined);
        self.strikethrough = self.strikethrough.or(base.strikethrough);
        self.obfuscated = self.obfuscated.or(base.obfuscated);
        if self.suggest.is_none() {
            self.suggest.clone_from(&base.suggest);
        }
        if self.hover.is_none() {
            self.hover.clone_from(&base.hover);
        }
    }
    pub fn is_bold(&self) -> bool {
        self.bold == Some(true)
    }
    pub fn is_italic(&self) -> bool {
        self.italic == Some(true)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}
/// A run of styled spans.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text {
    pub spans: Vec<Span>,
}
impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Text::literal(s)
    }
}
impl From<String> for Text {
    fn from(s: String) -> Self {
        Text::literal(s)
    }
}
impl Text {
    pub fn empty() -> Self {
        Self::default()
    }
    pub fn literal(s: impl Into<String>) -> Self {
        let text = s.into();
        if text.is_empty() {
            return Self::default();
        }
        Self {
            spans: vec![Span {
                text,
                style: Style::default(),
            }],
        }
    }
    pub fn styled(s: impl Into<String>, style: Style) -> Self {
        let mut t = Self::literal(s);
        for span in &mut t.spans {
            span.style = style.clone();
        }
        t
    }
    pub fn colored(s: impl Into<String>, color: u32) -> Self {
        Self::styled(s, Style::color(color))
    }
    /// Component.translatable(key, args) in the current language.
    pub fn translate(key: &str, args: &[Text]) -> Self {
        format_translation(&lang::get(key), args)
    }
    /// A translatable message whose arguments are plain strings.
    pub fn tr(key: &str, args: &[&dyn std::fmt::Display]) -> Self {
        let args: Vec<Text> = args.iter().map(|a| Text::literal(a.to_string())).collect();
        Self::translate(key, &args)
    }
    pub fn append(mut self, other: impl Into<Text>) -> Self {
        self.spans.extend(other.into().spans);
        self
    }
    pub fn push(&mut self, other: impl Into<Text>) {
        self.spans.extend(other.into().spans);
    }
    /// Apply `style` underneath every span (MutableComponent.withStyle on a parent).
    pub fn with_style(mut self, style: &Style) -> Self {
        for span in &mut self.spans {
            span.style.inherit(style);
        }
        self
    }
    pub fn with_color(self, color: u32) -> Self {
        self.with_style(&Style::color(color))
    }
    pub fn italic(self) -> Self {
        self.with_style(&Style {
            italic: Some(true),
            ..Style::default()
        })
    }
    pub fn underlined(self) -> Self {
        self.with_style(&Style {
            underlined: Some(true),
            ..Style::default()
        })
    }
    pub fn bold(self) -> Self {
        self.with_style(&Style {
            bold: Some(true),
            ..Style::default()
        })
    }
    /// Component.getString: the plain characters.
    pub fn plain(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
    pub fn is_empty(&self) -> bool {
        self.spans.iter().all(|s| s.text.is_empty())
    }
    /// Parse legacy formatting codes (`§a`, `§l`, `§r`...) as the font does.
    pub fn legacy(s: &str) -> Self {
        let mut out = Text::default();
        let mut style = Style::default();
        let mut current = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c != '§' {
                current.push(c);
                continue;
            }
            let Some(code) = chars.next() else { break };
            if !current.is_empty() {
                out.spans.push(Span {
                    text: std::mem::take(&mut current),
                    style: style.clone(),
                });
            }
            let code = code.to_ascii_lowercase();
            if let Some(c) = COLORS.iter().find(|c| c.0 == code) {
                // Colour codes reset decorations (Style.applyLegacyFormat).
                style = Style::color(c.2);
            } else {
                match code {
                    'k' => style.obfuscated = Some(true),
                    'l' => style.bold = Some(true),
                    'm' => style.strikethrough = Some(true),
                    'n' => style.underlined = Some(true),
                    'o' => style.italic = Some(true),
                    'r' => style = Style::default(),
                    _ => {}
                }
            }
        }
        if !current.is_empty() {
            out.spans.push(Span {
                text: current,
                style,
            });
        }
        out
    }
    /// Characters with their effective styles.
    pub fn chars(&self) -> impl Iterator<Item = (char, &Style)> {
        self.spans
            .iter()
            .flat_map(|s| s.text.chars().map(move |c| (c, &s.style)))
    }
    /// Sub-range of characters [from, to) keeping styles.
    pub fn slice(&self, from: usize, to: usize) -> Text {
        let mut out = Text::default();
        let mut index = 0;
        for span in &self.spans {
            let mut part = String::new();
            for c in span.text.chars() {
                if index >= from && index < to {
                    part.push(c);
                }
                index += 1;
            }
            if !part.is_empty() {
                out.spans.push(Span {
                    text: part,
                    style: span.style.clone(),
                });
            }
        }
        out
    }
    pub fn char_count(&self) -> usize {
        self.spans.iter().map(|s| s.text.chars().count()).sum()
    }
}

/// TranslatableContents.decomposeTemplate: `%s` takes the next argument,
/// `%n$s` a numbered one and `%%` is a percent sign. Malformed templates are
/// shown verbatim.
pub fn format_translation(template: &str, args: &[Text]) -> Text {
    let mut out = Text::default();
    let mut literal = String::new();
    let mut next = 0usize;
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' {
            literal.push(chars[i]);
            i += 1;
            continue;
        }
        // %(?:(\d+)\$)?([A-Za-z%]|$)
        let mut j = i + 1;
        let mut number = None;
        let digits_start = j;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
        if j > digits_start && j < chars.len() && chars[j] == '$' {
            number = chars[digits_start..j]
                .iter()
                .collect::<String>()
                .parse::<usize>()
                .ok();
            j += 1;
        } else {
            j = digits_start;
        }
        let conversion = chars.get(j).copied();
        match conversion {
            Some('%') if number.is_none() => {
                literal.push('%');
                i = j + 1;
            }
            // Not a format specifier at all: the percent sign is literal.
            Some(c) if !c.is_ascii_alphabetic() && c != '%' => {
                literal.push('%');
                i += 1;
            }
            Some('s') => {
                let index = match number {
                    Some(n) => n.wrapping_sub(1),
                    None => {
                        next += 1;
                        next - 1
                    }
                };
                let Some(arg) = args.get(index) else {
                    return Text::literal(template);
                };
                if !literal.is_empty() {
                    out.spans.push(Span {
                        text: std::mem::take(&mut literal),
                        style: Style::default(),
                    });
                }
                out.spans.extend(arg.spans.iter().cloned());
                i = j + 1;
            }
            _ => return Text::literal(template),
        }
    }
    if !literal.is_empty() {
        out.spans.push(Span {
            text: literal,
            style: Style::default(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formats_like_translatable_contents() {
        let a = [Text::literal("Steve"), Text::colored("Zombie", RED)];
        assert_eq!(
            format_translation("%s was slain by %s", &a).plain(),
            "Steve was slain by Zombie"
        );
        assert_eq!(
            format_translation("%2$s then %1$s", &a).plain(),
            "Zombie then Steve"
        );
        assert_eq!(
            format_translation("100%% sure %s", &a).plain(),
            "100% sure Steve"
        );
        // Missing arguments or unknown conversions show the template.
        assert_eq!(format_translation("%s %s %s", &a).plain(), "%s %s %s");
        assert_eq!(format_translation("%d", &a).plain(), "%d");
        assert_eq!(
            format_translation("50% off %s", &a).plain(),
            "50% off Steve"
        );
        assert_eq!(format_translation("end %", &a).plain(), "end %");
        let t = format_translation("%s was slain by %s", &a);
        assert_eq!(t.spans.last().unwrap().style.color, Some(RED));
    }
    #[test]
    fn legacy_codes_and_inheritance() {
        let t = Text::legacy("§aGreen §lbold§r plain");
        assert_eq!(t.plain(), "Green bold plain");
        assert_eq!(t.spans[0].style.color, Some(GREEN));
        assert!(t.spans[1].style.is_bold());
        assert_eq!(t.spans[2].style, Style::default());
        let t = Text::literal("x")
            .append(Text::colored("y", GOLD))
            .with_color(RED);
        assert_eq!(t.spans[0].style.color, Some(RED));
        assert_eq!(t.spans[1].style.color, Some(GOLD));
        assert_eq!(t.slice(1, 2).plain(), "y");
    }
}
