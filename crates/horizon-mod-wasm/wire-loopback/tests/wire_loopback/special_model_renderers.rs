use super::*;
use mod_api::__private::game::RegistrationContextNew;
use mod_api::{
    ClientHost, ItemStack, SpecialModelPart, SpecialModelRenderer, SpecialModelRendererRegistry,
    SpecialModelRenderers, Tag,
};
/// Draws `model` once per item of the stack, a block apart, tinted by the config's `tint`;
/// refuses a stack of more than 8.
struct Row(&'static str);
impl SpecialModelRenderer for Row {
    fn extract(
        &self,
        _host: &dyn ClientHost,
        stack: &ItemStack,
        config: &Tag,
    ) -> Result<Vec<SpecialModelPart>, String> {
        if stack.count() > 8 {
            return Err("too many".into());
        }
        let Some(Tag::Int(tint)) = config.get("tint") else {
            return Err("no tint".into());
        };
        Ok((0..stack.count())
            .map(|i| {
                SpecialModelPart::new(self.0)
                    .translated([i as f32, 0.0, 0.0])
                    .with_tints(vec![*tint as u32])
            })
            .collect())
    }
}
pub(super) fn register(renderers: &mut dyn SpecialModelRenderers) {
    renderers.register(
        "wire_test:pan".into(),
        Arc::new(Row("wire_test:item/early")),
    );
    renderers.register(
        "minecraft:bell".into(),
        Arc::new(Row("wire_test:item/bell")),
    );
    // The later registration of a type wins.
    renderers.register("wire_test:pan".into(), Arc::new(Row("wire_test:item/pan")));
}
struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn register_special_model_renderer(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterSpecialModelRendererEvent<'_>,
    ) {
        register(event.renderers);
    }
}
#[test]
fn wire_loopback_special_model_renderers_register_and_answer_alike_for_native_and_guest_mods() {
    struct Client;
    impl mod_api::Host for Client {}
    mod_api::impl_host_areas!(Client);
    impl mod_api::ServerHost for Client {}
    mod_api::impl_server_host_areas!(Client);
    impl ClientHost for Client {}
    mod_api::impl_client_host_areas!(Client);
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    install_counter();
    let stack = |count| ItemStack::new(REGISTERED_ITEM.get().unwrap(), count).unwrap();
    let config = Tag::compound([("tint", Tag::Int(0x7F11_2233))]);
    let mut answers = Vec::new();
    for guest in [false, true] {
        let hooks: Box<dyn Hooks> = if guest {
            Box::new(WasmMod {
                group: 0,
                subscribed: &[Some(mod_api::HookId::RegisterSpecialModelRenderer)],
                listeners: &[],
                corrupt: false,
            })
        } else {
            Box::new(Native)
        };
        mod_api::testing::with_mods(
            vec![mod_api::ModEntry::new("wire_test", hooks)].into(),
            || {
                let mut registry = SpecialModelRendererRegistry::default();
                mod_api::dispatch_register_special_model_renderer(
                    &mod_api::RegistrationContext::new(),
                    &mut registry,
                );
                let (renderers, replaced) = registry.into_renderers();
                assert_eq!(
                    renderers.keys().collect::<Vec<_>>(),
                    ["minecraft:bell", "wire_test:pan"],
                    "guest {guest}"
                );
                assert_eq!(replaced.len(), 1, "guest {guest}");
                assert_eq!(replaced[0].kind, "wire_test:pan");
                let parts = renderers["wire_test:pan"]
                    .extract(&Client, &stack(2), &config)
                    .unwrap();
                assert_eq!(
                    parts,
                    [
                        SpecialModelPart::new("wire_test:item/pan").with_tints(vec![0x7F11_2233]),
                        SpecialModelPart::new("wire_test:item/pan")
                            .translated([1.0, 0.0, 0.0])
                            .with_tints(vec![0x7F11_2233]),
                    ],
                    "guest {guest}"
                );
                let refused = renderers["minecraft:bell"].extract(&Client, &stack(9), &config);
                assert_eq!(refused, Err("too many".into()), "guest {guest}");
                if guest {
                    // A guest call whose reply doesn't decode (as a refused or trapped call
                    // leaves none) is an `Err`, never a silent `Ok`: the game then draws the
                    // missing model.
                    APPEND_ATTACK.set(3);
                    let trapped = renderers["wire_test:pan"].extract(&Client, &stack(2), &config);
                    APPEND_ATTACK.set(0);
                    assert!(
                        trapped
                            .as_ref()
                            .is_err_and(|e| e.starts_with("wasm call refused or trapped")),
                        "{trapped:?}"
                    );
                }
                answers.push(parts);
            },
        );
    }
    assert_eq!(answers[0], answers[1]);
}

pub(super) struct SpecialModelRenderersHooks;
impl SpecialModelRenderersHooks {
    pub(super) fn register_special_model_renderer(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterSpecialModelRendererEvent<'_>,
    ) {
        self::register(event.renderers);
    }
}
