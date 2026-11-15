"""The mod API's boundary: mods see only the API, and the game reaches the rest through
`mod_api::__private`."""
from pathlib import Path
import re
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
API = ROOT / "crates/horizon-mod-vocab/src"
def api_files():
    return [p for p in sorted(API.rglob("*.rs")) if p != API / "__private.rs" and not p.name.endswith("_tests.rs")]


def fragment_scope(path, scope):
    """Read the generator's restricted use/mod fragments at one lexical scope."""
    stack, lines = [], []
    for line in path.read_text().splitlines():
        module = re.fullmatch(r"\s*(?:pub(?:\([^)]*\))? )?mod (\w+) \{", line)
        if module:
            stack.append(module[1])
        elif line.strip() == "}":
            stack.pop()
        elif tuple(stack) == scope:
            lines.append(line)
    return "\n".join(lines)


def api_source(path):
    return fragment_scope(path, ()) if path.parent.name == "exports" else path.read_text()


def api_text():
    return "\n".join(api_source(p) for p in api_files())


def pub_items(path):
    """Names of the top-level `pub` items of a rustfmt-formatted Rust file, `pub use` names included."""
    text = path.read_text()
    declared = re.findall(r"^pub (?:unsafe )?(?:struct|enum|union|trait|type|fn|const|static|mod|macro) (\w+)", text, re.M)
    return set(declared) | use_names(text, path.parent.parent)


def use_names(text, src):
    """The names the `pub use` lines of `text` bind; a glob of `<domain>::types` (relative to `src`) is expanded to
    that file's `pub` items, any other glob stays `*`."""
    found = set()
    for path in re.findall(r"^\s*pub use ([^;]*);", text, re.M):
        if "{" in path:
            prefix, rest = path.split("{", 1)
            leaves = [prefix + n.strip() for n in re.sub(r"[{}\s]", " ", rest).split(",") if n.strip()]
        else:
            leaves = [path.strip()]
        for leaf in leaves:
            glob = re.fullmatch(r"(\w+)::types::\*", leaf.replace(" ", ""))
            if glob and (src / glob[1] / "types.rs").is_file():
                found |= pub_items(src / glob[1] / "types.rs")
            else:
                found.add(leaf.split(" as ")[-1].split("::")[-1].strip())
    return found


def domain_export_collisions(src):
    """Guard rustfmt-formatted root exports, including catalog-generated hook Event names.

    An identical direct re-export of a domain item is harmless and excluded.
    Other generated names cannot be enumerated without a build; their explicit
    re-exports are covered, but non-hook generated glob members are not.
    """
    explicit, domains = {}, {}
    glob_domains = set()
    for path in sorted((src / "exports").glob("*.rs")):
        glob_domains.update(re.findall(r"pub use (\w+)::types::\*;", fragment_scope(path, ())))
    for path in [src / "lib.rs", *sorted((src / "exports").glob("*.rs"))]:
        text = fragment_scope(path, ()) if path.parent.name == "exports" else path.read_text()
        for domain in re.findall(r"pub use (\w+)::types::\*;", text):
            types = src / domain / "types.rs"
            for name in pub_items(types):
                domains.setdefault(name, []).append(str(types))
        text = re.sub(r"pub use \w+::types::\*;", "", text)
        text = re.sub(r"pub use (\w+)::types::(\w+);",
                      lambda m: "" if m[1] in glob_domains and m[2] in pub_items(src / m[1] / "types.rs") else m[0], text)
        names = use_names(text, src) - {"*"}
        names |= set(re.findall(r"^pub (?:unsafe )?(?:struct|enum|union|trait|type|fn|const|static|mod) (\w+)", text, re.M))
        for name in names:
            explicit.setdefault(name, []).append(str(path))
    for path in sorted((src / "hooks").glob("*.rs")):
        for name in re.findall(r"^    fn (\w+)\(", path.read_text(), re.M):
            event = "".join(word[:1].upper() + word[1:] for word in name.split("_")) + "Event"
            explicit.setdefault(event, []).append(str(path) + " (generated event)")
    problems = []
    for name, locations in sorted(domains.items()):
        locations = locations + explicit.get(name, [])
        if len(locations) > 1:
            problems.append(f"{name}: {' and '.join(locations)}: rename the type or remove the explicit export")
    return problems


def game_side_api_names(api, private, src):
    """Names re-exported both as API (`api` text) and on the game's side (`private` text)."""
    return use_names(api, src) & (use_names(private, src) - {"*"})


def preceding(lines, i):
    """The attribute, doc and comment lines directly above line `i`, nearest first."""
    found = []
    while i > 0 and lines[i - 1].strip().startswith(("#[", "//")):
        i -= 1
        found.append(lines[i].strip())
    return found


class ModApiBoundaryTests(unittest.TestCase):
    def test_domain_globs_do_not_hide_names(self):
        self.assertEqual(domain_export_collisions(API), [])

    def test_domain_collision_guard_on_synthetic_exports(self):
        with tempfile.TemporaryDirectory() as folder:
            src = Path(folder)
            (src / "exports").mkdir()
            (src / "lib.rs").write_text("pub(crate) struct Name;\n")
            for domain in ("block", "item"):
                (src / domain).mkdir()
                (src / domain / "types.rs").write_text("pub struct Name;\npub(crate) struct Hidden;\n")
            exports = src / "exports/domains.rs"
            for extra in ("pub use other::Name;", "pub use other::Other as Name;", "pub use block::types::Name as Name;",
                          "pub struct Name;", "pub use item::types::Name;",
                          "pub use item::types::*;"):
                exports.write_text("pub use block::types::*;\n" + extra + "\n")
                problems = domain_export_collisions(src)
                self.assertEqual(len(problems), 1)
                self.assertIn("Name:", problems[0])
                self.assertIn("block/types.rs", problems[0])
            exports.write_text("pub use block::types::*;\npub use block::types::Name;\n")
            self.assertEqual(domain_export_collisions(src), [])
            exports.write_text("pub use block::types::*;\nmod __private {\n    pub use other::Name;\n}\n")
            self.assertEqual(domain_export_collisions(src), [])

    def test_every_public_type_has_an_explicit_wire_policy(self):
        from tools.check_mod_api_wire import repository_violations
        self.assertEqual(repository_violations(), [],
                         "derive Wire or attach // no wire: <reason> explaining a manual codec or refusal")

    def test_every_public_type_is_non_exhaustive_or_says_why(self):
        missing = []
        for path in api_files():
            lines = api_source(path).splitlines()
            for i, line in enumerate(lines):
                if re.match(r"\s*pub (struct|enum) \w+", line):
                    above = preceding(lines, i)
                    if not any(a == "#[non_exhaustive]" or a.startswith("// exhaustive: ") for a in above):
                        missing.append(f"{path.relative_to(ROOT)}:{i + 1}: {line.strip()}")
        self.assertEqual(missing, [], "add #[non_exhaustive], or a `// exhaustive: <why>` line above the type")

    def test_the_game_side_is_reached_only_through_private(self):
        # `built` holds copies under the API's own names, so it is left out.
        private = re.sub(r"\npub mod built \{.*?\n\}\n", "\n", (API / "__private.rs").read_text(), flags=re.S)
        fragments = sorted((API / "exports").glob("*.rs"))
        private += "\n" + "\n".join(fragment_scope(path, scope) for path in fragments
                                    for scope in [("__private",), ("__private", "game"),
                                                  ("__private", "reload"), ("__private", "wasm_host")])
        api = (API / "lib.rs").read_text() + "\n" + "\n".join(api_source(p) for p in fragments)
        self.assertEqual(game_side_api_names(api, private, API), set(), "a game-side item is re-exported as API")

    def test_a_game_side_name_behind_a_types_glob_is_caught(self):
        with tempfile.TemporaryDirectory() as src:
            (Path(src) / "block").mkdir()
            (Path(src) / "block/types.rs").write_text(
                "use crate::*;\n/// Docs.\n#[non_exhaustive]\npub struct BlockKey;\npub(crate) struct Hidden;\n"
                "pub fn game_tick() {}\nimpl BlockKey {\n    pub fn inner() {}\n}\n")
            private = "pub use crate::game::{game_tick, inner};\npub use crate::generated::calls::*;"
            self.assertEqual(game_side_api_names("pub use block::types::*;", private, Path(src)), {"game_tick"})

    def test_doc_hidden_outside_private_says_why(self):
        # `__private` is the game's side. Elsewhere an item is hidden only with a
        # `// hidden: <why>` line above, except a macro re-export or a crate-private module.
        unexplained = []
        for path in api_files():
            lines = api_source(path).splitlines()
            for i, line in enumerate(lines):
                if line.strip() != "#[doc(hidden)]" or i + 1 >= len(lines):
                    continue
                item = lines[i + 1].strip()
                if item.startswith(("pub(crate)", "pub use horizon_mod_api_macros::")):
                    continue
                if not any(a.startswith("// hidden: ") for a in preceding(lines, i)):
                    unexplained.append(f"{path.relative_to(ROOT)}:{i + 1}: {item}")
        self.assertEqual(unexplained, [], "move the item into `__private`, or add `// hidden: <why>`")

    def test_world_fluid_registration_uses_api_owned_values(self):
        source = (API / "parts/fluids.rs").read_text()
        registrar = source.split("pub trait FluidsRegistrar {", 1)[1].split("\n}", 1)[0]
        # The generator consumes the trait signature as the mod-facing contract.
        self.assertNotIn("horizon_mod_package::", registrar)
        self.assertRegex(api_text(), r"pub struct WorldFluid\b")

    def test_fluid_model_collection_uses_api_owned_values(self):
        source = (ROOT / "crates/horizon-client/src/game/native_hooks/register_fluid_models.rs").read_text()
        signature = source.split("fn native_register_fluid_models(", 1)[1].split(") {", 1)[0]
        # Collected hook fields are generated directly from this signature.
        self.assertNotIn("horizon_mod_package::", signature)
        self.assertRegex(api_text(), r"pub struct FluidModel\b")

    def test_surfaces_stay_in_area_fragments(self):
        for name in ["lib.rs", "__private.rs"]:
            source = (API / name).read_text()
            self.assertNotRegex(source, r"(?m)^\s*pub(?:\(crate\))? use ")
            self.assertIn('include!(concat!(env!("OUT_DIR"), "/mod_api_exports_', source)
        self.assertNotRegex((API / "lib.rs").read_text(), r"(?m)^(?:pub(?:\(crate\))? )?mod \w+;")

    def test_unhandled_arms_name_a_non_exhaustive_enum(self):
        text = api_text()
        for path in ROOT.glob("crates/horizon-client*/src/**/*.rs"):
            for name in re.findall(r'unreachable!\("(\w+): unhandled variant"\)', path.read_text()):
                self.assertRegex(
                    text,
                    rf"#\[non_exhaustive\]\s*(?:#\[[^\n]*\]\s*)*pub enum {name}\b",
                    f"{path.relative_to(ROOT)}: `{name}` is not a #[non_exhaustive] pub enum of the API",
                )


if __name__ == "__main__":
    unittest.main()
