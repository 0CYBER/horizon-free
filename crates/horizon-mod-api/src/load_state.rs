//! Platform-owned phase machine and load snapshots.
use crate::__private::game::Fields;
#[cfg(test)]
use crate::__private::game::{RegistrationContextNew, ServerReloadListenersOrder};
use crate::*;
#[cfg(test)]
use mod_vocab::__private::{LoadAccess, check_access};
use mod_vocab::__private::{ModPhase, PhaseState, RegistrationData};
#[cfg(test)]
use mod_vocab::__private::{PhaseError, PhaseOwner};
#[cfg(test)]
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
thread_local! {
    static PHASE: std::cell::Cell<PhaseState> = const { std::cell::Cell::new(PhaseState {
        phase: if cfg!(test) { ModPhase::World } else { ModPhase::Packages }, resume: None,
    }) };
}

/// The phase of this thread, used in refusal diagnostics.
pub fn current_phase() -> ModPhase {
    PHASE.with(|cell| cell.get().phase)
}

static PACKAGES: OnceLock<Arc<[PackageInfo]>> = OnceLock::new();

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static TEST_PACKAGES: std::cell::RefCell<Option<Arc<[PackageInfo]>>> = const { std::cell::RefCell::new(None) };
}
/// Runs `f` with only the packages `ids` installed, for tests.
#[cfg(any(test, feature = "testing"))]
pub fn with_test_packages<R>(ids: &[&str], f: impl FnOnce() -> R) -> R {
    let packages: Vec<_> = ids
        .iter()
        .map(|id| {
            PackageInfo::new(
                (*id).to_owned(),
                "1".to_owned(),
                PackageKind::Code,
                crate::Side::Client,
                vec![(*id).to_owned()],
                vec![],
            )
        })
        .collect();
    with_test_package_inventory(packages, f)
}

#[cfg(any(test, feature = "testing"))]
pub(crate) fn with_test_package_inventory<R>(
    packages: Vec<PackageInfo>,
    f: impl FnOnce() -> R,
) -> R {
    let previous = TEST_PACKAGES.replace(Some(packages.into()));
    let _ = crate::__private::initialize();
    let result = f();
    TEST_PACKAGES.set(previous);
    result
}

fn inventory(native: &[PackageInfo], content: &[PackageInfo]) -> Vec<PackageInfo> {
    let mut packages = native.to_vec();
    for selected in content {
        if !packages.iter().any(|p| p.id() == selected.id()) {
            packages.push(selected.clone());
        }
    }
    // The code inventory follows the one installed native/wasm dispatch slice;
    // content-only packages remain after it in their selected layer order.
    packages.sort_by_key(|p| {
        crate::__private::mods()
            .iter()
            .position(|entry| entry.fields().id == p.id())
            .unwrap_or(usize::MAX)
    });
    packages
}

/// Records the package inventory once at startup; a second call panics.
pub fn install_packages(native: &[PackageInfo], content: &[PackageInfo]) {
    let _ = crate::__private::initialize();
    assert!(
        PACKAGES.set(inventory(native, content).into()).is_ok(),
        "package inventory already installed"
    );
}

/// Install the host's ordered package inventory in a guest at initialization.
// hidden: the guest runtime installs this once before dispatching hooks.
#[doc(hidden)]
pub fn install_guest_packages(packages: Vec<PackageInfo>) {
    let _ = crate::__private::initialize();
    let _ = PACKAGES.set(packages.into());
}

static REGISTRATION_DATA: OnceLock<RegistrationData> = OnceLock::new();

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static TEST_REGISTRATION_DATA: std::cell::RefCell<Option<RegistrationData>> = const { std::cell::RefCell::new(None) };
}
/// Runs `f` with `data` as the registration data, for tests.
#[cfg(any(test, feature = "testing"))]
pub fn with_test_registration_data<R>(data: RegistrationData, f: impl FnOnce() -> R) -> R {
    let _ = crate::__private::initialize();
    TEST_REGISTRATION_DATA.set(Some(data));
    let result = f();
    TEST_REGISTRATION_DATA.set(None);
    result
}

pub(crate) fn registration_data() -> Option<RegistrationData> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(data) = TEST_REGISTRATION_DATA.with_borrow(Clone::clone) {
        return Some(data);
    }
    REGISTRATION_DATA.get().cloned()
}

/// Records the registries and settings registration reads, once at startup, after the
/// package inventory and before the first registration hook; a second call panics.
pub fn install_registration_data(data: RegistrationData) {
    let _ = crate::__private::initialize();
    assert!(
        REGISTRATION_DATA.set(data).is_ok(),
        "registration data already installed"
    );
}

pub(crate) fn current_packages() -> Arc<[PackageInfo]> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(packages) = TEST_PACKAGES.with_borrow(Clone::clone) {
        return packages;
    }
    PACKAGES.get().cloned().unwrap_or_else(|| Arc::from([]))
}
pub(crate) fn phase_snapshot() -> PhaseState {
    PHASE.with(std::cell::Cell::get)
}
pub(crate) fn set_phase(state: PhaseState) {
    PHASE.set(state);
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestReader {
    pub registries: std::collections::BTreeMap<String, Arc<[String]>>,
    pub tags: Arc<[(String, Arc<[String]>)]>,
}
#[cfg(test)]
impl LoadReader for TestReader {
    fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.registries.get(name).cloned()
    }
    fn fluid_tags(&self) -> Arc<[(String, Arc<[String]>)]> {
        self.tags.clone()
    }
    fn setting(&self, _: &str, _: &str) -> Option<String> {
        None
    }
}
#[cfg(test)]
impl LoopReader for TestReader {
    fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.registries.get(name).cloned()
    }
    fn setting(&self, _: &str, _: &str) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_dispatch_slice_keeps_order_in_registration_config_and_reload() {
        crate::__private::initialize().unwrap();
        use crate::*;
        use std::sync::Mutex;
        struct Trace {
            id: &'static str,
            calls: Arc<Mutex<Vec<(&'static str, &'static str)>>>,
        }
        #[crate::__subscriptions]
        impl Hooks for Trace {
            fn register(&self, _: &mut dyn Registrar, _: &RegistrationContext) {
                assert_eq!(current_phase(), ModPhase::Registration);
                self.calls.lock().unwrap().push(("register", self.id));
            }
            fn config_loading(&self, _: HostSide<'_>, _: &mut ConfigLoadingEvent<'_>) {
                assert_eq!(current_phase(), ModPhase::Config);
                self.calls.lock().unwrap().push(("config", self.id));
            }
            fn add_server_reload_listeners(
                &self,
                _: &dyn Host,
                _: &mut AddServerReloadListenersEvent<'_>,
            ) {
                assert_eq!(current_phase(), ModPhase::Reloading);
                self.calls.lock().unwrap().push(("reload", self.id));
            }
        }
        struct Server;
        impl Host for Server {}
        mod_api::impl_host_areas!(Server);
        impl ServerHost for Server {}
        mod_api::impl_server_host_areas!(Server);
        // The loader supplies a dependency-resolved slice; phase dispatch must retain it.
        let order = [1, 0, 2];
        let calls = Arc::new(Mutex::new(Vec::new()));
        let ids = ["z", "lib", "a"];
        let mods: Mods = order
            .into_iter()
            .map(|i| {
                ModEntry::new(
                    ids[i],
                    Box::new(Trace {
                        id: ids[i],
                        calls: calls.clone(),
                    }),
                )
            })
            .collect::<Vec<_>>()
            .into();
        crate::__private::install(mods.clone());
        let owner = PhaseOwner::new(ModPhase::Packages);
        owner.advance(ModPhase::Registration).unwrap();
        crate::__private::install_parts(
            &mods,
            &crate::__private::part_tests::rule_schema(),
            |_| false,
        )
        .unwrap();
        owner.advance(ModPhase::Setup).unwrap();
        let mut host = Server;
        for _world in 0..2 {
            {
                let _config = owner.config().unwrap();
                dispatch_config_loading(
                    HostSide::Server(&mut host),
                    &crate::__private::built::ConfigFile {
                        mod_id: "lib".into(),
                        kind: ConfigKind::Server,
                        file: "lib-server.toml".into(),
                        text: String::new(),
                    }
                    .into(),
                );
            }
            for _reload in 0..2 {
                let _reload = owner.reloading().unwrap();
                dispatch_add_server_reload_listeners(&host, &mut ServerListenerRegistry::new());
            }
        }
        let expected = [
            "register", "config", "reload", "reload", "config", "reload", "reload",
        ]
        .into_iter()
        .flat_map(|moment| ["lib", "z", "a"].map(|id| (moment, id)))
        .collect::<Vec<_>>();
        assert_eq!(*calls.lock().unwrap(), expected);
    }

    #[test]
    fn phase_edges_are_exhaustive_and_refusal_leaves_state_unchanged() {
        crate::__private::initialize().unwrap();
        use ModPhase::*;
        let phases = [Packages, Registration, Setup, Config, World, Reloading];
        let edges = [
            (Packages, Registration),
            (Registration, Setup),
            (Setup, Config),
            (Config, World),
            (World, Config),
            (Setup, Reloading),
            (World, Reloading),
        ];
        for from in phases {
            for to in phases {
                let owner = PhaseOwner::new(from);
                let result = owner.advance(to);
                assert_eq!(
                    result.is_ok(),
                    edges.contains(&(from, to)),
                    "{from:?} -> {to:?}"
                );
                assert_eq!(current_phase(), if result.is_ok() { to } else { from });
            }
        }
        let owner = PhaseOwner::new(Packages);
        let other = owner.clone();
        assert_eq!(
            std::thread::spawn(move || other.advance(Registration))
                .join()
                .unwrap(),
            Err(PhaseError::WrongThread)
        );
        assert_eq!(current_phase(), Packages);
    }

    #[test]
    fn reloads_restore_setup_or_world_and_world_config_can_repeat() {
        crate::__private::initialize().unwrap();
        use ModPhase::*;
        let owner = PhaseOwner::new(Packages);
        owner.advance(Registration).unwrap();
        owner.advance(Setup).unwrap();
        {
            let _reload = owner.reloading().unwrap();
            assert_eq!(current_phase(), Reloading);
            assert!(owner.reloading().is_err());
        }
        assert_eq!(current_phase(), Setup);
        assert!(owner.advance(Registration).is_err());
        for _world in 0..2 {
            {
                let _config = owner.config().unwrap();
                assert_eq!(current_phase(), Config);
                check_access("setting", LoadAccess::WorldOnly);
            }
            assert_eq!(current_phase(), World);
            for _reload in 0..2 {
                let _ = std::panic::catch_unwind(|| {
                    let _reload = owner.reloading().unwrap();
                    assert_eq!(current_phase(), Reloading);
                    check_access("block_state_default", LoadAccess::Metadata);
                    panic!("failed prepare");
                });
                assert_eq!(current_phase(), World);
            }
        }
        {
            let _loader = PhaseOwner::scope(Setup);
            assert_eq!(current_phase(), Setup);
        }
        assert_eq!(current_phase(), World);
    }

    #[test]
    fn fluid_state_facts_follow_metadata_access() {
        crate::__private::initialize().unwrap();
        use ModPhase::*;
        for phase in [Packages, Registration, Setup, Config, Reloading, World] {
            let _owner = PhaseOwner::new(phase);
            for operation in [
                "fluid_legacy_block",
                "fluid_state_definition",
                "fluid_state_validate",
                "fluid_type_properties",
            ] {
                let result = std::panic::catch_unwind(|| {
                    check_access(operation, LoadAccess::Metadata);
                });
                assert_eq!(result.is_ok(), matches!(phase, Setup | Reloading | World));
            }
        }
    }

    #[test]
    fn early_world_handle_and_transaction_calls_panic_naming_the_phase() {
        crate::__private::initialize().unwrap();
        use crate::{Host, HostTransaction, ServerHost, prelude::*};
        struct HostDouble;
        impl Host for HostDouble {}
        mod_api::impl_host_areas!(HostDouble);
        impl ServerHost for HostDouble {}
        mod_api::impl_server_host_areas!(HostDouble);
        for phase in [
            ModPhase::Registration,
            ModPhase::Config,
            ModPhase::Reloading,
            ModPhase::Setup,
        ] {
            let _owner = PhaseOwner::new(phase);
            for action in [0, 1, 2, 3] {
                let panic = std::panic::catch_unwind(|| {
                    let mut host = HostDouble;
                    match action {
                        0 => {
                            let _ = host.entity(1);
                        }
                        1 => {
                            let _ = host.level("minecraft:overworld");
                        }
                        2 => {
                            let _ = host.player(1);
                        }
                        _ => {
                            let _: Result<(), ()> = host.transaction(|_, _| Ok(()));
                        }
                    }
                })
                .expect_err("early access must panic, not return a refusal");
                let message = panic.downcast_ref::<String>().unwrap();
                assert!(message.contains(&format!("{phase:?}")), "{message}");
            }
        }
        let _owner = PhaseOwner::new(ModPhase::Registration);
        let refused = std::panic::catch_unwind(|| HostDouble.block_state_id("stone")).unwrap_err();
        assert!(
            refused
                .downcast_ref::<String>()
                .unwrap()
                .contains("Registration")
        );
    }

    #[test]
    fn inventory_keeps_first_content_package_after_code_mods() {
        crate::__private::initialize().unwrap();
        let native = [PackageInfo::new(
            "runtime".into(),
            "3".into(),
            PackageKind::Code,
            crate::Side::Client,
            vec!["runtime".into()],
            vec![],
        )];
        let package = |version: &str| {
            PackageInfo::new(
                "plain".into(),
                version.into(),
                PackageKind::Content,
                crate::Side::Client,
                vec!["plain".into()],
                vec![],
            )
        };
        let entries = inventory(&native, &[package("1"), package("2")]);
        TEST_PACKAGES.set(Some(entries.into()));
        let view = RegistrationContext::new().mods().clone();
        TEST_PACKAGES.set(None);
        assert_eq!(view.ids().collect::<Vec<_>>(), ["runtime", "plain"]);
        let runtime = view.get("runtime").unwrap();
        assert_eq!(runtime.kind(), PackageKind::Code);
        assert_eq!(runtime.version(), "3");
        assert_eq!(runtime.side(), crate::Side::Client);
        let plain = view.get("plain").unwrap();
        assert_eq!(plain.kind(), PackageKind::Content);
        assert_eq!(plain.version(), "1");
        assert_eq!(plain.side(), crate::Side::Client);
        assert_eq!(plain.namespaces().collect::<Vec<_>>(), ["plain"]);
        assert!(!plain.depends_on("runtime"));
        assert!(view.get("missing").is_none());
    }

    #[test]
    fn a_loop_context_reads_its_side_registries_and_mods() {
        crate::__private::initialize().unwrap();
        let source = TestReader {
            registries: std::collections::BTreeMap::from([(
                "item".to_owned(),
                vec!["a:b".into()].into(),
            )]),
            tags: Arc::from([]),
        };
        with_test_packages(&["one", "two"], || {
            let context = OwningLoop::new(LoopSide::Client, &source);
            assert_eq!(context.side(), LoopSide::Client);
            assert_eq!(
                context.registry("item").as_deref(),
                Some(&["a:b".to_owned()][..])
            );
            assert_eq!(context.registry("block"), None);
            assert_eq!(context.setting("one", "mode"), None);
            assert_eq!(context.mods().ids().collect::<Vec<_>>(), ["one", "two"]);
        });
    }

    #[test]
    fn registration_hooks_read_one_snapshot_and_refuse_bad_reads() {
        crate::__private::initialize().unwrap();
        use crate::*;
        use std::sync::Mutex;

        /// Every answer the context gives this hook, for the keys the test names.
        fn probe(cx: &RegistrationContext) -> Vec<String> {
            let registry = |name| format!("{:?}", cx.registry(name));
            let setting = |id, key| format!("{:?}", cx.setting(id, key));
            vec![
                format!(
                    "{:?}",
                    cx.instance_dir()
                        .map(|dir| dir == &crate::InstanceDirectory::Unix(vec![47, 255]))
                ),
                registry("minecraft:item"),
                registry("minecraft:missing"),
                registry("item"),
                registry(""),
                setting("reader", "mode"),
                setting("reader", "unset"),
                setting("other", "mode"),
                setting("absent", "mode"),
                setting("reader", ""),
                setting("reader", ".."),
                setting("reader", "../mode"),
                setting("reader", "a\\b"),
            ]
        }
        struct Reader(Arc<Mutex<Vec<String>>>);
        #[crate::__subscriptions]
        impl Hooks for Reader {
            fn register(&self, _: &mut dyn Registrar, cx: &RegistrationContext) {
                *self.0.lock().unwrap() = probe(cx);
            }
            fn component_types(
                &self,
                cx: &RegistrationContext,
                event: &mut ComponentTypesEvent<'_>,
            ) {
                *event.types = probe(cx);
            }
        }

        let data = RegistrationData::new(
            BTreeMap::from([(
                "minecraft:item".to_owned(),
                vec!["minecraft:air".to_owned(), "minecraft:stone".to_owned()],
            )]),
            BTreeMap::from([
                (
                    "reader".to_owned(),
                    BTreeMap::from([("mode".to_owned(), "fast".to_owned())]),
                ),
                (
                    "other".to_owned(),
                    BTreeMap::from([("mode".to_owned(), "slow".to_owned())]),
                ),
            ]),
            crate::InstanceDirectory::Unix(vec![47, 255]),
            crate::RuntimeEnvironment::new(
                crate::Dist::Client,
                "26.3".into(),
                "test".into(),
                false,
            ),
        );

        let registered = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new(
            "reader",
            Box::new(Reader(registered.clone())),
        )]
        .into();
        crate::__private::install(mods.clone());
        let expected = [
            "Ok(true)",
            r#"Ok(["minecraft:air", "minecraft:stone"])"#,
            r#"Err(UnknownRegistry("minecraft:missing"))"#,
            r#"Err(InvalidRegistryName("item"))"#,
            r#"Err(InvalidRegistryName(""))"#,
            r#"Ok(Some("fast"))"#,
            "Ok(None)",
            r#"Ok(Some("slow"))"#,
            r#"Err(UnknownMod("absent"))"#,
            r#"Err(InvalidSettingKey(""))"#,
            r#"Err(InvalidSettingKey(".."))"#,
            r#"Err(InvalidSettingKey("../mode"))"#,
            r#"Err(InvalidSettingKey("a\\b"))"#,
        ];
        with_test_packages(&["reader", "other"], || {
            with_test_registration_data(data, || {
                let cx = RegistrationContext::new();
                assert_eq!(cx.dist(), Dist::Client);
                assert_eq!(cx.game_version(), "26.3");
                assert_eq!(cx.horizon_version(), "test");
                assert!(!cx.game_tests_enabled());
                crate::__private::install_parts(
                    &mods,
                    &crate::__private::part_tests::rule_schema(),
                    |_| false,
                )
                .unwrap();
                assert_eq!(*registered.lock().unwrap(), expected);
                let mut types = Vec::new();
                dispatch_component_types(&RegistrationContext::new(), &mut types);
                assert_eq!(types, expected, "a later step reads the same snapshot");
            });
            // No engine view: every read is refused, and a bad name or key still says so.
            crate::__private::install_parts(
                &mods,
                &crate::__private::part_tests::rule_schema(),
                |_| false,
            )
            .unwrap();
            let refused = registered.lock().unwrap().clone();
            assert_eq!(refused[0], "Err(Unavailable)");
            assert_eq!(refused[1], "Err(Unavailable)");
            assert_eq!(refused[3], r#"Err(InvalidRegistryName("item"))"#);
            assert_eq!(refused[5], "Err(Unavailable)");
            assert_eq!(refused[9], r#"Err(InvalidSettingKey(""))"#);
        });
    }
}
