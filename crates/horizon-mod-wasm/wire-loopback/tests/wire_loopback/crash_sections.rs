use super::*;
struct Caches;
impl mod_api::CrashSection for Caches {
    fn value(&self) -> Option<String> {
        Some("3 caches".into())
    }
}
pub(super) fn register(registrar: &mut dyn mod_api::Registrar) {
    registrar.crash_section("caches", Arc::new(Caches));
}
#[test]
fn wire_loopback_a_guest_crash_section_answers_through_its_proxy() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    install_counter();
    let lines = mod_api::__private::game::mod_crash_lines();
    let (owner, _, line) = lines
        .iter()
        .find(|(_, label, _)| label.as_deref() == Some("caches"))
        .expect("the guest's crash section reached the registrar");
    assert_eq!(*owner, "wire_test");
    assert_eq!(line.value().as_deref(), Some("3 caches"));
}

pub(super) struct CrashSectionsHooks;
impl CrashSectionsHooks {
    pub(super) fn register(
        &self,
        registrar: &mut dyn mod_api::Registrar,
        cx: &mod_api::RegistrationContext,
    ) {
        self::register(registrar);
    }
}
