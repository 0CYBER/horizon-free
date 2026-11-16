# Trust native mods

Native mods run unsandboxed with full access to your computer. Read and trust their source
before installing or building them. Bundle validation checks packaging, not whether code is safe.
Native mixins compile function-replacement code with the same access as other native code.

## Give consent

The launcher shows a trust notice for native mods. In the command line, `--trust-native-code`
records your consent in launcher settings. Commands accept that saved consent without repeating
the flag; without consent they refuse to proceed. The window shows a red warning before each
build, including the mod IDs and versions.

The Mods page identifies each mod's author and marks third-party mods as not made or endorsed
by the Horizon project.

## Package checks

Registry entries distinguish native code with the `native-mod` kind and identify the source
repository with `repo`. Registry review is separate from the launcher's local consent and
validation checks; it does not sandbox the code you run.

- A `.hmod` contains a flat Rust project and can include `assets/`, `data/` and `icon.png`.
- The launcher validates the bundle without running its code, then builds it on your computer.
- Registry downloads are pinned by SHA-256. `horizon-mod.toml` supplies the mod identity and exact
  game build pins. Revocation entries in `revoked.toml` prevent installing revoked releases.
- A registry project's kind cannot change.

## Instance data

The launcher loads native mods for the instance where you install them, using that instance's
worlds, options and logs. Other instances select their own versions. This separation is not a
security boundary: native code can access files outside its instance.

The checks in `horizon-mixins` catch mistakes, not malicious code. See its
[security notes](https://github.com/horizon-rust/horizon-mixins/blob/main/docs/security.md).
