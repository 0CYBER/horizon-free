use super::*;
use mod_api::__private::game::BackendRefOf;
use mod_api::prelude::*;

#[test]
fn int_game_rule_declarations_and_refusals_round_trip() {
    for default in [
        mod_api::GameRuleDefault::Bool(true),
        mod_api::GameRuleDefault::Int {
            default: 8,
            min: 0,
            max: 64,
        },
        mod_api::GameRuleDefault::Int {
            default: i32::MIN,
            min: i32::MIN,
            max: i32::MAX,
        },
    ] {
        let mut out = Writer::new();
        out.put(&default);
        assert_eq!(
            Reader::decode::<mod_api::GameRuleDefault>(out.bytes()).unwrap(),
            default
        );
    }
    let mut out = Writer::new();
    out.put(&mod_api::GameRuleError::InvalidRange);
    assert_eq!(
        Reader::decode::<mod_api::GameRuleError>(out.bytes()).unwrap(),
        mod_api::GameRuleError::InvalidRange
    );
}

static CAP: std::sync::OnceLock<mod_api::IntGameRuleKey> = std::sync::OnceLock::new();

struct Register;
#[mod_api::__subscriptions]
impl Hooks for Register {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        registrar.game_rule_category("machines").unwrap();
        let key = registrar
            .game_rule_int("cap", "example:machines", 8, 0, 64)
            .unwrap();
        let _ = CAP.set(key);
    }
}

/// A server whose `example:cap` is 12 in the level of epoch 7.
struct World;
impl mod_api::Backend for World {}
mod_api::impl_backend_areas!(World, except(levels));
impl LevelsBackend for World {
    fn level_game_rule_int(
        &self,
        level: mod_api::LevelRef,
        key: mod_api::IntGameRuleKey,
    ) -> Option<i32> {
        (level.epoch() == mod_api::WorldEpoch::new(7) && Some(&key) == CAP.get()).then_some(12)
    }
}
impl mod_api::Host for World {
    fn backend(&self) -> Option<mod_api::BackendRef<'_>> {
        Some(mod_api::BackendRef::of(self))
    }
}
mod_api::impl_host_areas!(World);

#[test]
fn a_wasm_mod_reads_its_int_game_rule_by_key() {
    mod_api::__private::initialize().unwrap();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("example", Box::new(Register))].into();
    mod_api::testing::with_mods(mods.clone(), || {
        mod_api::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let decl = mod_api::__private::game_rules()
            .iter()
            .find(|rule| rule.name == "example:cap")
            .unwrap();
        let mut out = Writer::new();
        out.put(decl);
        assert_eq!(
            &Reader::decode::<mod_api::GameRuleDecl>(out.bytes()).unwrap(),
            decl
        );
        let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
        let world = World;
        let mut frame = boundary::Frame::default();
        frame.set_host(&world);
        let mut cx = boundary::Cx::default();
        cx.frames.push(frame);
        let cx = std::cell::RefCell::new(cx);
        guest::set_transport(Some(Rc::new(move |id, bytes| {
            let mut out = Writer::new();
            // SAFETY: world and its frame remain live for this synchronous transport.
            unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
            out.into_bytes()
        })));
        let key = *CAP.get().unwrap();
        for (epoch, expected) in [(7, Some(12)), (8, None)] {
            let level = mod_api::LevelRef::new(
                mod_api::WorldEpoch::new(epoch),
                mod_api::DimensionKey::OVERWORLD,
            );
            assert_eq!(level.game_rule_int(&guest::Guest, key), expected);
        }
        guest::set_transport(None);
    });
}
