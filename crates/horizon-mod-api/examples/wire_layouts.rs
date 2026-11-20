//! Prints what the game compares with a wasm mod's own when it loads, as this build compiled it:
//! the core layout (`mod_api::wire::LAYOUT`) and every call's key, hash and whether it is an
//! export (`mod_api::call_layouts`), with how many calls it wrote, so a truncated copy is told
//! apart. `tools/ci/surface_report.py` compares a pull request's with
//! its base's (`mod_surface::boundary::Compiled`). It must build on older trees too: CI builds a
//! base with the head's copy of this file.
fn main() {
    let mut calls = mod_api::call_layouts();
    calls.sort();
    let mut out = format!(
        "{{\n  \"schema\": 1,\n  \"core\": \"{:016x}\",\n  \"count\": {},\n  \"calls\": [",
        *mod_api::wire::LAYOUT,
        calls.len()
    );
    for (index, (key, hash, export)) in calls.iter().enumerate() {
        let comma = if index == 0 { "" } else { "," };
        out.push_str(&format!(
            "{comma}\n    {{ \"key\": {key:?}, \"layout\": \"{hash:016x}\", \"export\": {export} }}"
        ));
    }
    out.push_str("\n  ]\n}\n");
    print!("{out}");
}
