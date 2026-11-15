"""The images Railway builds for the telemetry server and the mod registry copy only the crates they name.

Each service is its own workspace with its own Cargo.lock. When one of its crates gains a path dependency on
another crate of the repository, the image must copy that crate too, and Railway must rebuild when it changes:
otherwise the next deploy fails (`no matching package named ...`) while the old deployment keeps running, and nobody
sees it (2026-11-09: both services failed after horizon-addresses and horizon-release-sign joined them).

A build script can read more than its own crate. horizon-mod-vocab's runs the mod API generator
(horizon-mod-api-gen), which reads the game's hook sources and the crate each hook target names, so an image whose
lockfile has horizon-mod-vocab copies every crate (`COPY crates crates`) and rebuilds when any of them changes (#3474).
"""

import json
import re
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SERVICES = ["horizon-telemetry-server", "horizon-mod-registry"]
# The crate whose build script runs the mod API generator, and the generator's sources.
VOCAB = "horizon-mod-vocab"
GENERATOR = ROOT / "crates" / "horizon-mod-api-gen" / "src"


def all_crates() -> set[str]:
    return {path.parent.name for path in (ROOT / "crates").glob("*/Cargo.toml")}


def path_packages(lock: Path) -> set[str]:
    """The packages a lockfile builds from a path: the ones without a `source`."""
    return {package["name"] for package in tomllib.loads(lock.read_text())["package"] if "source" not in package}


def generator_reads() -> tuple[set[str], bool]:
    """The crates the mod API generator reads by a path it names (`"crates/<name>/..."`), and whether it also
    reads crates it names only at run time (`root.join("crates").join(krate)`: a hook target's crate)."""
    named, dynamic = set(), False
    for source in GENERATOR.rglob("*.rs"):
        text = source.read_text(encoding="utf-8")
        named |= set(re.findall(r'"crates/([a-z0-9-]+)/', text))
        dynamic = dynamic or re.search(r'\.join\("crates"\)\s*\.join\(', text) is not None
    return named & all_crates(), dynamic


def generator_files() -> set[str]:
    """The repository files outside crates/ that the generator names by a literal path (`"tools/<...>"`) and that
    exist: retired names the generator only checks are gone are not files."""
    named = set()
    for source in GENERATOR.rglob("*.rs"):
        named |= set(re.findall(r'"(tools/[A-Za-z0-9_./-]+)"', source.read_text(encoding="utf-8")))
    return {name for name in named if (ROOT / name).is_file()}


def included_files(crates: set[str]) -> set[str]:
    """The repository files outside crates/ that the crates' own code includes at compile time
    (`include_str!`/`include_bytes!` before the first `#[cfg(test)]`, and not from a test file) and that exist."""
    found = set()
    for name in crates:
        for source in (ROOT / "crates" / name / "src").rglob("*.rs"):
            if "test" in source.name:
                continue
            text = source.read_text(encoding="utf-8").split("#[cfg(test)]")[0]
            for relative in re.findall(r'include_(?:str|bytes)![(]"([^"]+)"[)]', text):
                target = (source.parent / relative).resolve()
                if target.is_file() and target.is_relative_to(ROOT) and not target.is_relative_to(ROOT / "crates"):
                    found.add(target.relative_to(ROOT).as_posix())
    return found


def copies_every_crate(dockerfile: Path) -> bool:
    return re.search(r"^COPY crates crates\s*$", dockerfile.read_text(), re.MULTILINE) is not None


def copied_crates(dockerfile: Path) -> set[str]:
    if copies_every_crate(dockerfile):
        return all_crates()
    return set(re.findall(r"^COPY crates/([a-z0-9-]+) crates/\1\s*$", dockerfile.read_text(), re.MULTILINE))


def watched_crates(railway: Path) -> set[str]:
    patterns = json.loads(railway.read_text())["build"]["watchPatterns"]
    if "/crates/**" in patterns:
        return all_crates()
    return {match.group(1) for pattern in patterns if (match := re.fullmatch(r"/crates/([a-z0-9-]+)/\*\*", pattern))}


def needed_crates(service: str) -> set[str]:
    """What the image's build reads: its lockfile's path crates, and the generator's inputs when it runs."""
    needed = path_packages(ROOT / "crates" / service / "Cargo.lock")
    if VOCAB in needed:
        needed |= generator_reads()[0]
    return needed


class ServiceImageTests(unittest.TestCase):
    def test_each_image_copies_every_path_crate_of_its_lockfile(self):
        for service in SERVICES:
            crate = ROOT / "crates" / service
            needed = path_packages(crate / "Cargo.lock")
            with self.subTest(service=service):
                self.assertIn(service, needed)
                for name in needed:
                    self.assertTrue((ROOT / "crates" / name / "Cargo.toml").is_file(), f"{name} is not crates/{name}")
                missing = sorted(needed_crates(service) - copied_crates(crate / "Dockerfile"))
                self.assertEqual(missing, [], f"{service}/Dockerfile must COPY these crates")

    def test_an_image_that_runs_the_generator_copies_every_crate(self):
        # The generator also reads the crate each hook target names, known only when it runs: no list of crates is
        # complete, so the image copies them all and Railway watches them all.
        named, dynamic = generator_reads()
        self.assertIn("horizon-client", named)
        self.assertTrue(dynamic, "the generator no longer reads crates named at run time: a list of crates may do")
        for service in SERVICES:
            crate = ROOT / "crates" / service
            if VOCAB not in path_packages(crate / "Cargo.lock"):
                continue
            with self.subTest(service=service):
                self.assertTrue(copies_every_crate(crate / "Dockerfile"), f"{service}/Dockerfile must COPY crates crates")
                patterns = json.loads((crate / "railway.json").read_text())["build"]["watchPatterns"]
                self.assertIn("/crates/**", patterns, f"{service}/railway.json must watch /crates/**")

    def test_an_image_that_runs_the_generator_copies_and_watches_its_data_files(self):
        # The build script fails when a file the generator reads outside crates/ is missing from the image.
        self.assertIn("tools/layering/mod_vocab_state.json", generator_files())
        self.assertIn("tools/mixins/narrow-reference-rule.patch", included_files({"horizon-mod-package"}))
        for service in SERVICES:
            crate = ROOT / "crates" / service
            lock = path_packages(crate / "Cargo.lock")
            if VOCAB not in lock:
                continue
            files = generator_files() | included_files(lock)
            with self.subTest(service=service):
                copied = set(re.findall(r"^COPY (tools/\S+) \1\s*$", (crate / "Dockerfile").read_text(), re.MULTILINE))
                self.assertEqual(sorted(files - copied), [], f"{service}/Dockerfile must COPY these files")
                patterns = json.loads((crate / "railway.json").read_text())["build"]["watchPatterns"]
                self.assertEqual(sorted(f for f in files if f"/{f}" not in patterns), [], f"{service}/railway.json")

    def test_railway_rebuilds_when_any_of_those_crates_changes(self):
        for service in SERVICES:
            crate = ROOT / "crates" / service
            with self.subTest(service=service):
                missing = sorted(needed_crates(service) - watched_crates(crate / "railway.json"))
                self.assertEqual(missing, [], f"{service}/railway.json watchPatterns must list these crates")

    def test_the_checks_read_real_dockerfile_lines(self):
        # A COPY of a crate is recognised, a COPY of anything else is not, and a lockfile's registry packages
        # are not path packages.
        sample = ROOT / "crates" / "horizon-mod-registry" / "Dockerfile"
        self.assertIn("horizon-mod-registry", copied_crates(sample))
        self.assertNotIn("Cargo.toml", copied_crates(sample))
        self.assertNotIn("serde", path_packages(ROOT / "crates" / "horizon-mod-registry" / "Cargo.lock"))
        # A Dockerfile that names its crates one by one copies only those.
        telemetry = ROOT / "crates" / "horizon-telemetry-server" / "Dockerfile"
        self.assertFalse(copies_every_crate(telemetry))
        self.assertNotIn("horizon-client", copied_crates(telemetry))


if __name__ == "__main__":
    unittest.main()
