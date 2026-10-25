//! InWorldGameRulesScreen (AbstractGameRulesScreen), opened from World
//! Options: the rules by category under a search box, a switch or a number
//! box each, and Done sending the changed ones (ServerboundSetGameRulePacket).
//! Leaving with changes asks first (ConfirmScreen).
use horizon_client::{
    confirm_screen::Confirm,
    font::Font,
    gamerules::{self, GameRules, RULES, Rule, Value},
    hud::{HudAtlas, HudBatch},
    lang,
    text::{Style, Text, named_color},
    ui::{self, EditBox},
};

/// HeaderAndFooterLayout(12 + 9 + 15, 33): the title over the search box.
const HEADER: f32 = 36.0;
const FOOTER: f32 = 33.0;
/// RuleList: 24 pixel entries in AbstractSelectionList's 220 pixel rows.
const ROW: f32 = 24.0;
const ROW_WIDTH: f32 = 220.0;
const SEARCH_WIDTH: f32 = 200.0;
/// EditBox text colours (`hud::rgba` order, ABGR): IntegerRuleEntry's valid
/// (-2039584) and invalid (-65536).
const VALID: u32 = 0xFFE0_E0E0;
const INVALID: u32 = 0xFF00_00FF;

#[derive(Clone, Copy)]
enum Row {
    Category(&'static str),
    Rule(&'static Rule),
}

/// What a click or Escape asks of World Options.
#[derive(Debug, PartialEq)]
pub(super) enum Exit {
    /// Done: the rules that changed.
    Apply(Vec<(&'static str, Value)>),
    /// Cancel, or discarding the changes.
    Discard,
}

pub(super) struct RulesScreen {
    search: EditBox,
    values: GameRules,
    initial: GameRules,
    /// IntegerRuleEntry's boxes, by rule.
    numbers: Vec<(&'static str, EditBox)>,
    scroll: f32,
    /// ConfirmScreen "Game Rule Changes" in its place.
    confirm: bool,
}

fn name_key(r: &Rule) -> String {
    format!("gamerule.minecraft.{}", r.name)
}
fn category_key(category: &str) -> String {
    format!("gamerule.category.minecraft.{category}")
}
/// GameRule.deserialize of an integer rule: Brigadier's readInt (digits,
/// '.' and '-' only, nothing after), then the rule's range.
fn parse(r: &Rule, s: &str) -> Option<i32> {
    let number = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == '.');
    number
        .then(|| s.parse::<i32>().ok())
        .flatten()
        .filter(|v| (r.min..=r.max).contains(v))
}
fn lines(font: &Font, t: &Text, width: f32) -> Vec<Text> {
    font.split_lines(t, width)
        .into_iter()
        .map(|(line, _)| line)
        .collect()
}
fn row_left(w: f32) -> f32 {
    (w / 2.0).trunc() - ROW_WIDTH / 2.0
}
/// A rule's switch or box: 44 by 20, 45 from the entry content's right.
fn control_x(w: f32) -> f32 {
    row_left(w) + 2.0 + (ROW_WIDTH - 4.0) - 45.0
}
/// The header's LinearLayout (title, 4, search box) centred in its 36 pixels.
fn search_at(w: f32) -> (f32, f32) {
    (((w - SEARCH_WIDTH) / 2.0).trunc(), 4.0 + 9.0 + 4.0)
}
/// The footer's Done and Cancel, 150 wide and 8 apart.
fn footer(w: f32, h: f32) -> [(f32, f32); 2] {
    let x = ((w - 308.0) / 2.0).trunc();
    [(x, h - FOOTER + 6.0), (x + 158.0, h - FOOTER + 6.0)]
}
fn yellow() -> u32 {
    named_color("yellow").unwrap_or(0xFFFF55)
}

/// ConfirmScreen "Game Rule Changes".
fn confirm(font: &Font, size: (f32, f32)) -> Confirm {
    let title = Text::translate("editGamerule.inGame.discardChanges.title", &[]);
    let message = Text::translate("editGamerule.inGame.discardChanges.message", &[]);
    Confirm::new(font, size, &title, &message)
}

impl RulesScreen {
    pub(super) fn new(rules: &GameRules) -> Self {
        // setInitialFocus: the search box.
        let search = EditBox::new(0.0, 0.0, SEARCH_WIDTH, 15.0);
        let numbers = RULES
            .iter()
            .filter(|r| matches!(r.default, Value::Int(_)))
            .map(|r| {
                let mut b = EditBox::new(0.0, 0.0, 44.0, 20.0);
                b.focused = false;
                b.set_value(&rules.int(r.name).to_string());
                b.text_color = VALID;
                (r.name, b)
            })
            .collect();
        Self {
            search,
            values: rules.clone(),
            initial: rules.clone(),
            numbers,
            scroll: 0.0,
            confirm: false,
        }
    }

    /// RuleList.populateChildren: the rules matching the search (id, name,
    /// category or description), by category, each sorted by description id.
    fn rows(&self) -> Vec<Row> {
        let filter = self.search.value.to_lowercase();
        let has = |s: &str| s.to_lowercase().contains(&filter);
        let mut categories: Vec<(&'static str, Vec<&'static Rule>)> = Vec::new();
        for r in RULES {
            let key = name_key(r);
            let description = format!("{key}.description");
            let matches = has(r.name)
                || has(&lang::get(&key))
                || has(&lang::get(&category_key(r.category)))
                || (lang::has(&description) && has(&lang::get(&description)));
            if !matches {
                continue;
            }
            match categories.iter_mut().find(|(c, _)| *c == r.category) {
                Some((_, rules)) => rules.push(r),
                None => categories.push((r.category, vec![r])),
            }
        }
        // By the categories' ids, then the rules' description ids.
        categories.sort_by_key(|(c, _)| *c);
        categories
            .into_iter()
            .flat_map(|(c, mut rules)| {
                rules.sort_by_key(|r| name_key(r));
                std::iter::once(Row::Category(c)).chain(rules.into_iter().map(Row::Rule))
            })
            .collect()
    }
    fn max_scroll(rows: usize, h: f32) -> f32 {
        (ROW * rows as f32 + 4.0 - (h - HEADER - FOOTER)).max(0.0)
    }
    /// Entry `i`'s top: 2 under the list's top, less the scroll.
    fn row_top(&self, i: usize) -> f32 {
        HEADER + 2.0 - self.scroll.trunc() + ROW * i as f32
    }
    /// getEntryAtPosition: the row under the pointer, inside the list.
    fn hovered(&self, rows: usize, (x, y): (f32, f32), (w, h): (f32, f32)) -> Option<usize> {
        let left = row_left(w);
        let inside = y >= HEADER && y < h - FOOTER && x >= left && x < left + ROW_WIDTH;
        let i = ((y - self.row_top(0)) / ROW).floor();
        (inside && i >= 0.0 && (i as usize) < rows).then_some(i as usize)
    }
    fn number(&mut self, name: &str) -> Option<&mut EditBox> {
        self.numbers
            .iter_mut()
            .find(|(n, _)| *n == name)
            .map(|(_, b)| b)
    }
    /// An invalid number disables Done (markInvalid).
    fn invalid(&self) -> bool {
        self.numbers
            .iter()
            .any(|(name, b)| gamerules::rule(name).is_none_or(|r| parse(r, &b.value).is_none()))
    }
    fn changes(&self) -> Vec<(&'static str, Value)> {
        RULES
            .iter()
            .filter_map(|r| {
                let value = self.values.get(r.name)?;
                (Some(value) != self.initial.get(r.name)).then_some((r.name, value))
            })
            .collect()
    }
    /// InWorldGameRulesScreen.onClose: pending changes ask first.
    fn close(&mut self) -> Option<Exit> {
        if self.changes().is_empty() {
            Some(Exit::Discard)
        } else {
            self.confirm = true;
            None
        }
    }
    /// Escape: ConfirmScreen answers no; the rules close.
    pub(super) fn escape(&mut self) -> Option<Exit> {
        if self.confirm {
            self.confirm = false;
            None
        } else {
            self.close()
        }
    }
    /// The mouse wheel (AbstractSelectionList: half an entry a step).
    pub(super) fn wheel(&mut self, delta: f32, h: f32) {
        if !self.confirm {
            let max = Self::max_scroll(self.rows().len(), h);
            self.scroll = (self.scroll - delta * ROW / 2.0).clamp(0.0, max);
        }
    }
    fn focused(&mut self) -> Option<(Option<&'static str>, &mut EditBox)> {
        if self.search.focused {
            return Some((None, &mut self.search));
        }
        self.numbers
            .iter_mut()
            .find(|(_, b)| b.focused)
            .map(|(n, b)| (Some(*n), b))
    }
    /// The focused box changed: the search filters again from the top; a
    /// number sets its rule when valid (IntegerRuleEntry's responder).
    fn edited(&mut self, rule: Option<&'static str>) {
        let Some(name) = rule else {
            self.scroll = 0.0;
            return;
        };
        let Some(r) = gamerules::rule(name) else {
            return;
        };
        let value = self.number(name).map(|b| parse(r, &b.value));
        if let Some(Some(v)) = value {
            self.values.set(name, Value::Int(v));
        }
        if let Some(b) = self.number(name) {
            b.text_color = if matches!(value, Some(Some(_))) {
                VALID
            } else {
                INVALID
            };
        }
    }
    /// Run an edit on the focused box, then its responder.
    pub(super) fn edit(&mut self, f: impl FnOnce(&mut EditBox)) {
        let Some((rule, b)) = self.focused() else {
            return;
        };
        let before = b.value.clone();
        f(b);
        if b.value != before {
            self.edited(rule);
        }
    }
    pub(super) fn typing(&self) -> bool {
        !self.confirm && (self.search.focused || self.numbers.iter().any(|(_, b)| b.focused))
    }

    /// A left click; Some when the screen closes.
    pub(super) fn click(
        &mut self,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        size: (f32, f32),
    ) -> Option<Exit> {
        let (w, h) = size;
        let on = |x: f32, y: f32, bw: f32, bh: f32| ui::hovered(cursor, x, y, bw, bh);
        if self.confirm {
            match confirm(&atlas.font, size).answer_at(cursor) {
                Some(true) => return Some(Exit::Discard),
                Some(false) => self.confirm = false,
                None => {}
            }
            return None;
        }
        let [(dx, dy), (cx, cy)] = footer(w, h);
        if on(dx, dy, 150.0, 20.0) && !self.invalid() {
            return Some(Exit::Apply(self.changes()));
        }
        if on(cx, cy, 150.0, 20.0) {
            return self.close();
        }
        // A click moves the focus to the widget under it.
        let (sx, sy) = search_at(w);
        if on(sx, sy, SEARCH_WIDTH, 15.0) {
            self.unfocus();
            self.search.focused = true;
            self.search.reset_blink();
            self.search.click(atlas, cursor.0, false, false);
            return None;
        }
        let rows = self.rows();
        let i = self.hovered(rows.len(), cursor, size)?;
        let Row::Rule(r) = rows[i] else {
            return None;
        };
        if !on(control_x(w), self.row_top(i) + 2.0, 44.0, 20.0) {
            return None;
        }
        self.unfocus();
        match r.default {
            Value::Bool(_) => {
                let on = self.values.bool(r.name);
                self.values.set(r.name, Value::Bool(!on));
            }
            Value::Int(_) => {
                if let Some(b) = self.number(r.name) {
                    b.focused = true;
                    b.reset_blink();
                    b.click(atlas, cursor.0, false, false);
                }
            }
        }
        None
    }
    fn unfocus(&mut self) {
        self.search.focused = false;
        for (_, b) in &mut self.numbers {
            b.focused = false;
        }
    }
    /// Whether a click lands on a button (for its sound).
    pub(super) fn clicks_button(
        &self,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        size: (f32, f32),
    ) -> bool {
        let (w, h) = size;
        let on = |(x, y): (f32, f32)| ui::hovered(cursor, x, y, 150.0, 20.0);
        if self.confirm {
            return confirm(&atlas.font, size).answer_at(cursor).is_some();
        }
        let [done, cancel] = footer(w, h);
        if (on(done) && !self.invalid()) || on(cancel) {
            return true;
        }
        let rows = self.rows();
        self.hovered(rows.len(), cursor, size).is_some_and(|i| {
            matches!(rows[i], Row::Rule(r) if matches!(r.default, Value::Bool(_)))
                && ui::hovered(cursor, control_x(w), self.row_top(i) + 2.0, 44.0, 20.0)
        })
    }

    pub(super) fn draw(&mut self, batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32)) {
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        if self.confirm {
            let labels = [
                Text::translate("gui.yes", &[]),
                Text::translate("gui.no", &[]),
            ];
            confirm(&atlas.font, (w, h)).draw(batch, atlas, cursor, [&labels[0], &labels[1]]);
            return;
        }
        let rows = self.rows();
        self.scroll = self.scroll.clamp(0.0, Self::max_scroll(rows.len(), h));
        // RuleList: the in-world list background, its entries clipped to it,
        // then the separators and scrollbar.
        ui::list_background(batch, atlas, HEADER, h - FOOTER, true);
        let first = batch.vertices.len();
        let hovered = self.hovered(rows.len(), cursor, (w, h));
        let (left, x) = (row_left(w), control_x(w));
        for (i, row) in rows.iter().enumerate() {
            let top = self.row_top(i);
            if top + ROW < HEADER || top > h - FOOTER {
                continue;
            }
            let (cx, cy) = (left + 2.0, top + 2.0);
            match *row {
                Row::Category(c) => {
                    // A FocusableTextWidget padded by 4, centred in the entry.
                    let label = Text::translate(&category_key(c), &[])
                        .bold()
                        .underlined()
                        .with_color(yellow());
                    let lw = atlas.font.width_text(&label);
                    let mid = cx + (ROW_WIDTH - 4.0) / 2.0;
                    let lx = mid - (lw / 2.0).trunc();
                    batch.text_styled(atlas, &label, lx, cy + 10.0 - 8.0 + 4.0, u32::MAX, true);
                }
                Row::Rule(r) => {
                    let name = lines(&atlas.font, &Text::translate(&name_key(r), &[]), 170.0);
                    match name.as_slice() {
                        [one] => {
                            batch.text_styled(atlas, one, cx, cy + 5.0, u32::MAX, true);
                        }
                        [one, two, ..] => {
                            batch.text_styled(atlas, one, cx, cy, u32::MAX, true);
                            batch.text_styled(atlas, two, cx, cy + 10.0, u32::MAX, true);
                        }
                        [] => {}
                    }
                    let hover = hovered == Some(i) && ui::hovered(cursor, x, cy, 44.0, 20.0);
                    match r.default {
                        Value::Bool(_) => {
                            let key = if self.values.bool(r.name) {
                                "options.on"
                            } else {
                                "options.off"
                            };
                            let label = Text::translate(key, &[]);
                            ui::button(batch, atlas, x, cy, 44.0, 20.0, &label, true, hover);
                        }
                        Value::Int(_) => {
                            if let Some(b) = self.number(r.name) {
                                b.x = x;
                                b.y = cy;
                                b.draw(batch, atlas, None);
                            }
                        }
                    }
                }
            }
        }
        batch.clip_since(first, 0.0, HEADER, w, h - FOOTER);
        ui::separators(batch, atlas, HEADER, h - FOOTER, true);
        let content = ROW * rows.len() as f32 + 4.0;
        let bar_x = left + ROW_WIDTH + 6.0 + 2.0;
        ui::scrollbar(
            batch,
            atlas,
            bar_x,
            HEADER,
            h - FOOTER,
            content,
            self.scroll.trunc(),
        );
        // The header: the title centred over the search box.
        let title = Text::translate("editGamerule.title", &[]);
        let tw = atlas.font.width_text(&title);
        let (sx, sy) = search_at(w);
        let lw = tw.max(SEARCH_WIDTH);
        let lx = ((w - lw) / 2.0).trunc();
        batch.text_styled(
            atlas,
            &title,
            lx + ((lw - tw) / 2.0).trunc(),
            4.0,
            u32::MAX,
            true,
        );
        self.search.x = sx;
        self.search.y = sy;
        self.search.draw(batch, atlas, None);
        if self.search.value.is_empty() && !self.search.focused {
            // EditBox.SEARCH_HINT_STYLE: gray and italic.
            let hint = Text::translate("gui.game_rule.search", &[]).with_style(&Style {
                color: named_color("gray"),
                italic: Some(true),
                ..Style::default()
            });
            let (hx, hy) = (self.search.text_x(), self.search.text_y());
            batch.text_styled(atlas, &hint, hx, hy, u32::MAX, true);
        }
        let invalid = self.invalid();
        for ((bx, by), (key, active)) in footer(w, h)
            .into_iter()
            .zip([("gui.done", !invalid), ("gui.cancel", true)])
        {
            let hover = active && ui::hovered(cursor, bx, by, 150.0, 20.0);
            let label = Text::translate(key, &[]);
            ui::button(batch, atlas, bx, by, 150.0, 20.0, &label, active, hover);
        }
        // The hovered rule's tooltip: its id, description and default.
        if let Some(Row::Rule(r)) = hovered.map(|i| rows[i]) {
            let mut tip = vec![Text::literal(r.name).with_color(yellow())];
            let description = format!("{}.description", name_key(r));
            if lang::has(&description) {
                tip.extend(lines(
                    &atlas.font,
                    &Text::translate(&description, &[]),
                    150.0,
                ));
            }
            let gray = named_color("gray").unwrap_or(0xAAAAAA);
            tip.push(
                Text::translate(
                    "editGamerule.default",
                    &[Text::literal(r.default.to_string())],
                )
                .with_color(gray),
            );
            ui::tooltip(batch, atlas, &tip, cursor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_colours_are_vanillas() {
        assert_eq!(VALID, horizon_client::hud::rgba(224, 224, 224, 255));
        assert_eq!(INVALID, horizon_client::hud::rgba(255, 0, 0, 255));
    }

    #[test]
    fn numbers_parse_like_brigadier_within_the_rule() {
        let r = gamerules::rule("max_snow_accumulation_height").unwrap();
        assert_eq!(parse(r, "8"), Some(8));
        assert_eq!(parse(r, "9"), None);
        assert_eq!(parse(r, "-1"), None);
        for s in ["", "+3", " 3", "3 ", "3.0", "x"] {
            assert_eq!(parse(r, s), None, "{s:?}");
        }
        let r = gamerules::rule("fire_spread_radius_around_player").unwrap();
        assert_eq!(parse(r, "-1"), Some(-1));
    }

    #[test]
    fn rows_group_categories_and_rules_by_description_id() {
        let s = RulesScreen::new(&GameRules::default());
        let rows = s.rows();
        assert_eq!(rows.len(), RULES.len() + 7);
        assert!(matches!(rows[0], Row::Category("chat")));
        let categories: Vec<_> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Category(c) => Some(*c),
                Row::Rule(_) => None,
            })
            .collect();
        assert_eq!(
            categories,
            [
                "chat", "drops", "misc", "mobs", "player", "spawning", "updates"
            ]
        );
    }

    #[test]
    fn edits_change_rules_and_bad_numbers_hold_done() {
        let mut s = RulesScreen::new(&GameRules::default());
        assert!(s.changes().is_empty());
        s.unfocus();
        s.number("random_tick_speed").unwrap().focused = true;
        s.edit(|b| b.set_value("10"));
        assert_eq!(s.changes(), vec![("random_tick_speed", Value::Int(10))]);
        s.edit(|b| b.set_value("ten"));
        assert!(s.invalid());
        // The last valid value stays, as the responder left it.
        assert_eq!(s.changes(), vec![("random_tick_speed", Value::Int(10))]);
        // Leaving with changes asks; No returns, Yes discards.
        assert_eq!(s.escape(), None);
        assert!(s.confirm);
        assert_eq!(s.escape(), None);
        assert!(!s.confirm);
    }
}
