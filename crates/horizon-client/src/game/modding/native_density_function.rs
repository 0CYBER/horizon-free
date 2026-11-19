//! Native density function adapter: the mods' `DensityFunctionType`s as the engine's.
use horizon_client::crash::{ModCall, ModGuard, enter_native_mod};
use horizon_world::worldgen::mod_density_functions::{
    ModDensityFunction, ModDensityFunctionType, ModDensityFunctionTypes,
    install_mod_density_function_types,
};
use mod_api::{DensityFunction, DensityFunctionType};
use std::sync::Arc;

/// Who a call into a type's code belongs to: a panic in it is not caught (a density function must
/// not panic) and takes the game's crash path, whose report names the mod, the method and the
/// type. Entering is a thread-local mark, with no allocation, so it is fine per call.
#[derive(Clone, Copy)]
struct Owner {
    mod_id: &'static str,
    id: &'static str,
}
impl Owner {
    fn enter(self, hook: &'static str) -> ModGuard {
        enter_native_mod(ModCall {
            id: self.mod_id,
            hook,
            part: Some(self.id),
        })
    }
}

struct TypeAdapter {
    owner: Owner,
    density_type: Arc<dyn DensityFunctionType>,
}
impl ModDensityFunctionType for TypeAdapter {
    fn owner(&self) -> &str {
        self.owner.mod_id
    }
    fn decode(&self, json: &str) -> Result<Arc<dyn ModDensityFunction>, String> {
        let function = {
            let _call = self.owner.enter("decode");
            self.density_type.decode(json)?
        };
        Ok(Arc::new(FunctionAdapter {
            owner: self.owner,
            function,
        }))
    }
}

struct FunctionAdapter {
    owner: Owner,
    function: Arc<dyn DensityFunction>,
}
impl ModDensityFunction for FunctionAdapter {
    fn range(&self) -> [f32; 2] {
        let _call = self.owner.enter("range");
        self.function.range()
    }
    fn varies_with_y(&self) -> bool {
        let _call = self.owner.enter("varies_with_y");
        self.function.varies_with_y()
    }
    fn sample(&self, position: [i32; 3], seed: i64) -> f32 {
        let _call = self.owner.enter("sample");
        self.function.sample(position, seed)
    }
    fn fill(
        &self,
        origin: [i32; 3],
        step: [i32; 3],
        count: [usize; 3],
        seed: i64,
        out: &mut [f32],
    ) {
        let _call = self.owner.enter("fill");
        self.function.fill(origin, step, count, seed, out);
    }
}

fn installed_density_function_types() -> ModDensityFunctionTypes {
    mod_api::__private::density_function_type_declarations()
        .into_iter()
        .map(|(id, mod_id, density_type)| {
            let adapter = TypeAdapter {
                owner: Owner { mod_id, id },
                density_type,
            };
            (
                id.to_owned(),
                Arc::new(adapter) as Arc<dyn ModDensityFunctionType>,
            )
        })
        .collect()
}

/// Hand the density function types the mods registered to the engine, before any world opens.
pub(in crate::game) fn install() {
    if install_mod_density_function_types(installed_density_function_types()).is_err() {
        log::warn!("Mod density function types already installed; keeping the first table");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::api_values::install_parts;
    use mod_api::{Hooks, ModEntry, Registrar, RegistrationContext};

    /// The platform's crash instrumentation, installed before the adapter marks a mod call: a
    /// mark through `mod_api` first would seal it off for every test in this process.
    fn platform() {
        drop(mod_platform::content::crash::enter_native_mod(
            mod_api::content::crash::ModCall {
                id: "test",
                hook: "setup",
                part: None,
            },
        ));
    }

    /// `{"type": "test:tilt", "slope": s}`: s times x plus the seed, only the default `fill`.
    struct Tilt;
    struct Tilted(f32);
    impl DensityFunctionType for Tilt {
        fn decode(&self, json: &str) -> Result<Arc<dyn DensityFunction>, String> {
            let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
            let slope = value["slope"].as_f64().ok_or("no slope")?;
            Ok(Arc::new(Tilted(slope as f32)))
        }
    }
    impl DensityFunction for Tilted {
        fn range(&self) -> [f32; 2] {
            [-1e6, 1e6]
        }
        fn varies_with_y(&self) -> bool {
            false
        }
        fn sample(&self, [x, _, _]: [i32; 3], seed: i64) -> f32 {
            self.0 * x as f32 + seed as f32
        }
    }

    /// The engine reads the mod's own decoder under its namespaced id, and the decoded function
    /// keeps what the mod said; the default `fill` walks y fastest, then x, then z.
    #[test]
    fn a_registered_type_reaches_the_engine_and_decodes_its_fields() {
        platform();
        struct Registers;
        #[mod_api::__subscriptions]
        impl Hooks for Registers {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_density_function_type("tilt", Tilt);
            }
        }
        install_parts(&[ModEntry::new("test", Box::new(Registers))]).unwrap();
        let types = installed_density_function_types();
        let tilt = types["test:tilt"]
            .decode(r#"{"type":"test:tilt","slope":2}"#)
            .unwrap();
        assert_eq!(tilt.range(), [-1e6, 1e6]);
        assert!(!tilt.varies_with_y());
        assert_eq!(tilt.sample([3, 70, -4], 5), 11.0);
        let mut out = [0.0; 4];
        tilt.fill([-1, 0, 0], [4, 8, 1], [2, 2, 1], 0, &mut out);
        assert_eq!(out, [-2.0, -2.0, 6.0, 6.0]);
        let refused = types["test:tilt"].decode(r#"{"type":"test:tilt"}"#);
        assert_eq!(refused.err().as_deref(), Some("no slope"));
    }

    /// `{"type": "breaker:plates"}`: its `sample` and `fill` panic, saying which mod call the
    /// thread was inside.
    struct Breaks;
    struct Broken;
    impl DensityFunctionType for Breaks {
        fn decode(&self, _: &str) -> Result<Arc<dyn DensityFunction>, String> {
            Ok(Arc::new(Broken))
        }
    }
    fn inside() -> String {
        let call = mod_api::content::crash::current_native_call().expect("inside a mod call");
        format!(
            "{} {} {}",
            call.id,
            call.hook,
            call.part.unwrap_or_default()
        )
    }
    impl DensityFunction for Broken {
        fn range(&self) -> [f32; 2] {
            [0.0, 1.0]
        }
        fn sample(&self, _: [i32; 3], _: i64) -> f32 {
            panic!("{}", inside())
        }
        fn fill(&self, _: [i32; 3], _: [i32; 3], _: [usize; 3], _: i64, _: &mut [f32]) {
            panic!("{}", inside())
        }
    }

    /// A panic in the mod's `sample` or `fill` is not caught: it leaves the adapter as a panic, for
    /// the game's crash path, while the thread is marked as inside that mod's call of that type,
    /// which the crash report names. The mark ends with the call.
    #[test]
    fn a_panic_in_a_mod_function_propagates_naming_the_mod_and_type() {
        platform();
        let breaks = TypeAdapter {
            owner: Owner {
                mod_id: "breaker",
                id: "breaker:plates",
            },
            density_type: Arc::new(Breaks),
        };
        let broken = breaks.decode("{}").unwrap();
        let message = |payload: Box<dyn std::any::Any + Send>| {
            payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_default()
        };
        let sampled =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| broken.sample([0, 0, 0], 0)));
        assert_eq!(
            message(sampled.expect_err("sample panics")),
            "breaker sample breaker:plates"
        );
        let filled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            broken.fill([0; 3], [1; 3], [1; 3], 0, &mut [0.0])
        }));
        assert_eq!(
            message(filled.expect_err("fill panics")),
            "breaker fill breaker:plates"
        );
        assert!(mod_api::content::crash::current_native_call().is_none());
    }

    /// Each type a mod may not register is refused at startup, naming the mod.
    #[test]
    fn invalid_density_function_types_refuse_registration() {
        platform();
        struct Invalid;
        #[mod_api::__subscriptions]
        impl Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_density_function_type("tilt", Tilt);
                r.register_density_function_type("tilt", Tilt);
                r.register_density_function_type("other:tilt", Tilt);
                r.register_density_function_type("Tilt", Tilt);
            }
        }
        let errors = install_parts(&[ModEntry::new("test", Box::new(Invalid))]).unwrap_err();
        let expected = [
            "test registered density function type test:tilt twice",
            "test declares \"other:tilt\" in unowned namespace \"other\"",
            "test declares invalid identifier \"test:Tilt\"",
        ];
        assert_eq!(errors.len(), expected.len(), "{errors:?}");
        for (error, expected) in errors.iter().zip(expected) {
            assert!(error.contains(expected), "{error}");
        }
    }

    /// A wasm mod's registrar has only the operations that cross to wasm; its registration of a
    /// density function type stops loading, naming the type.
    #[test]
    #[should_panic(
        expected = "density function type \"plates\": a wasm mod cannot register a density function type"
    )]
    fn a_wasm_mods_density_function_type_is_refused_by_name() {
        struct Guest;
        impl mod_api::DimensionsRegistrar for Guest {
            fn add_dimension(&mut self, _: &str, _: mod_api::DimensionSettings) {}
        }
        mod_api::DimensionsRegistrar::add_density_function_type(
            &mut Guest,
            "plates",
            Arc::new(Tilt),
        );
    }
}
