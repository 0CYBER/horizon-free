use horizon_mod_sdk::{Mod, Registration, chat};

struct MyMod {
    greeted: u32,
}

impl Mod for MyMod {
    fn init() -> Result<(Self, Registration), String> {
        Ok((MyMod { greeted: 0 }, Registration::new().command("hello")))
    }

    fn command(&mut self, _name: &str, args: &str) -> Result<Option<String>, String> {
        self.greeted += 1;
        chat::show(&format!("Hello {args}! ({} so far)", self.greeted));
        Ok(None)
    }
}

horizon_mod_sdk::export_mod!(MyMod);
