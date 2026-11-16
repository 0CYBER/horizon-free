#![cfg(test)]
use crate::game::part_admission::admit_parts;
use mod_api::*;

struct First;
#[mod_api::horizon_part]
impl BlockPart for First {}
#[mod_api::horizon_part]
impl BlockEntityPart for First {}
struct Second;
#[mod_api::horizon_part]
impl BlockPart for Second {}
#[mod_api::horizon_part]
impl BlockEntityPart for Second {}

struct Attach(bool);
#[mod_api::__subscriptions]
impl Hooks for Attach {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let kind = r.block_entity_type_key("demo:stove");
        r.block_entity_part(&kind, First).unwrap();
        if self.0 {
            r.block_entity_part(&kind, Second).unwrap();
        }
    }
}

fn refusal(two: bool) -> String {
    let mods: Mods = vec![ModEntry::new("demo", Box::new(Attach(two)))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        admit_parts(&mods).unwrap_err().to_string()
    })
}

#[test]
fn undeclared_part_returns_load_error_without_panicking() {
    let error = refusal(false);
    assert!(error.starts_with("Mod parts refused:\n"));
    assert!(error.contains("undeclared native block entity type demo:stove"));
    assert_eq!(error.lines().count(), 2);
}

#[test]
fn two_refused_parts_preserve_messages_and_order() {
    let error = refusal(true);
    assert_eq!(
        error,
        format!(
            "Mod parts refused:\nmod demo attached {} to undeclared native block entity type demo:stove\nmod demo attached {} to undeclared native block entity type demo:stove",
            std::any::type_name::<First>(),
            std::any::type_name::<Second>()
        )
    );
}

#[test]
fn no_refusals_admits_parts() {
    let mods: Mods = Vec::new().into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        assert!(admit_parts(&mods).is_ok());
    });
}
