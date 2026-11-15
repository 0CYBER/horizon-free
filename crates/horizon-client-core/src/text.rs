//! Styled text (a flattened `Component`): spans with ChatFormatting colours
//! and decorations, translatable messages formatted like
//! `TranslatableContents` (`%s`, `%1$s`, `%%`), and legacy `§` codes.
use crate::lang;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

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
pub const DARK_PURPLE: u32 = 0xAA00AA;

pub fn named_color(name: &str) -> Option<u32> {
    COLORS.iter().find(|c| c.1 == name).map(|c| c.2)
}

/// Click actions carried by text components.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClickEvent {
    /// `ClickEvent.OpenUrl`: an http or https link.
    OpenUrl(String),
    /// `ClickEvent.ChangePage`: a book's page, counted from 1.
    ChangePage(i32),
    SuggestCommand(String),
    /// `ClickEvent.RunCommand`: the command, with or without its `/`.
    RunCommand(String),
    CopyToClipboard(String),
    /// `ClickEvent.ShowDialog`: a `minecraft:dialog` registry id, or the
    /// dialog itself as the component carried it.
    ShowDialog(std::sync::Arc<horizon_core::nbt::Tag>),
    /// `ClickEvent.Custom`: a server-defined action with an optional NBT payload.
    Custom {
        id: String,
        payload: Option<std::sync::Arc<horizon_core::nbt::Tag>>,
    },
    /// A local file to open with the system handler.
    /// Only allow this in locally constructed messages, never server/LAN text.
    OpenFile(String),
    /// A `/profiler` line the client's own buttons run; never decoded from a server's text, so only they reach the local profiler.
    Profiler(String),
}

/// A non-character `minecraft:object` embedded in a text component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineObject {
    Player {
        profile: std::sync::Arc<Value>,
        hat: bool,
        fallback: String,
    },
    Atlas {
        atlas: String,
        sprite: String,
        fallback: String,
    },
}
impl InlineObject {
    pub fn fallback(&self) -> &str {
        match self {
            Self::Player { fallback, .. } | Self::Atlas { fallback, .. } => fallback,
        }
    }
}

/// Text style; `None` fields inherit from the enclosing style.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub color: Option<u32>,
    /// Text component `shadow_color` as packed ARGB. Zero suppresses shadows.
    pub shadow_color: Option<u32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underlined: Option<bool>,
    pub strikethrough: Option<bool>,
    pub obfuscated: Option<bool>,
    pub click: Option<ClickEvent>,
    /// Hover text.
    pub hover: Option<std::sync::Arc<Text>>,
    /// `HoverEvent.ShowItem`: the event's `{id, count?, components?}` object.
    pub hover_item: Option<std::sync::Arc<Value>>,
    /// `HoverEvent.ShowEntity`: the event's `{id, uuid, name?}` object.
    pub hover_entity: Option<std::sync::Arc<Value>>,
    /// `Style.insertion`: the text a Shift-click puts in the chat box.
    pub insertion: Option<String>,
    /// The font (`FontDescription.Resource`), a full id like
    /// `minecraft:alt`; the default font when unset.
    pub font: Option<String>,
    /// Content metadata for a single U+FFFC object span; never inherited.
    pub inline_object: Option<InlineObject>,
}
/// A style's `font` field: an identifier (`alt` is `minecraft:alt`).
pub fn font_id(v: &Value) -> Option<String> {
    crate::identifier::parse_identifier(v.as_str()?)
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
        self.shadow_color = self.shadow_color.or(base.shadow_color);
        self.bold = self.bold.or(base.bold);
        self.italic = self.italic.or(base.italic);
        self.underlined = self.underlined.or(base.underlined);
        self.strikethrough = self.strikethrough.or(base.strikethrough);
        self.obfuscated = self.obfuscated.or(base.obfuscated);
        if self.click.is_none() {
            self.click.clone_from(&base.click);
        }
        // One HoverEvent: its three kinds are inherited together, so a style
        // with its own event never picks up another kind from `base`.
        if self.hover.is_none() && self.hover_item.is_none() && self.hover_entity.is_none() {
            self.hover.clone_from(&base.hover);
            self.hover_item.clone_from(&base.hover_item);
            self.hover_entity.clone_from(&base.hover_entity);
        }
        if self.insertion.is_none() {
            self.insertion.clone_from(&base.insertion);
        }
        if self.font.is_none() {
            self.font.clone_from(&base.font);
        }
    }
    pub fn is_bold(&self) -> bool {
        self.bold == Some(true)
    }
    pub fn is_italic(&self) -> bool {
        self.italic == Some(true)
    }
}
// A builder may produce no spans for a styled empty component. Release its
// unretained hover data explicitly rather than recursively dropping a deep chain.
pub(crate) fn discard_style(style: Style) {
    discard(Discard::Style(style));
}
pub(crate) fn discard_text(text: Text) {
    discard(Discard::Text(text));
}
enum Discard {
    Style(Style),
    Text(Text),
}
fn discard(root: Discard) {
    let mut pending = vec![root];
    while let Some(value) = pending.pop() {
        match value {
            Discard::Text(text) => {
                if let Some(source) = text.source
                    && let Ok(source) = std::sync::Arc::try_unwrap(source)
                {
                    crate::component::discard_component(source);
                }
                pending.extend(
                    text.rendered
                        .spans
                        .into_iter()
                        .map(|span| Discard::Style(span.style)),
                );
            }
            Discard::Style(mut style) => {
                if let Some(
                    ClickEvent::ShowDialog(tag)
                    | ClickEvent::Custom {
                        payload: Some(tag), ..
                    },
                ) = style.click.take()
                    && let Ok(tag) = std::sync::Arc::try_unwrap(tag)
                {
                    horizon_core::nbt::discard_tag(tag);
                }
                if let Some(hover) = style.hover.take()
                    && let Ok(text) = std::sync::Arc::try_unwrap(hover)
                {
                    pending.push(Discard::Text(text));
                }
                for value in [style.hover_item.take(), style.hover_entity.take()]
                    .into_iter()
                    .flatten()
                {
                    if let Ok(value) = std::sync::Arc::try_unwrap(value) {
                        crate::component::discard_component(value);
                    }
                }
                if let Some(InlineObject::Player { profile, .. }) = style.inline_object.take()
                    && let Ok(profile) = std::sync::Arc::try_unwrap(profile)
                {
                    crate::component::discard_component(profile);
                }
            }
        }
    }
}

/// Provenance is independent of visual style and is never serialized.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Origin {
    #[default]
    Remote,
    Local,
}

#[derive(Clone, Debug)]
pub struct Span {
    pub(crate) origin: Origin,
    pub text: String,
    pub style: Style,
}
impl PartialEq for Span {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.style == other.style
    }
}
impl Eq for Span {}
impl Span {
    /// A manually supplied span is untrusted. Cloning an existing span preserves provenance.
    pub fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
            origin: Origin::Remote,
        }
    }
    pub fn origin(&self) -> Origin {
        self.origin
    }
    pub fn with_text(&self, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: self.style.clone(),
            origin: self.origin,
        }
    }
    pub(crate) fn inherit_style(&mut self, base: &Style) {
        self.style.inherit(base);
        self.enforce_origin();
    }
    fn enforce_origin(&mut self) {
        if self.click_event().is_none() {
            self.style.click = None;
        }
    }
    /// The action that may be executed for this rendered node.
    pub fn click_event(&self) -> Option<&ClickEvent> {
        match &self.style.click {
            Some(ClickEvent::OpenFile(_)) if self.origin != Origin::Local => None,
            event => event.as_ref(),
        }
    }
}
/// Read-only rendered cache. Text deliberately has no DerefMut.
#[derive(Clone, Debug, Default)]
pub struct RenderedText {
    pub spans: Vec<Span>,
}

/// An immutable component tree with per-node provenance and its rendered cache.
/// Structured nodes retain their JSON and a private provenance index; flattened
/// literal leaves use the spans themselves as their canonical tree. Both export
/// through component(), which intentionally omits provenance. Equality is visual.
/// Deref exposes only a shared view, so callers cannot invalidate the cache.
#[derive(Clone, Debug)]
pub struct Text {
    pub(crate) rendered: RenderedText,
    // JSON pointers identify nodes in the canonical source. Missing entries are remote.
    origins: std::sync::Arc<BTreeMap<String, Origin>>,
    source: Option<std::sync::Arc<Value>>,
}
// Uniform origins are shared singletons: plain text and layout leaves must not
// allocate a provenance record per node (or even per line).
fn remote_origins() -> std::sync::Arc<BTreeMap<String, Origin>> {
    static ORIGINS: std::sync::LazyLock<std::sync::Arc<BTreeMap<String, Origin>>> =
        std::sync::LazyLock::new(|| std::sync::Arc::new(BTreeMap::new()));
    ORIGINS.clone()
}
fn local_origins() -> std::sync::Arc<BTreeMap<String, Origin>> {
    static ORIGINS: std::sync::LazyLock<std::sync::Arc<BTreeMap<String, Origin>>> =
        std::sync::LazyLock::new(|| std::sync::Arc::new([(String::new(), Origin::Local)].into()));
    ORIGINS.clone()
}
impl Default for Text {
    fn default() -> Self {
        Self {
            rendered: RenderedText::default(),
            origins: remote_origins(),
            source: None,
        }
    }
}
impl std::ops::Deref for Text {
    type Target = RenderedText;
    fn deref(&self) -> &RenderedText {
        &self.rendered
    }
}
impl PartialEq for Text {
    fn eq(&self, other: &Self) -> bool {
        self.rendered.spans == other.rendered.spans
    }
}
impl Eq for Text {}
impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Text::untrusted(s)
    }
}
impl From<String> for Text {
    fn from(s: String) -> Self {
        Text::untrusted(s)
    }
}
impl Text {
    pub fn empty() -> Self {
        Self::default()
    }
    pub fn literal(s: impl Into<String>) -> Self {
        let text = s.into();
        if text.is_empty() {
            return Self {
                source: Some(json!({"text":""}).into()),
                origins: local_origins(),
                ..Self::default()
            };
        }
        Self {
            source: Some(json!({ "text": text }).into()),
            origins: local_origins(),
            rendered: RenderedText {
                spans: vec![Span {
                    origin: Origin::Local,
                    text,
                    style: Style::default(),
                }],
            },
        }
    }
    /// Plain decoded/user supplied text. No component or serialized trust metadata
    /// is interpreted; useful for plain network fields and per-frame sign labels.
    pub fn untrusted(s: impl Into<String>) -> Self {
        let text = s.into();
        if text.is_empty() {
            return Self::default();
        }
        Self::from_spans(vec![Span::new(text, Style::default())])
    }
    /// Decode legacy formatting while keeping every resulting node remote.
    pub fn untrusted_legacy(s: &str) -> Self {
        let mut text = Self::legacy(s);
        for span in &mut text.rendered.spans {
            span.origin = Origin::Remote;
        }
        text
    }
    pub fn styled(s: impl Into<String>, style: Style) -> Self {
        // `inherit` never passes an object on, but a styled span's own style carries it.
        Self::literal(s).with_style(&style).map_styles(|own| {
            own.inline_object.clone_from(&style.inline_object);
        })
    }
    pub fn colored(s: impl Into<String>, color: u32) -> Self {
        Self::styled(s, Style::color(color))
    }
    /// ComponentUtils.copyOnClickText: a green, bracketed clipboard link.
    pub fn copyable(s: impl Into<String>) -> Self {
        let s = s.into();
        Self::literal("[")
            .append(Self::styled(
                s.clone(),
                Style {
                    click: Some(ClickEvent::CopyToClipboard(s)),
                    hover: Some(std::sync::Arc::new(Self::translate("chat.copy.click", &[]))),
                    ..Style::color(GREEN)
                },
            ))
            .append("]")
    }
    /// LocateCommand's coordinate link: suggest a teleport without executing it.
    /// Structure and POI results preserve the player's current height (`~`).
    pub fn teleport_link(x: i32, y: Option<i32>, z: i32) -> Self {
        let y = y.map_or_else(|| "~".to_owned(), |y| y.to_string());
        let coords = Self::tr("chat.coordinates", &[&x, &y, &z]);
        Self::literal("[")
            .append(coords)
            .append("]")
            .with_style(&Style {
                click: Some(ClickEvent::SuggestCommand(format!("/tp @s {x} {y} {z}"))),
                hover: Some(std::sync::Arc::new(Self::translate(
                    "chat.coordinates.tooltip",
                    &[],
                ))),
                ..Style::color(GREEN)
            })
    }
    /// Component.translatable(key, args) in the current language.
    pub fn translate(key: &str, args: &[Text]) -> Self {
        let mut t = format_translatable(key, None, args);
        let mut c = Map::new();
        c.insert("translate".into(), key.into());
        if !args.is_empty() {
            c.insert("with".into(), args.iter().map(Text::argument).collect());
        }
        t.origins = if args.is_empty() {
            local_origins()
        } else {
            let mut origins = BTreeMap::from([(String::new(), Origin::Local)]);
            for (i, arg) in args.iter().enumerate() {
                arg.copy_origins(&format!("/with/{i}"), &mut origins);
            }
            origins.into()
        };
        t.source = Some(Value::Object(c).into());
        t
    }
    /// Component.translatableWithFallback(key, fallback, args): `translate`, with `fallback` as
    /// the template when the language has no `key`.
    pub fn translate_with_fallback(key: &str, fallback: &str, args: &[Text]) -> Self {
        let mut t = format_translatable(key, Some(fallback), args);
        let mut c = Map::new();
        c.insert("translate".into(), key.into());
        c.insert("fallback".into(), fallback.into());
        if !args.is_empty() {
            c.insert("with".into(), args.iter().map(Text::argument).collect());
        }
        t.origins = if args.is_empty() {
            local_origins()
        } else {
            let mut origins = BTreeMap::from([(String::new(), Origin::Local)]);
            for (i, arg) in args.iter().enumerate() {
                arg.copy_origins(&format!("/with/{i}"), &mut origins);
            }
            origins.into()
        };
        t.source = Some(Value::Object(c).into());
        t
    }
    /// CommonComponents.optionNameValue: "Caption: value" joined as the language joins them
    /// (`options.generic_value`: "%s : %s" in French, "%s：%s" in Chinese). English's "%s: %s"
    /// when the language has no such key, as without prepared resources.
    pub fn option_name_value(caption: Text, value: Text) -> Self {
        Self::translate_with_fallback("options.generic_value", "%s: %s", &[caption, value])
    }
    /// A translatable message whose arguments are plain strings.
    pub fn tr(key: &str, args: &[&dyn std::fmt::Display]) -> Self {
        let args: Vec<Text> = args.iter().map(|a| Text::literal(a.to_string())).collect();
        Self::translate(key, &args)
    }
    pub fn append(mut self, other: impl Into<Text>) -> Self {
        self.push(other);
        self
    }
    /// MutableComponent.append: `other` becomes a sibling. A styled text is
    /// wrapped first, as its style doesn't reach what is appended here.
    pub fn push(&mut self, other: impl Into<Text>) {
        let mut other = other.into();
        let sibling = other.component();
        let mut origins = if self.source.is_some() {
            std::sync::Arc::try_unwrap(std::mem::take(&mut self.origins))
                .unwrap_or_else(|origins| (*origins).clone())
        } else {
            let mut origins = BTreeMap::new();
            self.copy_origins("", &mut origins);
            origins
        };
        let root = match self.source.take() {
            None if self.rendered.spans.is_empty() => std::sync::Arc::new(json!({ "text": "" })),
            None => std::sync::Arc::new(self.component()),
            Some(c) => c,
        };
        let mut root = if !root.is_object()
            || STYLE_KEYS
                .iter()
                .chain(["clickEvent", "hoverEvent"].iter())
                .any(|k| root.get(*k).is_some())
        {
            origins = origins
                .into_iter()
                .map(|(p, o)| (format!("/extra/0{p}"), o))
                .collect();
            let c = std::sync::Arc::try_unwrap(root)
                .unwrap_or_else(|c| crate::component::clone_value(&c));
            std::sync::Arc::new(json!({ "text": "", "extra": [c] }))
        } else {
            root
        };
        // Shared trees may be deeply nested mod input: clone iteratively, not through Value's
        // recursive Clone implementation inside Arc::make_mut.
        if std::sync::Arc::get_mut(&mut root).is_none() {
            root = std::sync::Arc::new(crate::component::clone_value(&root));
        }
        if let Some(Value::Object(o)) = std::sync::Arc::get_mut(&mut root) {
            match o.get_mut("extra") {
                Some(Value::Array(a)) => {
                    other.copy_origins(&format!("/extra/{}", a.len()), &mut origins);
                    a.push(sibling);
                }
                _ => {
                    other.copy_origins("/extra/0", &mut origins);
                    o.insert("extra".into(), Value::Array(vec![sibling]));
                }
            }
        }
        self.origins = origins.into();
        self.source = Some(root);
        self.rendered.spans.append(&mut other.rendered.spans);
        discard_text(other);
    }
    fn replace_component(&mut self, component: Value) {
        let old = self.source.replace(component.into());
        if let Some(old) = old
            && let Ok(old) = std::sync::Arc::try_unwrap(old)
        {
            crate::component::discard_component(old);
        }
    }
    /// Apply `style` underneath every span (MutableComponent.withStyle on a parent).
    pub fn with_style(mut self, style: &Style) -> Self {
        if self.source.is_none() {
            let mut origins = BTreeMap::new();
            self.copy_origins("", &mut origins);
            self.origins = origins.into();
        }
        let mut c = self.component();
        if !c.is_object() {
            let mut origins = BTreeMap::new();
            self.copy_origins("/extra/0", &mut origins);
            self.origins = origins.into();
            c = json!({"text":"", "extra":[c]});
        }
        let adds_hover = c.get("hover_event").is_none() && c.get("hoverEvent").is_none();
        if let Value::Object(o) = &mut c {
            inherit_component_style(o, style);
        }
        if adds_hover && let Some(hover) = &style.hover {
            let mut origins = (*self.origins).clone();
            hover.copy_origins("/hover_event/value", &mut origins);
            self.origins = origins.into();
        }
        self.replace_component(c);
        // The cached spans take the style underneath their own, as a fresh render of the
        // component above would give them. Rendering it again instead would translate existing
        // content anew, in whatever language is current now (arismoko/horizon#2493).
        for span in &mut self.rendered.spans {
            span.inherit_style(style);
            if style.inline_object.is_some() {
                span.style.inline_object.clone_from(&style.inline_object);
            }
        }
        self
    }
    /// The `Component` as vanilla's JSON (ComponentSerialization): the one
    /// built through the constructors, else the spans as literal siblings.
    /// The key of a `Component.translatable` built through [`Text::translate`].
    pub fn translation_key(&self) -> Option<&str> {
        self.source.as_deref()?.get("translate")?.as_str()
    }
    /// The retained source, if present, without materializing span-only text.
    pub fn source_component(&self) -> Option<&Value> {
        self.source.as_deref()
    }
    /// Borrow retained structured source without recursively cloning it.
    /// Span-only text is materialized once because it has no retained source.
    pub fn component_view(&self) -> std::borrow::Cow<'_, Value> {
        match &self.source {
            Some(source) => std::borrow::Cow::Borrowed(source),
            None => std::borrow::Cow::Owned(self.component()),
        }
    }
    pub fn component(&self) -> Value {
        let mut pending = vec![(self, false)];
        let mut done = Vec::new();
        while let Some((text, ready)) = pending.pop() {
            if let Some(source) = &text.source {
                done.push(crate::component::clone_value(source));
            } else if !ready {
                pending.push((text, true));
                pending.extend(
                    text.rendered
                        .spans
                        .iter()
                        .rev()
                        .filter_map(|s| s.style.hover.as_deref())
                        .map(|hover| (hover, false)),
                );
            } else {
                let count = text
                    .rendered
                    .spans
                    .iter()
                    .filter(|s| s.style.hover.is_some())
                    .count();
                let mut hovers = done.split_off(done.len() - count).into_iter();
                let mut span = |s: &Span| {
                    let mut o = Map::new();
                    o.insert("text".into(), s.text.clone().into());
                    o.extend(style_json_with(&s.style, &mut |_| {
                        hovers.next().expect("scheduled hover")
                    }));
                    Value::Object(o)
                };
                done.push(match text.rendered.spans.as_slice() {
                    [] => json!({ "text": "" }),
                    [one] => span(one),
                    many => {
                        let mut o = Map::new();
                        o.insert("text".into(), "".into());
                        o.insert(
                            "extra".into(),
                            Value::Array(many.iter().map(span).collect()),
                        );
                        Value::Object(o)
                    }
                });
            }
        }
        done.pop().expect("scheduled component")
    }
    /// A translation argument: vanilla passes numbers as numbers, which
    /// Horizon formats into unstyled literals.
    fn argument(&self) -> Value {
        argument_component(self.component())
    }

    /// Add raw component fields and regenerate the rendered cache.
    /// Nested components supplied as JSON are always remote.
    pub fn with_component_fields(mut self, fields: Vec<(String, Value)>) -> Self {
        let mut c = self.component();
        if let Value::Object(o) = &mut c {
            for (k, v) in fields {
                o.entry(k).or_insert(v);
            }
        }
        // Raw JSON fields are decoded input, even when attached to a local root.
        std::sync::Arc::make_mut(&mut self.origins).remove("");
        self.replace_component(c);
        self.refresh();
        self
    }
    /// Replace the canonical component with decoded input. Every node is remote;
    /// serialized fields cannot grant provenance.
    pub fn with_component(self, component: Value) -> Self {
        discard_text(self);
        Self::from(component)
    }
    /// The origin of a source node, addressed by its JSON pointer.
    pub fn node_origin(&self, pointer: &str) -> Origin {
        if self.source.is_some() {
            self.origins.get(pointer).copied().unwrap_or_default()
        } else {
            // Provenance queries are boundary work, never text layout work.
            let mut origins = BTreeMap::new();
            self.copy_origins("", &mut origins);
            origins.get(pointer).copied().unwrap_or_default()
        }
    }
    fn copy_origins(&self, prefix: &str, into: &mut BTreeMap<String, Origin>) {
        let mut pending = vec![(self, prefix.to_owned())];
        while let Some((text, prefix)) = pending.pop() {
            if text.source.is_some() {
                into.extend(
                    text.origins
                        .iter()
                        .map(|(p, o)| (format!("{prefix}{p}"), *o)),
                );
            } else {
                for (i, span) in text.rendered.spans.iter().enumerate() {
                    let path = if text.rendered.spans.len() == 1 {
                        prefix.clone()
                    } else {
                        format!("{prefix}/extra/{i}")
                    };
                    into.insert(path.clone(), span.origin);
                    if let Some(hover) = &span.style.hover {
                        pending.push((hover, format!("{path}/hover_event/value")));
                    }
                }
            }
        }
    }
    fn refresh(&mut self) {
        let Some(source) = &self.source else {
            return;
        };
        let mut local = std::collections::HashSet::new();
        for (path, origin) in self.origins.iter() {
            if *origin == Origin::Local
                && let Some(node) = source.pointer(path)
            {
                local.insert(node as *const Value);
            }
        }
        let mut built = crate::component::render_source(source, &|node| {
            if local.contains(&(node as *const Value)) {
                Origin::Local
            } else {
                Origin::Remote
            }
        });
        std::mem::swap(&mut self.rendered, &mut built.rendered);
        discard_text(built);
    }
    pub(crate) fn clear_origins(&mut self) {
        self.origins = remote_origins();
    }
    pub(crate) fn inherit_built_style(&mut self, style: &Style) {
        for span in &mut self.rendered.spans {
            span.inherit_style(style);
        }
        let mut c = self.component();
        let mut wrapper_style;
        let style = if !c.is_object() {
            let mut origins = BTreeMap::new();
            self.copy_origins("/extra/0", &mut origins);
            self.origins = origins.into();
            c = json!({"text":"", "extra":[c]});
            wrapper_style = style.clone();
            wrapper_style.inline_object = None;
            &wrapper_style
        } else {
            style
        };
        let adds_hover = c.get("hover_event").is_none() && c.get("hoverEvent").is_none();
        if adds_hover && let Some(hover) = &style.hover {
            let mut origins = (*self.origins).clone();
            hover.copy_origins("/hover_event/value", &mut origins);
            self.origins = origins.into();
        }
        if let Value::Object(o) = &mut c {
            inherit_component_style(o, style);
        }
        self.replace_component(c);
    }
    pub(crate) fn retain_source(&mut self, component: Value) {
        self.replace_component(component);
    }
    pub(crate) fn retain_annotated_source(
        &mut self,
        value: &Value,
        origin: &dyn Fn(&Value) -> Origin,
    ) {
        let mut origins = BTreeMap::new();
        let mut pending = vec![(value, String::new())];
        while let Some((node, path)) = pending.pop() {
            if origin(node) == Origin::Local {
                origins.insert(path.clone(), Origin::Local);
            }
            match node {
                Value::Object(fields) => {
                    for (key, child) in fields {
                        let key = key.replace('~', "~0").replace('/', "~1");
                        pending.push((child, format!("{path}/{key}")));
                    }
                }
                Value::Array(values) => {
                    for (i, child) in values.iter().enumerate() {
                        pending.push((child, format!("{path}/{i}")));
                    }
                }
                _ => {}
            }
        }
        self.origins = origins.into();
        self.replace_component(crate::component::clone_value(value));
    }
    /// Build a flattened component from existing spans, preserving their provenance.
    /// New spans made with Span::new remain remote, including nested raw metadata.
    pub fn from_spans(spans: Vec<Span>) -> Self {
        let mut text = Self {
            rendered: RenderedText { spans },
            ..Self::default()
        };
        for span in &mut text.rendered.spans {
            span.enforce_origin();
        }
        // These are already effective literal leaf nodes. Keep that canonical flat
        // tree directly: wrapping and bidi layout need no JSON or provenance-map
        // allocation per node. component() materializes JSON only at an export.
        text
    }
    /// Consume this text as canonical literal leaves, retaining each leaf's origin.
    pub fn into_spans(mut self) -> Vec<Span> {
        let spans = std::mem::take(&mut self.rendered.spans);
        discard_text(self);
        spans
    }
    /// Edit flattened visual styles without granting any node local origin.
    pub fn map_styles(&self, mut edit: impl FnMut(&mut Style)) -> Self {
        let mut spans = self.rendered.spans.clone();
        for span in &mut spans {
            edit(&mut span.style);
        }
        Self::from_spans(spans)
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
        self.rendered
            .spans
            .iter()
            .map(|s| match &s.style.inline_object {
                Some(object) if s.text == "\u{fffc}" => object.fallback(),
                _ => s.text.as_str(),
            })
            .collect()
    }
    pub fn is_empty(&self) -> bool {
        self.rendered.spans.iter().all(|s| s.text.is_empty())
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
                out.rendered.spans.push(Span {
                    origin: Origin::Local,
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
            out.rendered.spans.push(Span {
                origin: Origin::Local,
                text: current,
                style,
            });
        }
        Self::from_spans(out.rendered.spans)
    }
    /// `StringDecomposer.iterateFormatted` on every span: legacy `§` codes
    /// inside the span contents restyle what follows them (colour codes
    /// clear the decorations, `§r` returns to the span's own style) and are
    /// not drawn.
    pub fn resolve_legacy(&self) -> Text {
        if !self.rendered.spans.iter().any(|s| s.text.contains('§')) {
            return self.clone();
        }
        let mut out = Text::default();
        for span in &self.rendered.spans {
            let mut style = span.style.clone();
            let mut current = String::new();
            let mut chars = span.text.chars();
            while let Some(c) = chars.next() {
                if c != '§' {
                    current.push(c);
                    continue;
                }
                let Some(code) = chars.next() else { break };
                let next = legacy_format(&style, code, &span.style);
                if next != style {
                    if !current.is_empty() {
                        out.rendered.spans.push(Span {
                            origin: span.origin,
                            text: std::mem::take(&mut current),
                            style: style.clone(),
                        });
                    }
                    style = next;
                }
            }
            if !current.is_empty() {
                out.rendered.spans.push(Span {
                    origin: span.origin,
                    text: current,
                    style,
                });
            }
        }
        Self::from_spans(out.rendered.spans)
    }
    /// `resolve_legacy` borrowing `self` when no span holds a `§` code (the
    /// common case).
    pub fn resolved(&self) -> std::borrow::Cow<'_, Text> {
        if self.rendered.spans.iter().any(|s| s.text.contains('§')) {
            std::borrow::Cow::Owned(self.resolve_legacy())
        } else {
            std::borrow::Cow::Borrowed(self)
        }
    }
    /// The text without its legacy `§` codes (ChatFormatting.stripFormatting:
    /// a `§` and a code character, `0`-`9`, `a`-`f`, `k`-`o` or `r`).
    pub fn strip_legacy(&self) -> Text {
        if !self.rendered.spans.iter().any(|s| s.text.contains('§')) {
            return self.clone();
        }
        let code =
            |c: char| matches!(c.to_ascii_lowercase(), '0'..='9' | 'a'..='f' | 'k'..='o' | 'r');
        let mut out = Text::default();
        for span in &self.rendered.spans {
            let mut text = String::with_capacity(span.text.len());
            let mut chars = span.text.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '§' && chars.peek().is_some_and(|&n| code(n)) {
                    chars.next();
                } else {
                    text.push(c);
                }
            }
            if !text.is_empty() {
                out.rendered.spans.push(Span {
                    origin: span.origin,
                    text,
                    style: span.style.clone(),
                });
            }
        }
        Self::from_spans(out.rendered.spans)
    }
    /// Characters with their effective styles.
    pub fn chars(&self) -> impl Iterator<Item = (char, &Style)> {
        self.rendered
            .spans
            .iter()
            .flat_map(|s| s.text.chars().map(move |c| (c, &s.style)))
    }
    /// Sub-range of characters [from, to) keeping styles.
    pub fn slice(&self, from: usize, to: usize) -> Text {
        let mut out = Text::default();
        let mut index = 0;
        for span in &self.rendered.spans {
            let mut part = String::new();
            for c in span.text.chars() {
                if index >= from && index < to {
                    part.push(c);
                }
                index += 1;
            }
            if !part.is_empty() {
                out.rendered.spans.push(Span {
                    origin: span.origin,
                    text: part,
                    style: span.style.clone(),
                });
            }
        }
        Self::from_spans(out.rendered.spans)
    }
}

/// The style fields of a component's JSON.
const STYLE_KEYS: [&str; 11] = [
    "color",
    "shadow_color",
    "bold",
    "italic",
    "underlined",
    "strikethrough",
    "obfuscated",
    "click_event",
    "hover_event",
    "insertion",
    "font",
];
// Legacy event spellings still represent an already-set style field. A style
// inherited underneath them must not insert a modern spelling that shadows it.
fn inherit_component_style(fields: &mut Map<String, Value>, style: &Style) {
    for (key, value) in style_json(style) {
        if (key == "click_event" && fields.contains_key("clickEvent"))
            || (key == "hover_event" && fields.contains_key("hoverEvent"))
        {
            continue;
        }
        fields.entry(key).or_insert(value);
    }
}
/// `Style.Serializer`: the fields a style sets.
fn style_json(style: &Style) -> Vec<(String, Value)> {
    style_json_with(style, &mut Text::component)
}
fn style_json_with(
    style: &Style,
    hover_component: &mut dyn FnMut(&Text) -> Value,
) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    if let Some(c) = style.color {
        let name = COLORS
            .iter()
            .find(|n| n.2 == c)
            .map_or_else(|| format!("#{c:06X}"), |n| n.1.to_owned());
        out.push(("color".to_owned(), name.into()));
    }
    if let Some(shadow) = style.shadow_color {
        out.push(("shadow_color".to_owned(), Value::from(shadow as i32)));
    }
    for (key, flag) in [
        ("bold", style.bold),
        ("italic", style.italic),
        ("underlined", style.underlined),
        ("strikethrough", style.strikethrough),
        ("obfuscated", style.obfuscated),
    ] {
        if let Some(f) = flag {
            out.push((key.to_owned(), f.into()));
        }
    }
    if let Some(object) = &style.inline_object {
        match object {
            InlineObject::Player {
                profile,
                hat,
                fallback,
            } => {
                out.extend([
                    ("object".into(), "player".into()),
                    ("player".into(), crate::component::clone_value(profile)),
                    ("hat".into(), (*hat).into()),
                    ("fallback".into(), fallback.clone().into()),
                ]);
            }
            InlineObject::Atlas {
                atlas,
                sprite,
                fallback,
            } => out.extend([
                ("object".into(), "atlas".into()),
                ("atlas".into(), atlas.clone().into()),
                ("sprite".into(), sprite.clone().into()),
                ("fallback".into(), fallback.clone().into()),
            ]),
        }
    }
    if let Some(click) = &style.click {
        let event = (|| {
            Some(match click {
                ClickEvent::OpenUrl(url) => json!({ "action": "open_url", "url": url }),
                ClickEvent::ChangePage(page) => json!({ "action": "change_page", "page": page }),
                ClickEvent::SuggestCommand(command) => {
                    json!({ "action": "suggest_command", "command": command })
                }
                ClickEvent::CopyToClipboard(value) => {
                    json!({ "action": "copy_to_clipboard", "value": value })
                }
                ClickEvent::RunCommand(command) => {
                    json!({ "action": "run_command", "command": command })
                }
                // ClickEvent.ShowDialog.CODEC: the dialog the server sent it with.
                ClickEvent::ShowDialog(dialog) => {
                    let mut event = json!({ "action": "show_dialog" });
                    event["dialog"] = horizon_core::json_nbt::tag_to_json(dialog).ok()?;
                    event
                }
                ClickEvent::Custom { id, payload } => {
                    let mut event = json!({ "action": "custom", "id": id });
                    if let Some(payload) = payload {
                        event["payload"] = horizon_core::json_nbt::tag_to_json(payload).ok()?;
                    }
                    event
                }
                ClickEvent::Profiler(command) => {
                    json!({ "action": "run_command", "command": command })
                }
                ClickEvent::OpenFile(path) => json!({ "action": "open_file", "path": path }),
            })
        })();
        if let Some(event) = event {
            out.push(("click_event".to_owned(), event));
        }
    }
    if let Some(hover) = &style.hover {
        out.push(("hover_event".to_owned(), {
            let mut event = Map::new();
            event.insert("action".into(), "show_text".into());
            event.insert("value".into(), hover_component(hover));
            Value::Object(event)
        }));
    } else if let Some(Value::Object(item)) = style.hover_item.as_deref() {
        let Value::Object(mut event) = crate::component::clone_object(item) else {
            unreachable!()
        };
        event.insert("action".into(), "show_item".into());
        out.push(("hover_event".to_owned(), Value::Object(event)));
    } else if let Some(Value::Object(entity)) = style.hover_entity.as_deref() {
        let Value::Object(mut event) = crate::component::clone_object(entity) else {
            unreachable!()
        };
        event.insert("action".into(), "show_entity".into());
        out.push(("hover_event".to_owned(), Value::Object(event)));
    }
    if let Some(insertion) = &style.insertion {
        out.push(("insertion".to_owned(), insertion.clone().into()));
    }
    if let Some(font) = &style.font {
        out.push(("font".to_owned(), font.clone().into()));
    }
    out
}

/// `Style.applyLegacyFormat` for the code after a `§`, `§r` going back to
/// `reset` (`StringDecomposer.iterateFormatted`); an unknown code changes nothing.
pub fn legacy_format(style: &Style, code: char, reset: &Style) -> Style {
    let code = code.to_ascii_lowercase();
    if let Some(c) = COLORS.iter().find(|c| c.0 == code) {
        // A colour clears the decorations.
        return Style {
            color: Some(c.2),
            bold: Some(false),
            italic: Some(false),
            underlined: Some(false),
            strikethrough: Some(false),
            obfuscated: Some(false),
            ..style.clone()
        };
    }
    let mut s = style.clone();
    match code {
        'k' => s.obfuscated = Some(true),
        'l' => s.bold = Some(true),
        'm' => s.strikethrough = Some(true),
        'n' => s.underlined = Some(true),
        'o' => s.italic = Some(true),
        'r' => s = reset.clone(),
        _ => {}
    }
    s
}

/// Shared lookup for constructors and rendering-only component decoding.
/// A fallback is a literal template, never another translation key.
pub(crate) fn format_translatable(key: &str, fallback: Option<&str>, args: &[Text]) -> Text {
    format_translatable_origin(key, fallback, args, Origin::Local)
}
pub(crate) fn format_translatable_origin(
    key: &str,
    fallback: Option<&str>,
    args: &[Text],
    origin: Origin,
) -> Text {
    let template = match fallback {
        Some(fallback) => lang::current()
            .and_then(|language| language.get(key).map(str::to_owned))
            .unwrap_or_else(|| fallback.to_owned()),
        None => lang::get(key),
    };
    format_translation_origin(&template, args, origin)
}
/// The most spans one translation expands to. Vanilla's `TranslatableContents.decompose` keeps
/// references to its arguments, so its cost stays linear in the input; copying them here makes
/// nested `%1$s` templates multiply (`k` references nested `D` deep: `k^D` copies), and a server's
/// `fallback` template is any text it likes.
const MAX_TRANSLATION_SPANS: usize = 1 << 15;
/// The most text one translation expands to: one long argument referenced many times multiplies
/// bytes, not spans.
const MAX_TRANSLATION_BYTES: usize = 1 << 22;

/// TranslatableContents.decomposeTemplate: `%s` takes the next argument,
/// `%n$s` a numbered one and `%%` is a percent sign. Malformed templates are
/// shown verbatim, and so is one that would expand past [`MAX_TRANSLATION_SPANS`] or
/// [`MAX_TRANSLATION_BYTES`].
pub fn format_translation(template: &str, args: &[Text]) -> Text {
    let text = format_translation_origin(template, args, Origin::Local);
    Text::from_spans(text.rendered.spans)
}
pub(crate) fn format_translation_origin(template: &str, args: &[Text], origin: Origin) -> Text {
    let mut out = Text::default();
    let mut bytes = 0usize;
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
                    return Text::from_spans(vec![Span {
                        text: template.into(),
                        style: Style::default(),
                        origin,
                    }]);
                };
                bytes = bytes
                    .saturating_add(literal.len())
                    .saturating_add(arg.rendered.spans.iter().map(|s| s.text.len()).sum());
                if out
                    .rendered
                    .spans
                    .len()
                    .saturating_add(arg.rendered.spans.len())
                    >= MAX_TRANSLATION_SPANS
                    || bytes > MAX_TRANSLATION_BYTES
                {
                    return Text::from_spans(vec![Span {
                        text: template.into(),
                        style: Style::default(),
                        origin,
                    }]);
                }
                if !literal.is_empty() {
                    out.rendered.spans.push(Span {
                        origin,
                        text: std::mem::take(&mut literal),
                        style: Style::default(),
                    });
                }
                out.rendered
                    .spans
                    .extend(arg.rendered.spans.iter().cloned());
                i = j + 1;
            }
            _ => {
                return Text::from_spans(vec![Span {
                    text: template.into(),
                    style: Style::default(),
                    origin,
                }]);
            }
        }
    }
    if !literal.is_empty() {
        out.rendered.spans.push(Span {
            origin,
            text: literal,
            style: Style::default(),
        });
    }
    out
}

/// Text as its JSON component.
impl From<&Text> for serde_json::Value {
    fn from(text: &Text) -> Self {
        text.component()
    }
}

/// Text as it crosses to a wasm mod: its JSON component, and the current language's template
/// for each key the component translates. Where no language is loaded (a wasm mod), the text
/// is rebuilt with those, so it reads exactly as the game built it.
#[derive(Clone, Debug, PartialEq)]
pub struct TextForm {
    /// The component (`Text::component`).
    pub component: Value,
    /// Each translated key with its template in the current language; empty with no language.
    pub translations: Vec<(String, String)>,
}

impl From<&Text> for TextForm {
    fn from(text: &Text) -> Self {
        fn keys<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
            match v {
                Value::Object(o) => {
                    if let Some(Value::String(key)) = o.get("translate") {
                        out.push(key);
                    }
                    o.values().for_each(|v| keys(v, out));
                }
                Value::Array(items) => items.iter().for_each(|v| keys(v, out)),
                _ => {}
            }
        }
        let component = text.component();
        // Most texts translate nothing: the language is asked only for the keys found.
        let mut found = Vec::new();
        keys(&component, &mut found);
        let translations = match (found.is_empty(), lang::current()) {
            (false, Some(language)) => found
                .into_iter()
                .filter_map(|key| Some((key.to_owned(), language.get(key)?.to_owned())))
                .collect(),
            _ => Vec::new(),
        };
        Self {
            component,
            translations,
        }
    }
}

/// Text from its form: rebuilt from the component in the current language, or, carried where
/// no language is loaded, in the translations the form brought.
impl From<TextForm> for Text {
    fn from(form: TextForm) -> Self {
        if form.translations.is_empty() || lang::current().is_some() {
            return Text::from(form.component);
        }
        let language = lang::Language::of(lang::DEFAULT, form.translations);
        lang::scoped(language, || Text::from(form.component)).0
    }
}

/// Text from its JSON component, its translations left as their keys.
impl From<serde_json::Value> for Text {
    fn from(component: serde_json::Value) -> Self {
        crate::component::spans_of(component)
    }
}

/// Apply vanilla's numeric translation-argument representation to an owned component.
pub fn argument_component(c: Value) -> Value {
    match c
        .as_object()
        .and_then(|o| o.get("text").filter(|_| o.len() == 1))
    {
        Some(Value::String(s)) if s.parse::<i64>().is_ok_and(|n| n.to_string() == *s) => {
            s.parse::<i64>().map_or(c.clone(), Value::from)
        }
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file_style() -> Style {
        Style {
            click: Some(ClickEvent::OpenFile("/tmp/local.png".into())),
            ..Style::default()
        }
    }
    fn origins(text: &Text) -> Vec<Origin> {
        text.spans.iter().map(Span::origin).collect()
    }

    #[test]
    fn mixed_origins_survive_clone_append_style_and_flatten() {
        let local = Text::styled("A", file_style());
        let remote = Text::from(json!({"text":"B", "origin":"Local",
            "click_event":{"action":"open_file", "path":"/tmp/local.png"}}));
        assert_eq!(origins(&local), [Origin::Local]);
        assert_eq!(
            local.spans[0].click_event(),
            Some(&ClickEvent::OpenFile("/tmp/local.png".into()))
        );
        assert_eq!(origins(&remote.clone()), [Origin::Remote]);
        assert_eq!(remote.spans[0].click_event(), None);
        for (text, expected) in [
            (
                local.clone().append(remote.clone()),
                [Origin::Local, Origin::Remote],
            ),
            (
                remote.clone().append(local.clone()),
                [Origin::Remote, Origin::Local],
            ),
        ] {
            assert_eq!(origins(&text), expected);
            let styled = text.clone().bold();
            assert_eq!(origins(&styled), expected);
            assert_eq!(origins(&styled.slice(0, 2)), expected);
            assert_eq!(origins(&Text::from_spans(styled.spans.clone())), expected);
            let received = Text::from(text.component());
            assert_eq!(origins(&received), [Origin::Remote, Origin::Remote]);
            assert!(received.spans.iter().all(|s| s.click_event().is_none()));
        }
        let child = Text::from(json!({"text":"remote"}));
        let mixed = Text::literal("local")
            .append(child)
            .with_style(&file_style());
        assert_eq!(origins(&mixed), [Origin::Local, Origin::Remote]);
        assert!(mixed.spans[0].click_event().is_some());
        assert!(mixed.spans[1].click_event().is_none());
        let array = Text::from(json!([{"text":"A", "color":"red"}])).append(Text::literal("B"));
        let expected = vec![
            Span::new("A", Style::color(RED)),
            Span::new("B", Style::default()),
        ];
        assert_eq!(array.spans, expected);
        assert_eq!(Text::from(array.component()).spans, expected);
        assert_eq!(origins(&array), [Origin::Remote, Origin::Local]);
    }

    #[test]
    fn canonical_parent_style_matches_expected_rendered_spans() {
        let text = Text::empty()
            .with_component(json!({"text":"parent", "color":"red", "bold":true,
            "extra":[{"text":"child", "color":"blue"}, {"text":"sibling"}]}))
            .with_style(&Style {
                color: Some(GREEN),
                italic: Some(true),
                ..Style::default()
            });
        let expected = vec![
            Span::new(
                "parent",
                Style {
                    color: Some(RED),
                    bold: Some(true),
                    italic: Some(true),
                    ..Style::default()
                },
            ),
            Span::new(
                "child",
                Style {
                    color: Some(0x5555FF),
                    bold: Some(true),
                    italic: Some(true),
                    ..Style::default()
                },
            ),
            Span::new(
                "sibling",
                Style {
                    color: Some(RED),
                    bold: Some(true),
                    italic: Some(true),
                    ..Style::default()
                },
            ),
        ];
        assert_eq!(text.spans, expected);
        assert_eq!(Text::from(text.component()).spans, expected);
        assert_eq!(origins(&text), [Origin::Remote; 3]);
        let legacy =
            Text::from(json!({"text":"label", "hoverEvent":{"action":"show_text", "value":"old"}}))
                .with_style(&Style {
                    hover: Some(std::sync::Arc::new(Text::literal("new"))),
                    ..Style::default()
                });
        let expected = vec![Span::new(
            "label",
            Style {
                hover: Some(std::sync::Arc::new(Text::untrusted("old"))),
                ..Style::default()
            },
        )];
        assert_eq!(legacy.spans, expected);
        assert_eq!(Text::from(legacy.component()).spans, expected);
        assert_eq!(
            origins(legacy.spans[0].style.hover.as_ref().unwrap()),
            [Origin::Remote]
        );
    }

    #[test]
    fn translation_and_hover_keep_independent_node_origins() {
        let remote = Text::from(json!({"text":"R"}));
        let local = Text::literal("L");
        let translated = Text::translate_with_fallback("test.origin", "[%s/%s]", &[remote, local])
            .with_style(&Style {
                hover: Some(std::sync::Arc::new(Text::literal("tip"))),
                ..file_style()
            });
        assert_eq!(translated.plain(), "[R/L]");
        assert_eq!(
            origins(&translated),
            [
                Origin::Local,
                Origin::Remote,
                Origin::Local,
                Origin::Local,
                Origin::Local
            ]
        );
        assert_eq!(translated.node_origin(""), Origin::Local);
        assert_eq!(translated.node_origin("/with/0"), Origin::Remote);
        assert_eq!(translated.node_origin("/with/1"), Origin::Local);
        assert_eq!(translated.node_origin("/hover_event/value"), Origin::Local);
        let tip = translated.spans[0].style.hover.as_ref().unwrap();
        assert_eq!(origins(tip), [Origin::Local]);
        let received = Text::from(translated.component());
        assert_eq!(origins(&received), [Origin::Remote; 5]);
        assert_eq!(
            origins(received.spans[0].style.hover.as_ref().unwrap()),
            [Origin::Remote]
        );
        let form = Text::from(TextForm::from(&translated));
        assert_eq!(origins(&form), [Origin::Remote; 5]);
        let object_hover = Text::styled(
            "\u{fffc}",
            Style {
                inline_object: Some(InlineObject::Atlas {
                    atlas: "minecraft:blocks".into(),
                    sprite: "minecraft:block/stone".into(),
                    fallback: "stone".into(),
                }),
                ..file_style()
            },
        )
        .append(Text::untrusted("remote"));
        let parent = Text::literal("parent").with_style(&Style {
            hover: Some(std::sync::Arc::new(object_hover)),
            ..Style::default()
        });
        let hover = parent.spans[0]
            .style
            .hover
            .as_ref()
            .unwrap()
            .as_ref()
            .clone()
            .bold();
        // Styling keeps object metadata; plain text exposes its fallback.
        assert_eq!(hover.plain(), "stoneremote");
        assert!(hover.spans[0].style.inline_object.is_some());
        assert_eq!(origins(&hover), [Origin::Local, Origin::Remote]);
        assert!(hover.spans[0].click_event().is_some());
        assert!(hover.spans[1].click_event().is_none());
    }

    #[test]
    fn raw_replacement_and_resolved_components_cannot_grant_trust() {
        let raw = json!({"text":"remote", "click_event":{"action":"open_file", "path":"/tmp/x"}});
        let raw_field = Text::literal("local")
            .with_component_fields(vec![("click_event".into(), raw["click_event"].clone())]);
        assert_eq!(origins(&raw_field), [Origin::Remote]);
        assert_eq!(raw_field.spans[0].click_event(), None);
        assert_eq!(origins(&Text::from("implicit")), [Origin::Remote]);
        assert_eq!(
            origins(&crate::component::name_text(
                &horizon_core::nbt::Tag::String("§cname".into())
            )),
            [Origin::Remote]
        );
        let replaced = Text::literal("local").with_component(raw);
        assert_eq!(replaced.plain(), "remote");
        assert_eq!(origins(&replaced), [Origin::Remote]);
        let resolved = crate::component::to_text(&json!({"selector":"@p"}), &mut |_| {
            Text::styled("resolved", file_style())
        });
        assert_eq!(resolved.plain(), "resolved");
        assert_eq!(origins(&resolved), [Origin::Remote]);
        assert_eq!(resolved.spans[0].click_event(), None);
        let forged = Span {
            text: "remote".into(),
            style: file_style(),
            origin: Origin::Remote,
        };
        assert_eq!(forged.click_event(), None);
    }

    #[test]
    fn legacy_flattening_preserves_remote_origin_and_local_file_action() {
        let text = Text::from(json!({"text":"R§c!"})).append(Text::styled("L§a!", file_style()));
        let resolved = text.resolve_legacy();
        assert_eq!(resolved.plain(), "R!L!");
        assert_eq!(
            origins(&resolved),
            [Origin::Remote, Origin::Remote, Origin::Local, Origin::Local]
        );
        assert_eq!(
            origins(&text.strip_legacy()),
            [Origin::Remote, Origin::Local]
        );
        assert!(
            resolved.spans[2..]
                .iter()
                .all(|s| s.click_event().is_some())
        );
    }

    /// A translated text crossing to where no language is loaded reads as the game built it:
    /// the same spans, so the same `plain()`.
    #[test]
    fn a_text_form_reads_the_same_without_a_language() {
        let language = lang::Language::of(
            lang::DEFAULT,
            [("chat.type.text".to_owned(), "<%s> %s".to_owned())],
        );
        let ((form, built), _) = lang::scoped(language, || {
            let text = Text::translate(
                "chat.type.text",
                &[Text::literal("Steve"), Text::literal("hello")],
            );
            (TextForm::from(&text), text)
        });
        assert_eq!(built.plain(), "<Steve> hello");
        // The component alone, rebuilt with no language, reads as its key.
        assert_eq!(Text::from(form.component.clone()).plain(), "chat.type.text");
        let crossed = Text::from(form);
        assert_eq!(crossed, built);
        assert_eq!(crossed.plain(), "<Steve> hello");
    }

    /// Styling translated text where its language isn't loaded keeps the translation it was
    /// built with and gains the style; its spans keep their origins (arismoko/horizon#2493).
    #[test]
    fn styling_keeps_the_translation_it_was_built_with() {
        let language = lang::Language::of(
            lang::DEFAULT,
            [("chat.type.text".to_owned(), "<%s> %s".to_owned())],
        );
        let (built, _) = lang::scoped(language, || {
            Text::translate(
                "chat.type.text",
                &[Text::literal("Steve"), Text::literal("hello")],
            )
        });
        let origins: Vec<_> = built.rendered.spans.iter().map(|s| s.origin).collect();
        let red = Style {
            color: Some(0xFF5555),
            ..Style::default()
        };
        // A language without the key: rendering the source again would read `chat.type.text`.
        let (styled, _) = lang::scoped(lang::Language::of(lang::DEFAULT, []), || {
            built.with_style(&red)
        });
        assert_eq!(styled.plain(), "<Steve> hello");
        assert!(
            styled
                .rendered
                .spans
                .iter()
                .all(|span| span.style.color == Some(0xFF5555))
        );
        assert_eq!(
            styled
                .rendered
                .spans
                .iter()
                .map(|s| s.origin)
                .collect::<Vec<_>>(),
            origins
        );
        assert!(styled.component().get("color").is_some());
    }

    /// A parent's style goes underneath its children: a child that sets only its colour keeps
    /// it and gains the parent's bold, in the cached spans and in a fresh render alike.
    #[test]
    fn a_parent_style_goes_underneath_a_childs_own() {
        let text = Text::literal("a").append(Text::colored("b", 0x55FF55));
        let origins: Vec<_> = text.rendered.spans.iter().map(|s| s.origin).collect();
        let styled = text.with_style(&Style {
            color: Some(0x5555FF),
            bold: Some(true),
            ..Style::default()
        });
        let fresh = Text::from(styled.component());
        for spans in [&styled.rendered.spans, &fresh.rendered.spans] {
            let read: Vec<_> = spans
                .iter()
                .map(|s| (s.text.as_str(), s.style.color, s.style.bold))
                .collect();
            assert_eq!(
                read,
                [
                    ("a", Some(0x5555FF), Some(true)),
                    ("b", Some(0x55FF55), Some(true))
                ]
            );
        }
        assert_eq!(
            styled
                .rendered
                .spans
                .iter()
                .map(|s| s.origin)
                .collect::<Vec<_>>(),
            origins
        );
    }

    #[test]
    fn a_styled_object_span_keeps_its_object_through_append_and_slice() {
        let object = InlineObject::Atlas {
            atlas: "minecraft:gui".into(),
            sprite: "horizon:gui/mark".into(),
            fallback: "H".into(),
        };
        let sprite = Text::styled(
            "\u{fffc}",
            Style {
                inline_object: Some(object.clone()),
                ..Style::color(WHITE)
            },
        );
        let line = Text::colored("[", DARK_GRAY)
            .append(sprite)
            .append(Text::colored("] ", DARK_GRAY));
        let line = line.slice(0, 3);
        assert_eq!(line.spans[1].style.inline_object, Some(object));
        assert_eq!(line.plain(), "[H]");
    }
    #[test]
    fn appending_builds_the_same_tree_in_linear_time() {
        // A styled text is wrapped once, so its style doesn't reach what follows; a plain one
        // takes the siblings itself; a copy taken before an append keeps its own tree.
        let mut styled = Text::literal("a").with_style(&Style {
            bold: Some(true),
            ..Style::default()
        });
        styled.push(Text::literal("b"));
        let before = styled.clone();
        styled.push(Text::literal("c"));
        assert_eq!(
            styled.component(),
            json!({"text": "", "extra": [{"text": "a", "bold": true}, {"text": "b"}, {"text": "c"}]})
        );
        assert_eq!(
            before.component(),
            json!({"text": "", "extra": [{"text": "a", "bold": true}, {"text": "b"}]})
        );
        let mut plain = Text::default();
        plain.push(Text::literal("x"));
        plain.push(Text::literal("y"));
        assert_eq!(
            plain.component(),
            json!({"text": "", "extra": [{"text": "x"}, {"text": "y"}]})
        );
        // 20 000 siblings, as a server's chat may carry: 76 s before, as each append cloned the
        // whole tree.
        let started = std::time::Instant::now();
        let mut long = Text::default();
        for _ in 0..20_000 {
            long.push(Text::literal("a"));
        }
        assert_eq!(long.rendered.spans.len(), 20_000);
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }
    #[test]
    fn click_events_serialize_and_inherit_as_one_style_field() {
        for (event, json) in [
            (
                ClickEvent::SuggestCommand("/seed".into()),
                json!({"action": "suggest_command", "command": "/seed"}),
            ),
            (
                ClickEvent::CopyToClipboard("-9223372036854775808".into()),
                json!({"action": "copy_to_clipboard", "value": "-9223372036854775808"}),
            ),
            (
                ClickEvent::OpenFile("/tmp/My screenshots/世界.png".into()),
                json!({"action": "open_file", "path": "/tmp/My screenshots/世界.png"}),
            ),
        ] {
            let style = Style {
                click: Some(event.clone()),
                ..Style::default()
            };
            let text = Text::literal("link").with_style(&style);
            assert_eq!(text.component()["click_event"], json);
            assert_eq!(text.rendered.spans[0].style.click, Some(event));
            let child = Text::copyable("42").with_style(&style);
            assert_eq!(
                child.rendered.spans[1].style.click,
                Some(ClickEvent::CopyToClipboard("42".into()))
            );
            assert_eq!(
                child.component()["extra"][0]["click_event"],
                json!({"action": "copy_to_clipboard", "value": "42"})
            );
        }
    }

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
        assert_eq!(t.rendered.spans.last().unwrap().style.color, Some(RED));
    }
    #[test]
    fn legacy_codes_and_inheritance() {
        let t = Text::legacy("§aGreen §lbold§r plain");
        assert_eq!(t.plain(), "Green bold plain");
        assert_eq!(t.rendered.spans[0].style.color, Some(GREEN));
        assert!(t.rendered.spans[1].style.is_bold());
        assert_eq!(t.rendered.spans[2].style, Style::default());
        let t = Text::literal("x")
            .append(Text::colored("y", GOLD))
            .with_color(RED);
        assert_eq!(t.rendered.spans[0].style.color, Some(RED));
        assert_eq!(t.rendered.spans[1].style.color, Some(GOLD));
        assert_eq!(t.slice(1, 2).plain(), "y");
    }
    #[test]
    fn resolved_borrows_clean_text_and_restyles_codes() {
        let clean = Text::colored("Steve", GOLD);
        assert!(matches!(clean.resolved(), std::borrow::Cow::Borrowed(_)));
        // A colour code inside a span clears its decorations (a bold
        // "§cSteve" draws "Steve" red, not bold).
        let styled = Style {
            bold: Some(true),
            ..Style::default()
        };
        let styled = Text::styled("§cSteve", styled);
        let resolved = styled.resolved();
        assert_eq!(resolved.plain(), "Steve");
        assert_eq!(resolved.rendered.spans[0].style.color, Some(RED));
        assert_eq!(resolved.rendered.spans[0].style.bold, Some(false));
    }
    #[test]
    fn inherited_array_style_survives_refresh() {
        let mut text = Text::from(json!([{"text":"one"}, {"text":"two","italic":false}]));
        text.inherit_built_style(&Style {
            color: Some(RED),
            italic: Some(true),
            ..Default::default()
        });
        assert_eq!(
            text.component(),
            json!({"text":"", "extra":[[{"text":"one"},{"text":"two","italic":false}]], "color":"red", "italic":true})
        );
        text.refresh();
        assert_eq!(text.plain(), "onetwo");
        assert_eq!(
            text.rendered.spans.last().unwrap().style.italic,
            Some(false)
        );
        assert_eq!(text.rendered.spans.last().unwrap().style.color, Some(RED));
    }
    #[test]
    fn styling_installs_atlas_and_player_metadata() {
        for object in [
            InlineObject::Atlas {
                atlas: "a".into(),
                sprite: "b".into(),
                fallback: "".into(),
            },
            InlineObject::Player {
                profile: std::sync::Arc::new(json!({})),
                hat: true,
                fallback: "".into(),
            },
        ] {
            let text = Text::styled(
                "\u{fffc}",
                Style {
                    inline_object: Some(object),
                    ..Style::default()
                },
            );
            assert!(text.spans[0].style.inline_object.is_some());
        }
    }
    #[test]
    fn styling_remote_span_drops_local_file_action() {
        let text = Text::from(json!({"text":"remote"})).with_style(&Style {
            click: Some(ClickEvent::OpenFile("secret".into())),
            ..Style::default()
        });
        assert!(text.spans[0].style.click.is_none());
    }
}
