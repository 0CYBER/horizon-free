use super::*;

#[test]
fn read_encoding_keeps_nonresident_stack_limits() {
    let items = super::stack_limit_tests::catalog();
    let key = mod_api::__private::item_key("minecraft:iron_sword").unwrap();
    let mut damaged = ApiStack::of(key);
    damaged
        .set_components([(JavaString::from("minecraft:max_damage"), Tag::Int(250))].into())
        .unwrap();
    damaged.set_damage(251);
    let mut removed = ApiStack::of(key);
    removed
        .set_components(
            [(
                JavaString::from("!minecraft:max_damage"),
                Tag::Compound(Default::default()),
            )]
            .into(),
        )
        .unwrap();
    removed.set_damage(1);
    let mut excess = ApiStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
    excess.set_count(65).unwrap();
    for api in [damaged, removed, excess] {
        assert!(api.try_into_engine(&items).is_err(), "write remains strict");
        let encoded = stack_value(&api).unwrap();
        assert_eq!(encoded.count, api.count());
        assert_eq!(encoded.damage, api.damage());
        assert_eq!(encoded.try_into_api().unwrap(), api);
    }
}

// Exercise the actual adapter bodies with a command-recording engine. Real stack
// encoding is covered above; here a strict-conversion mutant must suppress the
// predicate/draw command and fail independently at each handwritten consumer.
#[test]
#[ignore = "daily"]
fn read_consumers_forward_nonresident_stacks_and_mutants() {
    fn body(source: &str, marker: &str) -> String {
        let source = &source[source.find(marker).unwrap()..];
        let start = source.find('{').unwrap();
        let mut depth = 1;
        let end = source[start + 1..]
            .char_indices()
            .find_map(|(i, c)| {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                (depth == 0).then_some(start + 1 + i)
            })
            .unwrap();
        source[start + 1..end].into()
    }
    let draw = include_str!("../../host/draw.rs");
    let block = &draw[draw.find("impl BlockEntityDrawContext").unwrap()..];
    let tooltip = include_str!("../../blocks/stations/tooltip.rs");
    let methods = [
        (
            "may_place",
            body(
                include_str!("../api_values.rs"),
                "fn may_place(&self, stack: &ApiStack)",
            ),
        ),
        ("item_stack", body(draw, "fn item_stack(")),
        ("gui_draw_item_with", body(draw, "fn gui_draw_item_with(")),
        ("block_item", body(block, "fn item(")),
        ("tooltip", body(tooltip, "fn draw_tooltip_item_part(")),
    ];
    let fixture = r#"
#![allow(unused)]
#[derive(Clone, Debug, PartialEq)] struct Stack { damage: u32, max_damage: u32, count: u32, max_count: u32 }
impl Stack { fn try_into_engine(&self, _: &()) -> Result<Self, ()> { if self.damage > self.max_damage || self.count > self.max_count { Err(()) } else { Ok(self.clone()) } } }
pub(crate) fn stack_value(stack: &Stack) -> Result<Stack, ()> { Ok(stack.clone()) }
mod game { pub mod api_values { pub(crate) use crate::stack_value; } }
struct Display;
impl Display { fn try_into_engine(&self, _: &()) -> Result<(), ()> { Ok(()) } }
struct Engine;
impl Engine {
    fn items(&self) -> &() { &() }
    fn may_place(&self, _: &Stack) -> bool { true }
    fn native_item_stack(&self, out: &mut Vec<Stack>, stack: &Stack, _: [f32; 2], _: f32) -> bool { out.push(stack.clone()); true }
    fn native_gui_draw_item_with(&self, out: &mut Vec<Stack>, stack: &Stack, _: [f32; 2], _: Option<&str>) { out.push(stack.clone()); }
    fn native_draw_item(&self, out: &mut Vec<Stack>, stack: &Stack, _: (), _: Option<f32>, _: i32) -> bool { out.push(stack.clone()); true }
    fn native_tooltip_component_draw(&self, stack: &Stack, out: &mut Vec<Stack>, _: [f32; 2], _: [f32; 2]) { out.push(stack.clone()); }
}
struct Context<'a> { game: &'a Engine, engine: &'a Engine, surface: &'a mut Vec<Stack> }
impl Context<'_> {
    fn fire(&mut self, stack: &Stack) -> bool {
        let at = [0.; 2]; let size = 16.; let text: Option<&str> = None;
        let display = Display; let owner_yaw: Option<f32> = None; let seed = 0;
        BODY
    }
}
impl Engine {
    fn tooltip(&self, stack: &Stack, batch: &mut Vec<Stack>) {
        let origin = [0.; 2]; let size = [70., 46.];
        TOOLTIP
    }
}
fn main() {
    for stack in [Stack { damage: 251, max_damage: 250, count: 1, max_count: 1 }, Stack { damage: 1, max_damage: 0, count: 1, max_count: 1 }, Stack { damage: 0, max_damage: 0, count: 65, max_count: 64 }] {
        let engine = Engine; let mut out = vec![];
        assert!(Context { game: &engine, engine: &engine, surface: &mut out }.fire(&stack));
        CHECK
    }
}
"#;
    let scratch = tempfile::tempdir().unwrap();
    let dir = scratch.path();
    for (name, original) in methods {
        for mutant in [false, true] {
            let adapted = original.replace("return;", "return false;");
            let (code, check, tooltip) = match name {
                "may_place" => (adapted, "assert!(out.is_empty());", String::new()),
                "tooltip" => (
                    "self.game.tooltip(stack, &mut *self.surface); true".into(),
                    "assert_eq!(out, vec![stack.clone()]);",
                    original.clone(),
                ),
                "gui_draw_item_with" => (
                    format!("{adapted}; true"),
                    "assert_eq!(out, vec![stack.clone()]);",
                    String::new(),
                ),
                _ => (
                    adapted,
                    "assert_eq!(out, vec![stack.clone()]);",
                    String::new(),
                ),
            };
            let source = fixture
                .replace("BODY", &code)
                .replace("TOOLTIP", &tooltip)
                .replace("CHECK", check);
            let source = if mutant {
                source.replace(
                    "stack_value(stack)",
                    if name == "tooltip" {
                        "stack.try_into_engine(self.items())"
                    } else {
                        "stack.try_into_engine(self.game.items())"
                    },
                )
            } else {
                source
            };
            // Qualifying replacement above leaves a path before the expression; remove it.
            let source = source.replace(
                "crate::game::api_values::stack.try_into_engine",
                "stack.try_into_engine",
            );
            let input = dir.join(format!("{name}-{mutant}.rs"));
            let binary = dir.join(format!("{name}-{mutant}"));
            std::fs::write(&input, source).unwrap();
            let compiled = std::process::Command::new("rustc")
                .args(["--edition=2024", "--crate-name=read_consumer"])
                .arg(&input)
                .arg("-o")
                .arg(&binary)
                .output()
                .unwrap();
            assert!(
                compiled.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&compiled.stderr)
            );
            let result = std::process::Command::new(binary).output().unwrap();
            assert_eq!(
                result.status.success(),
                !mutant,
                "{name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}
