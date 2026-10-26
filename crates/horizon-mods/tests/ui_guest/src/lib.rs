//! A mod for the runtime's `ui` tests: `/ui_screen` opens a screen with one
//! of each widget, `/ui_settings` sets a settings page, `/ui_close` closes
//! the screen; the `ui-events` calls answer in chat so the host sees them.
use horizon_mod_sdk::{
    Mod, Registration, chat,
    ui::{self, ItemWidget, ListWidget, Screen, TextWidget, Widget},
};

struct Guest;

fn widgets() -> Vec<Widget> {
    vec![
        Widget::Text(TextWidget {
            id: 1,
            text: "Pick a row".into(),
            color: 0,
        }),
        Widget::Item(ItemWidget {
            id: 2,
            item: "minecraft:diamond".into(),
            name: String::new(),
        }),
        Widget::List(ListWidget {
            id: 7,
            rows: vec!["first".into(), "second".into(), "third".into()],
            selected: Some(1),
        }),
    ]
}

impl Mod for Guest {
    fn init() -> Result<(Self, Registration), String> {
        let hooks = ["ui_screen", "ui_settings", "ui_close"]
            .into_iter()
            .fold(Registration::new(), Registration::command);
        Ok((Guest, hooks))
    }
    fn command(&mut self, name: &str, _args: &str) -> Result<Option<String>, String> {
        match name {
            "ui_screen" => ui::open_screen(&Screen {
                title: "Guest's screen".into(),
                widgets: widgets(),
            }),
            "ui_settings" => ui::set_settings(&widgets()),
            "ui_close" => ui::close_screen(),
            _ => {}
        }
        Ok(None)
    }
    fn ui_select(&mut self, id: u32, row: u32) {
        chat::show(&format!("selected {id}:{row}"));
    }
    fn ui_closed(&mut self) {
        chat::show("closed");
    }
    fn ui_settings_opened(&mut self) {
        ui::set_settings(&[Widget::Text(TextWidget {
            id: 1,
            text: "fresh".into(),
            color: 0x33CC33,
        })]);
        chat::show("settings opened");
    }
}

horizon_mod_sdk::export_mod!(Guest);
