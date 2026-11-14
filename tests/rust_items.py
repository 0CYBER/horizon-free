"""Find a Rust function by name, wherever it lives under `crates/` (#4026).

Tests that paste a real function into a Rust fixture ask for it by name, not by file path,
so moving the function to another file breaks no test. These fixtures are a stopgap until
real Rust tests replace them, so the reading is deliberately simple:

- a definition is any line `[pub(...)] [const|async|unsafe|extern "ABI"] fn name`, found in
  the text as written: a `fn name` line inside a comment, a string or a macro body counts too;
- the body runs from the first `{` after the name to its matching `}`, counting every brace
  character, including those in strings, character literals and comments.

Neither limit can pick a function silently: any second definition of the name, test-only or
not, is refused with the places it was found.
"""
from functools import lru_cache
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
_QUALIFIERS = r'(?:(?:const|async|unsafe|extern(?:[ \t]+"[^"]*")?)[ \t]+)*'


@lru_cache(maxsize=None)
def _sources(under):
    return tuple((path, path.read_text(encoding="utf-8"))
                 for path in sorted((ROOT / under).rglob("*.rs")) if "target" not in path.parts)


def _in_impl_of(text, at, owner):
    """Whether position `at` sits in a top-level `impl` block for type `owner`."""
    text, at = "\n" + text, at + 1  # an `impl` on the file's first line
    header = text.rfind("\nimpl", 0, at)
    if header < 0 or text.rfind("\n}", 0, at) > header:
        return False
    line = text[header + 1:text.index("\n", header + 1)]
    return re.match(r"impl\b(?:<[^>]*>)?\s+(?:[\w:<>', ]+\s+for\s+)?" + re.escape(owner) + r"\b", line) is not None


def rust_fn(name, under="crates", owner=None):
    """The one function `name` defined under `under`, from its `fn` to its closing brace.

    `owner` narrows the search: a crate folder name (`horizon-client`) searches only that
    crate, any other name (`Game`) only methods of a top-level `impl` of that type. Only the
    visibility is dropped (`pub(in crate::game) fn x(` comes back as `fn x(`), so a fixture
    can paste it into its own `impl`. Any second definition is refused, as is a missing one.
    """
    if owner and (ROOT / "crates" / owner).is_dir():
        under, owner = Path("crates") / owner, None
    pattern = re.compile(r"^[ \t]*(?:pub(?:\([^)]*\))?[ \t]+)?(" + _QUALIFIERS + r"fn[ \t]+"
                         + re.escape(name) + r"\b)", re.M)
    found = [(path, text, match) for path, text in _sources(under) for match in pattern.finditer(text)
             if owner is None or _in_impl_of(text, match.start(), owner)]
    if len(found) != 1:
        places = [f"{path}:{text.count(chr(10), 0, match.start()) + 1}" for path, text, match in found]
        raise LookupError(f"expected one `fn {name}` under {under}"
                          + (f" in `impl {owner}`" if owner else "") + f", found {len(found)}: {places}")
    _, text, match = found[0]
    end = text.index("{", match.start(1)) + 1
    depth = 1
    while depth:
        depth += (text[end] == "{") - (text[end] == "}")
        end += 1
    return "    " + text[match.start(1):end]
