//! Books (`BookEditScreen`, `BookSignScreen`, `BookViewScreen`,
//! `LecternScreen`): writing pages of a book and quill (14 lines of 114
//! pixels, 1024 characters, 100 pages), signing it into a written book,
//! reading written books, and lecterns (placing a book, turning its pages
//! with a redstone pulse, the comparator output and taking the book).
use super::Game;
use horizon_client::{
    containers::{BlockEntity, Lectern},
    hud::{HudBatch, rgba},
    item_data::WrittenBook,
    items::ItemStack,
    text::Text,
    ui,
};
use horizon_core::position::BlockPos;
use horizon_world::level::flags as level_flags;
use std::time::Instant;
use winit::{
    event::{ElementState, KeyEvent},
    keyboard::{KeyCode, PhysicalKey},
};

const BLACK: u32 = 0xFF00_0000;
const PAGE_CHARS: usize = 1024;
const MAX_PAGES: usize = 100;
const LINES: usize = 14;
/// `BookSignScreen`'s title box.
const BOOK_TITLE: horizon_client::clipboard::FieldId = ("book.title", 0);

pub(super) enum BookScreen {
    Edit {
        /// Inventory slot of the book (the selected hotbar slot).
        slot: usize,
        pages: Vec<String>,
        page: usize,
        signing: bool,
        title: String,
        opened: Instant,
    },
    View {
        pages: Vec<String>,
        page: usize,
        lectern: Option<(i32, i32, i32)>,
    },
}
impl Game<'_> {
    fn book_lines(&self, page: &str) -> Vec<Text> {
        self.res.atlas.font.split(&Text::literal(page), 114.0)
    }
    /// `WritableBookItem.use` / `WrittenBookItem.use`.
    pub(super) fn use_book(&mut self, stack: &ItemStack) -> bool {
        let screen = match &*stack.item {
            "minecraft:writable_book" => {
                let mut pages = stack
                    .data()
                    .map(|d| d.writable_pages.clone())
                    .unwrap_or_default();
                if pages.is_empty() {
                    pages.push(String::new());
                }
                BookScreen::Edit {
                    slot: self.selected,
                    pages,
                    page: 0,
                    signing: false,
                    title: String::new(),
                    opened: Instant::now(),
                }
            }
            "minecraft:written_book" => BookScreen::View {
                pages: stack
                    .data()
                    .and_then(|d| d.written.as_ref())
                    .map(|w| w.pages.clone())
                    .unwrap_or_default(),
                page: 0,
                lectern: None,
            },
            _ => return false,
        };
        self.open_book(screen);
        self.swing_arm();
        true
    }
    fn open_book(&mut self, screen: BookScreen) {
        self.book = Some(screen);
        self.keys.clear();
        self.right = false;
        self.capture(false);
    }
    pub(super) fn close_book(&mut self) {
        self.book = None;
        self.capture(true);
    }
    /// `BookEditScreen.saveChanges`: trailing empty pages dropped.
    fn save_book(&mut self, sign: bool) {
        let Some(BookScreen::Edit {
            slot, pages, title, ..
        }) = self.book.take()
        else {
            return;
        };
        let mut pages = pages;
        while pages.len() > 1 && pages.last().is_some_and(String::is_empty) {
            pages.pop();
        }
        if let Some(stack) = self.inventory.get_mut(slot).and_then(Option::as_mut)
            && &*stack.item == "minecraft:writable_book"
        {
            if sign {
                let mut written = ItemStack::new("minecraft:written_book", 1);
                written.data_mut().written = Some(WrittenBook {
                    title: title.trim().to_owned(),
                    author: super::chat::PLAYER_NAME.to_owned(),
                    generation: 0,
                    pages,
                });
                *stack = written;
            } else {
                stack.data_mut().writable_pages = if pages.iter().all(String::is_empty) {
                    Vec::new()
                } else {
                    pages
                };
                stack.tidy();
            }
        }
        self.capture(true);
    }
    pub(super) fn book_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        if let Some(shortcut) = self.edit_shortcut(event) {
            self.book_shortcut(shortcut);
            return;
        }
        let key = match event.physical_key {
            PhysicalKey::Code(k) => Some(k),
            _ => None,
        };
        self.book_input(key, event.text.as_deref());
    }
    /// A key and the text it typed in the open book (the keyboard, or a
    /// controller's on-screen keyboard).
    pub(super) fn book_input(&mut self, key: Option<KeyCode>, text: Option<&str>) {
        match self.book.as_mut() {
            Some(BookScreen::View { .. }) => match key {
                Some(KeyCode::Escape) => self.close_book(),
                Some(KeyCode::PageUp) => self.turn_page(false),
                Some(KeyCode::PageDown) => self.turn_page(true),
                _ => {}
            },
            Some(BookScreen::Edit {
                signing: true,
                title,
                ..
            }) => match key {
                Some(KeyCode::Escape) => {
                    if let Some(BookScreen::Edit { signing, .. }) = self.book.as_mut() {
                        *signing = false;
                    }
                }
                Some(KeyCode::Backspace) => {
                    if !self.text_selection.take(BOOK_TITLE, title) {
                        title.pop();
                    }
                }
                Some(KeyCode::Enter | KeyCode::NumpadEnter) => {
                    if !title.trim().is_empty() {
                        self.save_book(true);
                    }
                }
                _ => {
                    if let Some(typed) = text {
                        self.text_selection.take(BOOK_TITLE, title);
                        for c in typed.chars().filter(|c| !c.is_control()) {
                            if title.chars().count() < 15 {
                                title.push(c);
                            }
                        }
                    }
                }
            },
            Some(BookScreen::Edit { .. }) => match key {
                Some(KeyCode::Escape) => self.save_book(false),
                Some(KeyCode::PageUp) => self.turn_page(false),
                Some(KeyCode::PageDown) => self.turn_page(true),
                Some(KeyCode::Backspace) => {
                    if !self.take_page_selection() {
                        self.edit_page(|p| {
                            p.pop();
                        })
                    }
                }
                Some(KeyCode::Enter | KeyCode::NumpadEnter) => {
                    self.take_page_selection();
                    self.type_into_page("\n")
                }
                _ => {
                    if let Some(typed) = text {
                        let typed: String = typed.chars().filter(|c| !c.is_control()).collect();
                        if !typed.is_empty() {
                            self.take_page_selection();
                        }
                        self.type_into_page(&typed);
                    }
                }
            },
            None => {}
        }
    }
    /// Typing for `horizon-ctl type` (`\n` is a line break).
    pub(super) fn book_type(&mut self, text: &str) {
        let text = text.replace("\\n", "\n");
        match self.book.as_mut() {
            Some(BookScreen::Edit {
                signing: true,
                title,
                ..
            }) => {
                for c in text.chars().filter(|c| !c.is_control()) {
                    if title.chars().count() < 15 {
                        title.push(c);
                    }
                }
            }
            Some(BookScreen::Edit { .. }) => self.type_into_page(&text),
            _ => {}
        }
    }
    /// Before typing on a page: Select All's selection is cleared (true).
    fn take_page_selection(&mut self) -> bool {
        match self.book.as_mut() {
            Some(BookScreen::Edit { pages, page, .. }) => self
                .text_selection
                .take(("book.page", *page), &mut pages[*page]),
            _ => false,
        }
    }
    /// Ctrl+A/C/V/X: the title box (an `EditBox` of 15) while signing, else
    /// the page (`MultilineTextField.insertText`: line breaks kept, cut to
    /// the 1024 characters left, rejected when it overflows the 14 lines).
    fn book_shortcut(&mut self, shortcut: horizon_client::clipboard::EditShortcut) {
        let font = &self.res.atlas.font;
        match self.book.as_mut() {
            Some(BookScreen::Edit {
                signing: true,
                title,
                ..
            }) => {
                Self::append_shortcut(
                    &mut self.text_selection,
                    &mut self.clipboard,
                    BOOK_TITLE,
                    title,
                    shortcut,
                    15,
                );
            }
            Some(BookScreen::Edit { pages, page, .. }) => {
                // truncateInsertionText counts the whole value, selection included.
                let room = PAGE_CHARS.saturating_sub(pages[*page].chars().count());
                self.text_selection.apply(
                    ("book.page", *page),
                    &mut pages[*page],
                    shortcut,
                    &mut self.clipboard,
                    |value, pasted| {
                        let text: String = horizon_client::clipboard::filter_text(pasted, true)
                            .chars()
                            .take(room)
                            .collect();
                        let next = format!("{value}{text}");
                        font.split(&Text::literal(&next), 114.0).len() <= LINES && {
                            *value = next;
                            true
                        }
                    },
                );
            }
            _ => {}
        }
    }
    fn edit_page(&mut self, f: impl FnOnce(&mut String)) {
        if let Some(BookScreen::Edit { pages, page, .. }) = self.book.as_mut() {
            f(&mut pages[*page]);
        }
    }
    /// MultiLineEditBox: characters that keep the page within 14 lines and 1024 characters.
    fn type_into_page(&mut self, typed: &str) {
        let Some(BookScreen::Edit { pages, page, .. }) = self.book.as_ref() else {
            return;
        };
        let mut text = pages[*page].clone();
        for c in typed.chars() {
            let mut next = text.clone();
            next.push(c);
            if next.chars().count() <= PAGE_CHARS && self.book_lines(&next).len() <= LINES {
                text = next;
            }
        }
        self.edit_page(|p| *p = text);
    }
    /// The page buttons (`pageForward` adds a page while editing).
    fn turn_page(&mut self, forward: bool) {
        let mut lectern_page = None;
        match self.book.as_mut() {
            Some(BookScreen::Edit { pages, page, .. }) => {
                if forward {
                    if *page + 1 >= pages.len() && pages.len() < MAX_PAGES {
                        pages.push(String::new());
                    }
                    if *page + 1 < pages.len() {
                        *page += 1;
                    }
                } else if *page > 0 {
                    *page -= 1;
                }
            }
            Some(BookScreen::View {
                pages,
                page,
                lectern,
            }) => {
                let before = *page;
                if forward && *page + 1 < pages.len() {
                    *page += 1;
                } else if !forward && *page > 0 {
                    *page -= 1;
                }
                if *page != before
                    && let Some(pos) = *lectern
                {
                    lectern_page = Some((pos, *page));
                }
            }
            None => return,
        }
        self.audio
            .play("minecraft:item.book.page_turn", None, 1.0, 1.0);
        if let Some((pos, page)) = lectern_page {
            self.set_lectern_page(pos, page as i32);
        }
    }
    /// `LecternBlockEntity.setPage` and `LecternBlock.signalPageChange`.
    fn set_lectern_page(&mut self, key: (i32, i32, i32), page: i32) {
        if let Some(BlockEntity::Lectern(l)) = self.world.block_entities.get_mut(&key) {
            l.page = page;
        }
        self.set_lectern_powered(key, true);
        self.lectern_pulses.push((key, 2));
        self.container_changed(key);
    }
    fn set_lectern_powered(&mut self, key: (i32, i32, i32), powered: bool) {
        self.set_lectern_property(key, "powered", powered);
        let p = BlockPos::new(key.0, key.1, key.2);
        let state = self.world.block(key.0, key.1, key.2);
        self.with_level(|b, level, ticks| {
            let block = b.block(state);
            b.update_neighbors_at(level, ticks, BlockPos::new(p.x, p.y - 1, p.z), block);
        });
    }
    fn set_lectern_property(&mut self, key: (i32, i32, i32), name: &str, value: bool) {
        let state = self.world.block(key.0, key.1, key.2);
        let registry = self.res.assets.registry;
        let Some(mut s) = registry.state(state).cloned() else {
            return;
        };
        if !s.properties.contains_key(name) {
            return;
        }
        s.properties.insert(name.into(), value.to_string());
        if let Some(new) = registry.id(&s) {
            let p = BlockPos::new(key.0, key.1, key.2);
            self.with_level(|b, level, ticks| {
                b.set_block(level, ticks, p, new, level_flags::UPDATE_ALL)
            });
        }
    }
    /// The 2-tick pulses of turned lecterns.
    pub(super) fn tick_lecterns(&mut self) {
        let mut done = Vec::new();
        for (key, left) in &mut self.lectern_pulses {
            *left -= 1;
            if *left <= 0 {
                done.push(*key);
            }
        }
        self.lectern_pulses.retain(|(_, left)| *left > 0);
        for key in done {
            self.set_lectern_powered(key, false);
        }
    }
    /// `LecternBlock.useItemOn`: a book goes onto an empty lectern.
    pub(super) fn use_lectern(&mut self, pos: [i32; 3], held: Option<&ItemStack>) -> bool {
        let key = (pos[0], pos[1], pos[2]);
        if &*self
            .res
            .facts
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name
            != "minecraft:lectern"
        {
            return false;
        }
        let has_book = matches!(self.world.block_entities.get(&key), Some(BlockEntity::Lectern(l)) if l.book.is_some());
        let Some(book) = held.filter(|h| {
            matches!(
                &*h.item,
                "minecraft:writable_book" | "minecraft:written_book"
            )
        }) else {
            return false;
        };
        if has_book {
            return false;
        }
        let mut one = book.clone();
        one.count = 1;
        self.world.block_entities.insert(
            key,
            BlockEntity::Lectern(Lectern {
                book: Some(one),
                page: 0,
            }),
        );
        self.set_lectern_property(key, "has_book", true);
        if !self.creative {
            self.consume_held();
        }
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        self.audio
            .play("minecraft:item.book.put", Some(at), 1.0, 1.0);
        self.container_changed(key);
        true
    }
    /// `LecternBlock.openScreen`: reading the lectern's book.
    pub(super) fn open_lectern(&mut self, pos: [i32; 3]) -> bool {
        let key = (pos[0], pos[1], pos[2]);
        let Some(BlockEntity::Lectern(l)) = self.world.block_entities.get(&key) else {
            return false;
        };
        let Some(book) = &l.book else { return false };
        let data = book.data();
        let pages = data
            .and_then(|d| d.written.as_ref().map(|w| w.pages.clone()))
            .or_else(|| data.map(|d| d.writable_pages.clone()))
            .unwrap_or_default();
        let page = (l.page.max(0) as usize).min(pages.len().saturating_sub(1));
        self.open_book(BookScreen::View {
            pages,
            page,
            lectern: Some(key),
        });
        true
    }
    /// `LecternMenu` "Take Book".
    fn take_lectern_book(&mut self, key: (i32, i32, i32)) {
        let book = match self.world.block_entities.get_mut(&key) {
            Some(BlockEntity::Lectern(l)) => {
                l.page = 0;
                l.book.take()
            }
            _ => None,
        };
        if let Some(book) = book
            && let Some(rest) = self.give(book)
            && let Some(eye) = self.eye_position()
        {
            self.spawn_item(rest, [eye[0], eye[1] - 0.3, eye[2]], [0.0, 0.1, 0.0], 40);
        }
        self.set_lectern_property(key, "has_book", false);
        self.container_changed(key);
        self.close_book();
    }
    pub(super) fn book_click(&mut self, size: [u32; 2]) {
        let b = HudBatch::new(size);
        let (w, left) = (b.width, ((b.width - 192.0) / 2.0).floor());
        let (mx, my) = self.cursor;
        let inside =
            |x: f32, y: f32, bw: f32, bh: f32| mx >= x && my >= y && mx < x + bw && my < y + bh;
        let click = |game: &mut Self| {
            game.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0)
        };
        match &self.book {
            Some(BookScreen::Edit {
                signing: true,
                title,
                ..
            }) => {
                let blank = title.trim().is_empty();
                if inside(w / 2.0 - 100.0, 196.0, 98.0, 20.0) && !blank {
                    click(self);
                    self.save_book(true);
                } else if inside(w / 2.0 + 2.0, 196.0, 98.0, 20.0) {
                    click(self);
                    if let Some(BookScreen::Edit { signing, .. }) = self.book.as_mut() {
                        *signing = false;
                    }
                }
            }
            Some(BookScreen::Edit { page, .. }) => {
                let page = *page;
                if inside(left + 116.0, 159.0, 23.0, 13.0) {
                    self.turn_page(true);
                } else if page > 0 && inside(left + 43.0, 159.0, 23.0, 13.0) {
                    self.turn_page(false);
                } else if inside(w / 2.0 - 100.0, 196.0, 98.0, 20.0) {
                    click(self);
                    if let Some(BookScreen::Edit { signing, .. }) = self.book.as_mut() {
                        *signing = true;
                    }
                } else if inside(w / 2.0 + 2.0, 196.0, 98.0, 20.0) {
                    click(self);
                    self.save_book(false);
                }
            }
            Some(BookScreen::View {
                pages,
                page,
                lectern,
            }) => {
                let (page, count, lectern) = (*page, pages.len(), *lectern);
                if page + 1 < count && inside(left + 116.0, 159.0, 23.0, 13.0) {
                    self.turn_page(true);
                } else if page > 0 && inside(left + 43.0, 159.0, 23.0, 13.0) {
                    self.turn_page(false);
                } else if let Some(key) = lectern {
                    if inside(w / 2.0 - 100.0, 196.0, 98.0, 20.0) {
                        click(self);
                        self.close_book();
                    } else if inside(w / 2.0 + 2.0, 196.0, 98.0, 20.0) {
                        click(self);
                        self.take_lectern_book(key);
                    }
                } else if inside((w - 200.0) / 2.0, 196.0, 200.0, 20.0) {
                    click(self);
                    self.close_book();
                }
            }
            None => {}
        }
    }
    pub(super) fn draw_book(&self, batch: &mut HudBatch) {
        let Some(book) = &self.book else { return };
        let atlas = &self.res.atlas;
        let (w, h) = (batch.width, batch.height);
        let (mx, my) = self.cursor;
        let hover =
            |x: f32, y: f32, bw: f32, bh: f32| mx >= x && my >= y && mx < x + bw && my < y + bh;
        batch.fill(atlas, 0.0, 0.0, w, h, rgba(16, 16, 16, 160));
        let left = ((w - 192.0) / 2.0).floor();
        let top = 2.0;
        if let Some(mut r) = atlas.sprites.get("gui/book").copied() {
            let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
            r.u1 = r.u0 + 192.0 * du;
            r.v1 = r.v0 + 192.0 * dv;
            batch.rect_uv(left, top, 192.0, 192.0, r, u32::MAX);
        }
        let tr = |key: &str, fallback: &str| self.translate(key, fallback);
        let page_buttons = |batch: &mut HudBatch, back: bool, forward: bool| {
            for (show, x, name) in [
                (forward, 116.0, "page_forward"),
                (back, 43.0, "page_backward"),
            ] {
                if show {
                    let sprite = if hover(left + x, top + 157.0, 23.0, 13.0) {
                        format!("gui/sprites/widget/{name}_highlighted")
                    } else {
                        format!("gui/sprites/widget/{name}")
                    };
                    batch.sprite(atlas, &sprite, left + x, top + 157.0);
                }
            }
        };
        let indicator = |batch: &mut HudBatch, page: usize, count: usize| {
            let text = tr("book.pageIndicator", "Page %s of %s")
                .replacen("%s", &(page + 1).to_string(), 1)
                .replacen("%s", &count.max(1).to_string(), 1);
            let tw = HudBatch::text_width(atlas, &text);
            batch.text(atlas, &text, left + 148.0 - tw, top + 16.0, BLACK, false);
        };
        let button = |batch: &mut HudBatch, x: f32, bw: f32, label: &str, active: bool| {
            ui::button(
                batch,
                atlas,
                x,
                196.0,
                bw,
                20.0,
                &Text::literal(label),
                active,
                hover(x, 196.0, bw, 20.0),
            );
        };
        match book {
            BookScreen::Edit {
                signing: true,
                title,
                opened,
                ..
            } => {
                let edit = tr("book.editTitle", "Enter Book Title:");
                let tw = HudBatch::text_width(atlas, &edit);
                batch.text(
                    atlas,
                    &edit,
                    left + 36.0 + (114.0 - tw) / 2.0,
                    34.0,
                    BLACK,
                    false,
                );
                let blink = (opened.elapsed().as_millis() / 300) % 2 == 0;
                let shown = format!("{title}{}", if blink { "_" } else { " " });
                let tw = HudBatch::text_width(atlas, &shown);
                batch.text(
                    atlas,
                    &shown,
                    left + 36.0 + (114.0 - tw) / 2.0,
                    50.0,
                    BLACK,
                    false,
                );
                let by = tr("book.byAuthor", "by %1$s").replace("%1$s", super::chat::PLAYER_NAME);
                let tw = HudBatch::text_width(atlas, &by);
                batch.text(
                    atlas,
                    &by,
                    left + 36.0 + (114.0 - tw) / 2.0,
                    60.0,
                    rgba(64, 64, 64, 255),
                    false,
                );
                let warning = tr(
                    "book.finalizeWarning",
                    "Note! When you sign the book, it will no longer be editable.",
                );
                for (i, line) in atlas
                    .font
                    .split(&Text::literal(&warning), 114.0)
                    .iter()
                    .enumerate()
                {
                    batch.text_styled(
                        atlas,
                        line,
                        left + 36.0,
                        82.0 + i as f32 * 9.0,
                        BLACK,
                        false,
                    );
                }
                button(
                    batch,
                    w / 2.0 - 100.0,
                    98.0,
                    &tr("book.finalizeButton", "Sign and Close"),
                    !title.trim().is_empty(),
                );
                button(
                    batch,
                    w / 2.0 + 2.0,
                    98.0,
                    &tr("gui.cancel", "Cancel"),
                    true,
                );
            }
            BookScreen::Edit {
                pages,
                page,
                opened,
                ..
            } => {
                indicator(batch, *page, pages.len());
                let blink = (opened.elapsed().as_millis() / 300) % 2 == 0;
                let lines = self.book_lines(&pages[*page]);
                for (i, line) in lines.iter().take(LINES).enumerate() {
                    batch.text_styled(
                        atlas,
                        line,
                        left + 36.0,
                        top + 30.0 + i as f32 * 9.0,
                        BLACK,
                        false,
                    );
                }
                if blink {
                    // The cursor after the last character.
                    let last = lines
                        .last()
                        .map(|l| atlas.font.width_text(l))
                        .unwrap_or(0.0);
                    let row = lines.len().saturating_sub(1) as f32
                        + if pages[*page].ends_with('\n') {
                            1.0
                        } else {
                            0.0
                        };
                    let x = if pages[*page].ends_with('\n') {
                        0.0
                    } else {
                        last
                    };
                    batch.fill(
                        atlas,
                        left + 36.0 + x,
                        top + 29.0 + row * 9.0,
                        1.0,
                        9.0,
                        BLACK,
                    );
                }
                page_buttons(batch, *page > 0, true);
                button(
                    batch,
                    w / 2.0 - 100.0,
                    98.0,
                    &tr("book.signButton", "Sign"),
                    true,
                );
                button(batch, w / 2.0 + 2.0, 98.0, &tr("gui.done", "Done"), true);
            }
            BookScreen::View {
                pages,
                page,
                lectern,
            } => {
                indicator(batch, *page, pages.len());
                if let Some(text) = pages.get(*page) {
                    for (i, line) in self.book_lines(text).iter().take(LINES).enumerate() {
                        batch.text_styled(
                            atlas,
                            line,
                            left + 36.0,
                            top + 30.0 + i as f32 * 9.0,
                            BLACK,
                            false,
                        );
                    }
                }
                page_buttons(batch, *page > 0, *page + 1 < pages.len());
                if lectern.is_some() {
                    button(batch, w / 2.0 - 100.0, 98.0, &tr("gui.done", "Done"), true);
                    button(
                        batch,
                        w / 2.0 + 2.0,
                        98.0,
                        &tr("lectern.take_book", "Take Book"),
                        true,
                    );
                } else {
                    button(
                        batch,
                        (w - 200.0) / 2.0,
                        200.0,
                        &tr("gui.done", "Done"),
                        true,
                    );
                }
            }
        }
    }
}
/// `LecternBlockEntity.getRedstoneSignal`.
pub(super) fn lectern_signal(world: &horizon_client::world::World, pos: BlockPos) -> Option<i32> {
    match world.block_entities.get(&(pos.x, pos.y, pos.z)) {
        Some(BlockEntity::Lectern(l)) => Some(l.book.as_ref().map_or(0, |b| {
            let data = b.data();
            let count = data
                .and_then(|d| d.written.as_ref().map(|w| w.pages.len()))
                .or_else(|| data.map(|d| d.writable_pages.len()))
                .unwrap_or(0)
                .max(1);
            let f = if count > 1 {
                l.page as f32 / (count - 1) as f32
            } else {
                1.0
            };
            (f * 14.0).floor() as i32 + 1
        })),
        _ => None,
    }
}
