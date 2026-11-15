//! Load-phase data-file condition registration.
pub trait Hooks {
    /// Register named conditions once at mod load, before server data admission.
    /// Each handler is a raw-JSON handler: it reads the condition's JSON value.
    /// Names must belong to the registering mod's namespace. Native and wasm
    /// handlers use the same registration and evaluation route. Registering one
    /// name owns that namespace; an unknown condition in it refuses data reload.
    /// Foreign unknown ids omit their file with one ERROR-level log entry,
    /// even inside `or(true, unknown)`; every combinator branch is visited.
    #[hook(
        side = "server",
        contract = "collect",
        out = "conditions",
        context = "registration",
        neoforge = "RegisterEvent"
    )]
    fn register_load_conditions();
}
