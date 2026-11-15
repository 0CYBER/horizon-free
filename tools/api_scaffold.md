# API scaffolding

`python3 tools/api_scaffold.py --dry-run` previews the allow-list in
`tools/api_scaffold.toml`. Use `--config path` for another allow-list and `--root path`
for an isolated checkout. `--apply` writes only after every entry has been planned
successfully. Existing type or backend declarations refuse the entire run; existing
files are appended to, never replaced wholesale. A second apply refuses.

Each entry requires a domain (`world`, `block`, `item`, `entity`, `client`,
`command`, `text`, `content` or `wire`) and a lower-case topic. Values go to
`crates/horizon-mod-vocab/src/<domain>/<topic>.rs`. New topic files are registered in
that domain's `mod.rs`; the domain itself must already be registered by the API.
`lib` and `mod` are not valid topics. Exports, backend areas and game conversion
modules retain their topic-based filenames. Choose distinct topics for unrelated
areas, even when they belong to different domains.

Every emitted value, including a handle's hidden fields projection, explicitly
derives `horizon_mod_api_macros::Wire`. There is no hand-written codec or generator
fallback. Handles use `WorldEpoch`. Default API names remove an engine leaf's
`Native` prefix; explicit API names with that prefix are refused. Engine paths and
engine construction expressions are preserved verbatim.

This is a first pass, not an automatic API implementation. Review the Rust output,
field types, docs and conversions, implement handle reads (initially `None`), and
run the API generator and Rust checks after integration. Configured field and return
types must already use the intended API names. The tool does not rewrite engine
expressions, infer nested conversions, or check existing conversion implementations.

Run the focused tool tests with:

```sh
python3 -m unittest discover -s tests -p test_api_scaffold.py
```
