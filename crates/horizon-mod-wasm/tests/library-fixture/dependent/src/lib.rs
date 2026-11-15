use mod_api::Hooks;

#[derive(Default)]
pub struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let kind = registrar.block_entity_type("dependent").blocks(&[]).build();
        registrar.block_entity_part(&kind, Dependent).unwrap();
    }
}

struct Dependent;
#[mod_api::horizon_part]
impl mod_api::BlockPart for Dependent {}
#[mod_api::horizon_part]
impl mod_api::BlockEntityPart for Dependent {
    fn block_entity_tick(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::BlockEntityTickEvent<'_>,
    ) {
        let owned = wasm_test_library::owned();
        assert_eq!(owned.text, "shared");
        assert_eq!(owned.values, [7, 8]);
        drop(owned);
        let object = wasm_test_library::object();
        assert_eq!(object.answer(), 19);
        assert_eq!(wasm_test_library::generic(41_u32), 42);
        unsafe {
            wasm_test_library::SHARED = 73;
        }
        event.entity.data = vec![
            wasm_test_library::shared_value(),
            unsafe { wasm_test_library::DROPS },
            (&raw mut wasm_test_library::SHARED) as usize as i32,
        ];
    }
}

pub fn on_tick(_: mod_api::HostSide<'_>, args: &mut mod_points::TickArgs) {
    args.cancel = true;
}

#[cfg(test)]
mod tests {
    #[test]
    fn ordinary_rust_library_api() {
        assert_eq!(wasm_test_library::shared_value(), 37);
        assert_eq!(wasm_test_library::generic(41_u32), 42);
        let value = wasm_test_library::owned();
        assert_eq!(value.text, "shared");
        assert_eq!(value.values, [7, 8]);
        drop(value);
        assert_eq!(unsafe { wasm_test_library::DROPS }, 1);
        assert_eq!(wasm_test_library::object().answer(), 19);
    }
}
