use super::*;

// Compile the generated crossing and the actual handwritten adapter bodies against
// an isolated strict stack boundary, so refusal and atomic writeback are observable.
#[test]
#[ignore = "daily"]
fn untouched_writeback_runtime_and_mutants() {
    let ty: Type = syn::parse_quote!(&mut ItemStack);
    let sibling: Type = syn::parse_quote!(&mut bool);
    let crossing = stack_crossing(
        &[("stack".into(), &ty), ("handled".into(), &sibling)],
        None,
        "Proof",
        false,
        &Default::default(),
    )
    .unwrap();
    assert!(
        crossing
            .after
            .contains("mod proof write refused: {error:?}")
    );
    let generated = format!(
        "{} self.native_generated(&mut __api_stack, &mut __api_handled); {}",
        crossing.before, crossing.after
    );
    let handwritten = include_str!("../../horizon-client/src/game/modding/api_values.rs");
    let mut methods =
        format!("fn generated(&mut self, stack: &mut Stack, handled: &mut bool) {{ {generated} }}");
    for name in [
        "anvil_update",
        "grindstone_place",
        "grindstone_take",
        "menu_button",
        "menu_quick_move",
    ] {
        let start = handwritten.find(&format!("fn api_{name}(")).unwrap();
        let text = &handwritten[start..];
        let begin = text.find("{").unwrap();
        let mut depth = 1;
        let end = text[begin + 1..]
            .char_indices()
            .find_map(|(i, c)| {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                (depth == 0).then_some(begin + 1 + i)
            })
            .unwrap();
        let body = &text[begin + 1..end];
        methods.push_str(&format!("fn {name}(&mut self, value: &mut Stack, entity: &mut Stack, carried: &mut Option<Stack>, stack: &mut Stack, cancel: &mut bool, handled: &mut bool) {{ let player = 0; let pos = [0; 3]; let context = &(); let container_slots = 1; let slot = 0; {body} }}"));
    }
    let fixture = r#"
#![allow(unused)]
extern crate self as log;
#[macro_export] macro_rules! warn { ($($arg:tt)*) => { eprintln!($($arg)*); } }
#[derive(Clone, Debug, PartialEq)] struct Stack(u32);
struct Game { change: bool }
static VALIDATIONS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[derive(Debug)] enum BlockEntitySnapshotError { InvalidStack }
mod mod_api {
    #[derive(Debug)] pub enum StackWriteError { Unavailable }
    pub enum HookId { Proof, MenuButton, MenuQuickMove }
    pub mod __private {
        pub fn listened(_: super::HookId) -> bool { true }
        pub fn has_part_hook(_: super::HookId) -> bool { true }
    }
}
mod game { pub mod api_values {
    pub trait TryIntoApi { type Api; fn try_into_api(&self) -> Result<Self::Api, ()>; }
    pub trait TryIntoEngine { type Engine; fn try_into_engine(&self, _: &()) -> Result<Self::Engine, crate::mod_api::StackWriteError>; }
    pub trait ItemBoundary { fn boundary_items(&self) -> Option<&()> { Some(&()) } }
} }
use game::api_values::*;
impl TryIntoApi for Stack { type Api = Stack; fn try_into_api(&self) -> Result<Stack, ()> { Ok(self.clone()) } }
impl TryIntoEngine for Stack { type Engine = Stack; fn try_into_engine(&self, _: &()) -> Result<Stack, mod_api::StackWriteError> { VALIDATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed); if self.0 > 250 { Err(mod_api::StackWriteError::Unavailable) } else { Ok(self.clone()) } } }
impl TryIntoApi for Option<Stack> { type Api = Self; fn try_into_api(&self) -> Result<Self, ()> { Ok(self.clone()) } }
impl TryIntoEngine for Option<Stack> { type Engine = Self; fn try_into_engine(&self, items: &()) -> Result<Self, mod_api::StackWriteError> { self.as_ref().map(|s| s.try_into_engine(items)).transpose() } }
impl Stack { fn revision(&self) -> u32 { 0 } }
fn finish_block_entity_writeback<T>(_: &mut Stack, _: &Stack, _: u32, _: &(), _: &str, result: Result<T, BlockEntitySnapshotError>) -> Result<T, BlockEntitySnapshotError> { result }
impl ItemBoundary for Game {}
impl Game {
    fn items(&self) -> &() { &() }
    fn native_generated(&mut self, stack: &mut Stack, handled: &mut bool) { if self.change { stack.0 = 252; } *handled = true; }
    fn native_anvil_update(&mut self, _: u128, _: &mut Stack, _: [i32; 3], stack: &mut Stack, _: &mut bool) { if self.change { stack.0 = 252; } }
    fn native_grindstone_place(&mut self, a: u128, b: &mut Stack, c: [i32; 3], d: &mut Stack, e: &mut bool) { self.native_anvil_update(a,b,c,d,e); }
    fn native_grindstone_take(&mut self, a: u128, b: &mut Stack, c: [i32; 3], d: &mut Stack, e: &mut bool) { self.native_anvil_update(a,b,c,d,e); }
    fn native_menu_button(&mut self, _: &(), _: &mut Stack, stack: &mut Option<Stack>) { if self.change { *stack = Some(Stack(252)); } }
    fn native_menu_quick_move(&mut self, _: [i32; 3], _: &mut Stack, _: usize, _: usize, stack: &mut Stack, handled: &mut bool) { self.native_generated(stack, handled); }
    METHODS
}
fn main() {
    for change in [false, true] {
        let mut game = Game { change };
        let mut stack = Stack(251);
        let mut handled = false;
        game.generated(&mut stack, &mut handled);
        assert_eq!(stack, Stack(251));
        assert_eq!(handled, !change, "untouched argument must not refuse sibling output");
        for (index, fire) in [Game::anvil_update, Game::grindstone_place, Game::grindstone_take, Game::menu_button, Game::menu_quick_move].into_iter().enumerate() {
            let mut value = Stack(251); let mut entity = Stack(0);
            let mut carried = Some(Stack(251)); let mut cancel = false;
            let mut handled = false;
            VALIDATIONS.store(0, std::sync::atomic::Ordering::Relaxed);
            fire(&mut game, &mut value, &mut entity, &mut carried, &mut stack, &mut cancel, &mut handled);
            assert_eq!(VALIDATIONS.load(std::sync::atomic::Ordering::Relaxed), usize::from(change));
            assert_eq!(value, Stack(251)); assert_eq!(stack, Stack(251)); assert_eq!(carried, Some(Stack(251)));
            if index < 3 { assert_eq!(cancel, change); }
            if index == 4 { assert_eq!(handled, !change); }
        }
        let mut cancel = false;
        game.anvil_update(&mut Stack(251), &mut Stack(0), &mut None, &mut Stack(251), &mut cancel, &mut false);
        assert_eq!(cancel, change);
    }
}
"#;
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/untouched-writeback");
    std::fs::create_dir_all(&dir).unwrap();
    let mut variants = vec![
        ("baseline", methods.clone(), true),
        (
            "revalidate",
            methods
                .replace("__api_stack != __original_stack", "true")
                .replace("if api == original", "if false")
                .replace("if cursor == original", "if false")
                .replace("if held == original", "if false"),
            false,
        ),
        (
            "accept_changed",
            methods.replace("__api_stack != __original_stack", "false"),
            false,
        ),
    ];
    for (name, condition) in [
        ("anvil_update", "api == original"),
        ("grindstone_place", "api == original"),
        ("grindstone_take", "api == original"),
        ("menu_button", "cursor == original"),
        ("menu_quick_move", "held == original"),
    ] {
        let start = methods.find(&format!("fn {name}(")).unwrap();
        let end = methods[start + 3..]
            .find("fn ")
            .map_or(methods.len(), |i| start + 3 + i);
        let mut mutant = methods.clone();
        mutant.replace_range(start..end, &methods[start..end].replace(condition, "false"));
        assert_ne!(mutant, methods);
        variants.push((name, mutant, false));
    }
    for (name, methods, success) in variants {
        let source = dir.join(format!("{name}.rs"));
        let binary = dir.join(name);
        std::fs::write(&source, fixture.replace("METHODS", &methods)).unwrap();
        let output = std::process::Command::new("rustc")
            .arg("--edition=2024")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = std::process::Command::new(binary).output().unwrap();
        if name == "baseline" {
            assert_eq!(
                String::from_utf8_lossy(&output.stderr)
                    .matches("mod proof write refused: Unavailable")
                    .count(),
                1
            );
        }
        assert_eq!(
            output.status.success(),
            success,
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
