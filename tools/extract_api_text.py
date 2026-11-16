#!/usr/bin/env python3
"""Move the data-only text boundary below Game; run at the transport checkpoint."""
from pathlib import Path

root = Path(__file__).resolve().parents[1]
src = root / 'crates/horizon-client/src'
old = src / 'game/modding/api_values/text_boundary.rs'
style = old.with_name('text_style_capture.rs')
text = old.read_text()
api_impl = '''impl TryIntoApi for engine::Text {
    type Api = Text;
    type Error = Error;
    fn try_into_api(&self) -> Result<Text, Error> {
        capture(Source::Engine(self))
    }
}'''
engine_impl = '''impl<C: ?Sized> TryIntoEngine<C> for Text {
    type Engine = engine::Text;
    type Error = Error;
    fn try_into_engine(&self, _: &C) -> Result<engine::Text, Error> {
        owned_text_component(self).map(Into::into)
    }
}'''
assert api_impl in text and engine_impl in text
text = text.replace('use super::{TryIntoApi, TryIntoEngine};\n', '', 1)
text = text.replace('pub(super) fn captured_text', 'pub fn captured_text').replace('pub(in crate::game) fn style_hover', 'pub fn style_hover')
text = text.replace(api_impl, '''/// Capture an engine text without losing structured content or style.
pub fn to_api(text: &engine::Text) -> Result<Text, TextFormError> {
    capture(Source::Engine(text))
}''').replace(engine_impl, '''/// Encode API text, preserving the boundary's validation and representation errors.
pub fn to_engine(text: &Text) -> Result<engine::Text, TextFormError> {
    owned_text_component(text).map(Into::into)
}''')
# Existing tests keep their call syntax through private, test-only adapters.
text += '''
#[cfg(test)]
trait TryIntoApi {
    fn try_into_api(&self) -> Result<Text, TextFormError>;
}
#[cfg(test)]
impl TryIntoApi for engine::Text {
    fn try_into_api(&self) -> Result<Text, TextFormError> { to_api(self) }
}
#[cfg(test)]
trait TryIntoEngine<C: ?Sized> {
    fn try_into_engine(&self, context: &C) -> Result<engine::Text, TextFormError>;
}
#[cfg(test)]
impl<C: ?Sized> TryIntoEngine<C> for Text {
    fn try_into_engine(&self, _: &C) -> Result<engine::Text, TextFormError> { to_engine(self) }
}
'''
(src / 'api_text').mkdir(exist_ok=True)
(src / 'api_text/text_style_capture.rs').write_text('''use super::*;

/// Capture a standalone engine style through the same text boundary.
pub fn style_to_api(style: &engine::Style) -> Result<TextStyle, TextFormError> {
    let span = engine::Span::new("", style.clone());
    Ok(capture(Source::Span(&span))?.style)
}
''')
text = text.replace('#[path = "text_style_capture.rs"]', '#[path = "api_text/text_style_capture.rs"]')
text = text.replace('mod text_style_capture;', 'mod text_style_capture;\npub use text_style_capture::style_to_api;')
(src / 'api_text.rs').write_text(text)
old.write_text('''#[path = "text_style_capture.rs"]
mod text_style_capture;
use super::{TryIntoApi, TryIntoEngine};
use horizon_client_core::text as engine;
use mod_api::text::{Text, TextFormError};

pub(super) use horizon_client_lib::api_text::captured_text;
pub(in crate::game) use horizon_client_lib::api_text::style_hover;

impl TryIntoApi for engine::Text {
    type Api = Text;
    type Error = TextFormError;
    fn try_into_api(&self) -> Result<Text, TextFormError> {
        horizon_client_lib::api_text::to_api(self)
    }
}
impl<C: ?Sized> TryIntoEngine<C> for Text {
    type Engine = engine::Text;
    type Error = TextFormError;
    fn try_into_engine(&self, _: &C) -> Result<engine::Text, TextFormError> {
        horizon_client_lib::api_text::to_engine(self)
    }
}
''')
style.write_text('''use super::*;

impl TryIntoApi for engine::Style {
    type Api = mod_api::text::TextStyle;
    type Error = TextFormError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        horizon_client_lib::api_text::style_to_api(self)
    }
}
''')
lib = src / 'client_lib.rs'
lib.write_text(lib.read_text().replace('pub mod advancements;', 'pub mod api_text;\npub mod advancements;', 1))
