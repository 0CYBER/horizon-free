#!/usr/bin/env python3
"""Before the first cargo call of a self-hosted game-build.yml job: refuse cargo configuration from outside the
checkout, then give the job an empty cargo home of its own.

Fails, saying what it found, when an ancestor of the checkout (its parent up to /) holds .cargo/config or
.cargo/config.toml, or when a variable that changes what cargo or rustc run is set. It prints paths, cargo's own
table names and variable names only, never a value: these logs are public and a cargo config can hold registry
tokens. The checkout's own .cargo/config.toml is the repository's. On success it empties $RUNNER_TEMP/cargo-home
and writes CARGO_HOME to $GITHUB_ENV.

game-build.yml runs this file from its own commit, never from the checkout it guards, with macOS's /usr/bin/python3
(3.9 with the Command Line Tools) as `python3 -I -S`: so standard library only, no site, nothing from the checkout,
and nothing newer than Python 3.9 except tomllib, which is optional (without it a config is "could not be parsed").

Usage: cargo_isolation.py <checkout> [--top <dir>]  (--top stops the ancestor walk; for tests)
"""
import os
import shutil
import sys

CLEAN = "cargo isolation: clean"
CONFIG_NAMES = ("config", "config.toml")
FORBIDDEN = {
    "RUSTC", "RUSTDOC", "RUSTFLAGS", "RUSTDOCFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_ENCODED_RUSTFLAGS", "CARGO_ENCODED_RUSTDOCFLAGS", "CARGO_INCREMENTAL", "CARGO_TARGET_DIR",
}
FORBIDDEN_PREFIXES = (
    "CARGO_BUILD_", "CARGO_TARGET_", "CARGO_PROFILE_", "CARGO_REGISTRIES_", "CARGO_REGISTRY_", "CARGO_SOURCE_",
    "CARGO_NET_", "CARGO_HTTP_", "CARGO_ALIAS_", "CARGO_UNSTABLE_",
)
# What these jobs set themselves, with the one value they set (dtolnay/rust-toolchain exports it).
ALLOWED = {"CARGO_INCREMENTAL": "0"}


def forbidden(name):
    return name in FORBIDDEN or name.startswith(FORBIDDEN_PREFIXES)


def ancestors(checkout, top=None):
    seen, found = set(), []
    for start in (os.path.abspath(checkout), os.path.realpath(checkout)):
        here = start
        while True:
            parent = os.path.dirname(here)
            if parent == here or (top is not None and os.path.normcase(here) == os.path.normcase(top)):
                break
            here = parent
            if here not in seen:
                seen.add(here)
                found.append(here)
    return found


# Cargo's own top-level tables: only these names are printed. Any other top-level name, and every nested key, can
# carry data (a URL with credentials as a key under [patch] or [source], a registry's name), so they are not.
KNOWN_TABLES = {
    "alias", "build", "cargo-new", "credential-alias", "doc", "env", "future-incompat-report", "cache", "http",
    "install", "net", "patch", "paths", "profile", "registries", "registry", "source", "target", "term", "unstable",
    "resolver",
}


def table_names(path):
    """The known top-level table names in a TOML file and a count of the others, or None when it can't be read or
    parsed."""
    try:
        import tomllib
        with open(path, "rb") as file:
            data = tomllib.load(file)
    except Exception:
        return None
    known = sorted(name for name in data if name in KNOWN_TABLES)
    others = sum(1 for name in data if name not in KNOWN_TABLES)
    return known + ([f"other ({others})"] if others else [])


def problems(checkout, environ, top=None):
    found = []
    for directory in ancestors(checkout, top):
        for config in CONFIG_NAMES:
            path = os.path.join(directory, ".cargo", config)
            if not os.path.lexists(path):
                continue
            names = table_names(path)
            if names is None:
                found.append(f"cargo configuration outside the checkout: {path} (could not be parsed)")
            else:
                found.append(f"cargo configuration outside the checkout: {path} (tables: {', '.join(names) or 'none'})")
    # A single name is printed as it is; a prefix family only by its prefix and a count: the rest of such a name is
    # free text (a registry's name, an alias, a target triple).
    families = {}
    for name in sorted(environ):
        if not forbidden(name) or environ[name] == "" or ALLOWED.get(name) == environ[name]:
            continue
        if name in FORBIDDEN:
            found.append(f"variable set: {name}")
        else:
            prefix = next(prefix for prefix in FORBIDDEN_PREFIXES if name.startswith(prefix))
            families[prefix] = families.get(prefix, 0) + 1
    for prefix, count in sorted(families.items()):
        found.append(f"variables set: {prefix}* ({count})")
    return found


def main(argv):
    args = list(argv)
    top = None
    if "--top" in args:
        index = args.index("--top")
        top = os.path.abspath(args[index + 1])
        del args[index:index + 2]
    [checkout] = args
    found = problems(checkout, dict(os.environ), top)
    if found:
        for line in found:
            print(f"::error::{line}")
        print("::error::cargo would use configuration from outside the checkout; remove it from the runner")
        return 1
    home = os.path.join(os.environ["RUNNER_TEMP"], "cargo-home")
    shutil.rmtree(home, ignore_errors=True)
    os.mkdir(home)
    with open(os.environ["GITHUB_ENV"], "a", encoding="utf-8") as env:
        env.write(f"CARGO_HOME={home}\n")
    print(f"CARGO_HOME={home}")
    # game-build.yml accepts the run only when this is its last line: an empty or cut-off download prints nothing.
    print(CLEAN)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
