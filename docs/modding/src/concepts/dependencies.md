# Dependencies between mods

A mod can publish functions and statics from its root crate for another mod to call.
Declare dependencies in `horizon-mod.toml`, not `Cargo.toml`:

```toml
[dependencies]
base_mod = "^1"
extra_mod = { version = "^2", optional = true }
```

Requirements use Cargo semver syntax and are checked against each installed mod's `horizon-mod.toml`
`version`, not its Cargo package version. Content-only mods use these declarations to validate their
requirements and, by default, load their layers after dependencies. Code dependencies additionally require
API packages for the dependent and every installed dependency. Missing required dependencies,
incompatible versions and cycles refuse resolution with the mods named. An absent optional dependency
is permitted; an installed optional dependency must match its requirement.

Detailed rows also accept `side`, `order`, `reason` and `url`; see the
[complete manifest schema](../reference/manifest.md). Side filters the declaration by installation
before checking it: an integrated server is client, a dedicated server is server. `order = "before"`
or `"none"` changes listener/content dispatch, not compiler staging or actual imports.

The instance's `horizon-mods/dependency-overrides.toml` can drop a logical constraint/dispatch edge
or make it a warning. These overrides also cover ordinary dependency rows. This policy is a Horizon
addition: it cannot fabricate a code library or bypass native Cargo or wasm ABI/hash/toolchain checks.
A code dependency still links dependency-first or is refused by name. Selected content and loaded
native/wasm code share one rule resolver; warnings and every applied override appear on both affected
installed Mods rows, alongside any failed/off status.

The launcher adds a path dependency using the mod ID as the Cargo alias and the target's actual Cargo
package name. Optional dependencies also get a Cargo feature named exactly as their mod ID, enabled
only when that mod is installed. Guard calls with `if_installed!`, which builds the block only with
that feature and runs it only when the mod is installed, so the same source works when built to
`mod.wasm`, where the library is linked at load and an absent one's functions trap:

```rust,ignore
mod_api::if_installed!(host, extra_mod, {
    extra_mod::register_extra(host);
});
```

Do not define these dependencies or features yourself in `Cargo.toml`.
Public statics must use appropriate synchronization for shared and reentrant hooks; private state in
another mod's hook object is not exposed.

Hook listeners run in descending priority, then resolved dependency dispatch order, with dependency-order ties kept in bundle order. Mixin injection order remains
horizon-mixins' priority, mod ID, then injection ID for source splices. Function-slot listeners
instead use descending priority, dependency/registration order, then injection ID within a mod;
call-site listeners retain mod-ID tie ordering.
