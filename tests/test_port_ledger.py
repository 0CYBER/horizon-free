import json
import os
import struct
import subprocess
import sys
import tempfile
import unittest
import zipfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from unittest import mock

from test_class_index import Pool
from tools import port_ledger

SCRIPT = Path(port_ledger.__file__).resolve()


def class_bytes(name: str, parent: str, fields: tuple[str, ...] = (), methods: tuple[str, ...] = ()) -> bytes:
    """A small class file with declared int fields and abstract methods."""
    pool = Pool()
    this, super_ = pool.cls(name), pool.cls(parent)
    encoded_fields = [struct.pack(">HHHH", 1, pool.utf8(field), pool.utf8("I"), 0) for field in fields]
    encoded_methods = [struct.pack(">HHHH", 0x0401, pool.utf8(method), pool.utf8("()V"), 0) for method in methods]
    return (b"\xca\xfe\xba\xbe" + struct.pack(">HH", 0, 69) + pool.bytes()
            + struct.pack(">HHHH", 0x0421, this, super_, 0)
            + struct.pack(">H", len(encoded_fields)) + b"".join(encoded_fields)
            + struct.pack(">H", len(encoded_methods)) + b"".join(encoded_methods) + struct.pack(">H", 0))


class PortLedgerJarOnlyTests(unittest.TestCase):
    def test_default_build_inventories_the_lab_imported_jars(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "research-lock.json").write_text(json.dumps({"minecraft": {"version": "fixture"}}),
                                                    encoding="utf-8")
            jars = root / ".local/minecraft/fixture/jars"
            jars.mkdir(parents=True)
            for filename, name, method in (("client.jar", "ClientOnly", "clientTick"),
                                           ("server-game.jar", "ServerOnly", "serverTick")):
                with zipfile.ZipFile(jars / filename, "w") as archive:
                    archive.writestr(f"net/minecraft/test/{name}.class", class_bytes(
                        f"net/minecraft/test/{name}", "java/lang/Object", methods=(method,)))
            out = root / "output"
            env = dict(os.environ, PORT_LEDGER_ROOT=str(root), PORT_LEDGER_OUT=str(out),
                       HOME=str(root / "home"), APPDATA=str(root / "appdata"))
            done = subprocess.run([sys.executable, str(SCRIPT), "build", "--jobs", "1"],
                                  cwd=root, env=env, capture_output=True, text=True, timeout=30)
            self.assertEqual(done.returncode, 0, done.stderr)
            ledger = json.loads((out / "ledger.json").read_text(encoding="utf-8"))
            self.assertEqual(set(ledger["methods"]), {
                "net/minecraft/test/ClientOnly#clientTick",
                "net/minecraft/test/ServerOnly#serverTick",
            })

    def test_jar_only_and_partial_sources_preserve_field_and_stale_citations(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            rust = root / "crates/example/src/lib.rs"
            rust.parent.mkdir(parents=True)
            rust.write_text("/// Child.present\nfn a() {}\n"
                            "/// Child.inherited\nfn b() {}\n"
                            "/// Child.missing\nfn c() {}\n"
                            "/// Child.tick\nfn d() {}\n"
                            "/// Child.sourceOnly\nfn e() {}\n", encoding="utf-8")
            jar = root / "game.jar"
            with zipfile.ZipFile(jar, "w") as archive:
                archive.writestr("net/minecraft/test/Parent.class", class_bytes(
                    "net/minecraft/test/Parent", "java/lang/Object", fields=("inherited",), methods=("tick",)))
                archive.writestr("net/minecraft/test/Child.class", class_bytes(
                    "net/minecraft/test/Child", "net/minecraft/test/Parent", fields=("present",)))
            sources = root / "sources"
            parent = sources / "net/minecraft/test/Parent.java"
            parent.parent.mkdir(parents=True)
            parent.write_text("package net.minecraft.test;\npublic class Parent {\n int inherited;\n void tick() {}\n}\n",
                              encoding="utf-8")
            out = root / "output"
            with mock.patch.multiple(port_ledger, ROOT=root, OUT=out, LEDGER=out / "ledger.json",
                                     MAP_DIR=root / "maps", ORACLE_DIR=root / "oracles",
                                     ProcessPoolExecutor=ThreadPoolExecutor), \
                    mock.patch.object(port_ledger, "rust_files", return_value=["crates/example/src/lib.rs"]):
                for source_roots in ([], [str(sources)]):
                    with self.subTest(source_roots=source_roots):
                        result = port_ledger.build(source_roots, [str(jar)], 1)
                        stale = {g["cite"] for g in result["gaps"] if g["kind"] == "stale"}
                        self.assertIn("Child.missing", stale)
                        self.assertNotIn("Child.present", stale)
                        self.assertNotIn("Child.inherited", stale)
                        self.assertNotIn("Child.tick", stale)
                        self.assertIn("Child.sourceOnly", stale)
                        self.assertEqual(json.loads((out / "ledger.json").read_text())["sources"], source_roots)
                child = sources / "net/minecraft/test/Child.java"
                child.write_text("package net.minecraft.test;\npublic class Child extends Parent {\n"
                                 " int present;\n int sourceOnly;\n void tick() {}\n}\n", encoding="utf-8")
                for jars in ([str(jar)], []):
                    with self.subTest(jars=jars):
                        result = port_ledger.build([str(sources)], jars, 1)
                        stale = {g["cite"] for g in result["gaps"] if g["kind"] == "stale"}
                        self.assertNotIn("Child.present", stale)
                        self.assertNotIn("Child.sourceOnly", stale)
                        self.assertNotIn("Child.tick", stale)
                        self.assertIn("Child.missing", stale)


class PortLedgerStaleScanTests(unittest.TestCase):
    """A citation is stale only when its own vanilla class and all its supertypes lack the method."""

    SOURCES = {
        "net/minecraft/test/SimplexNoise.java":
            "package net.minecraft.test;\n"
            "public class SimplexNoise { public double get(double x, double y) { return 0.0; } }\n",
        "net/minecraft/test/AbstractScrollArea.java":
            "package net.minecraft.test;\n"
            "public abstract class AbstractScrollArea { public void updateScrolling() {} }\n",
        "net/minecraft/test/AbstractSelectionList.java":
            "package net.minecraft.test;\n"
            "public abstract class AbstractSelectionList<E extends AbstractSelectionList.Entry<E>>\n"
            "   extends AbstractScrollArea { public static class Entry<E> {} }\n",
        "net/minecraft/test/Base.java":
            "package net.minecraft.test;\npublic class Base { public void inherited() {} }\n",
        "net/minecraft/test/Middle.java":
            "package net.minecraft.test;\npublic class Middle extends Base { }\n",
        "net/minecraft/test/Leaf.java":
            "package net.minecraft.test;\npublic class Leaf extends Middle { }\n",
        "net/minecraft/test/QualifiedLeaf.java":
            "package net.minecraft.test;\npublic class QualifiedLeaf extends net.minecraft.test.Base { }\n",
        "net/minecraft/test/Level.java":
            "package net.minecraft.test;\npublic abstract class Level implements LevelAccessor, AutoCloseable { }\n",
        "net/minecraft/test/LevelAccessor.java":
            "package net.minecraft.test;\n"
            "public interface LevelAccessor extends CollisionGetter, EntityGetter { }\n",
        "net/minecraft/test/CollisionGetter.java":
            "package net.minecraft.test;\n"
            "public interface CollisionGetter { default void getBlockCollisions() {} }\n",
        "net/minecraft/test/EntityGetter.java":
            "package net.minecraft.test;\n"
            "public interface EntityGetter { default void getEntitiesOfClass() {} }\n",
        "net/minecraft/world/level/gameevent/vibrations/VibrationSystem.java":
            "package net.minecraft.world.level.gameevent.vibrations;\n"
            "public interface VibrationSystem { class Listener { boolean present() { return false; } } }\n",
        **{f"net/minecraft/test/Chain{depth}.java":
            f"package net.minecraft.test;\npublic class Chain{depth}"
            + (f" extends Chain{depth + 1}" if depth < 14 else "")
            + (" { public void deep() {} }\n" if depth == 14 else " { }\n")
            for depth in range(15)},
    }
    RUST = (
        "/// Math.max for the bounds.\nfn a() {}\n"
        "/// Ranges.margins for the viewport.\nfn b() {}\n"
        "/// SimplexNoise.getValue, which 1.21 had; 26.3 spells it get.\nfn c() {}\n"
        "/// VibrationSystem.Listener.isOccluded for the allay.\nfn d() {}\n"
        "/// VibrationSystem.Listener.present does exist.\nfn e() {}\n"
        "/// AbstractSelectionList.updateScrolling, through the generic superclass.\nfn f() {}\n"
        "/// Leaf.inherited, from Base through Middle.\nfn g() {}\n"
        "/// Level.getBlockCollisions, through LevelAccessor to CollisionGetter.\nfn h() {}\n"
        "/// Level.getEntitiesOfClass, through the second superinterface.\nfn i() {}\n"
        "/// QualifiedLeaf.inherited, through a fully qualified superclass.\nfn j() {}\n"
        "/// Chain0.deep, from the deepest supertype in a long chain.\nfn k() {}\n"
    )

    def test_only_a_known_vanilla_class_makes_a_citation_stale(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            sources = root / "sources"
            for rel, text in self.SOURCES.items():
                source = sources / rel
                source.parent.mkdir(parents=True, exist_ok=True)
                source.write_text(text, encoding="utf-8")
            rust = root / "crates/example/src/lib.rs"
            rust.parent.mkdir(parents=True)
            rust.write_text(self.RUST, encoding="utf-8")
            out = root / "output"
            with mock.patch.multiple(port_ledger, ROOT=root, OUT=out, LEDGER=out / "ledger.json",
                                     MAP_DIR=root / "maps", ORACLE_DIR=root / "oracles",
                                     ProcessPoolExecutor=ThreadPoolExecutor), \
                    mock.patch.object(port_ledger, "rust_files", return_value=["crates/example/src/lib.rs"]):
                result = port_ledger.build([str(sources)], [], 1)
                stale = {g["cite"] for g in result["gaps"] if g["kind"] == "stale"}
                # Math (JDK) and Ranges (Horizon) are not vanilla classes, and every inherited method
                # is found through its supertypes; only the rename and the nested-class miss are stale.
                self.assertEqual(stale, {"SimplexNoise.getValue", "VibrationSystem.Listener.isOccluded"})


class PortLedgerCliTests(unittest.TestCase):
    """--json is read the same before or after the query's name (port_ledger.py's docstring)."""

    # A ledger with one class, enough for `where` to answer without a built ledger.
    LEDGER = {
        "classes": {"net/minecraft/world/Foo": {"file": "net/minecraft/world/Foo.java", "name": "Foo"}},
        "methods": {"net/minecraft/world/Foo#tick": {
            "class": "net/minecraft/world/Foo", "name": "tick", "file": "net/minecraft/world/Foo.java",
            "lines": [[10, 20]], "callees": [], "client": False}},
        "class_cites": {}, "ported": {}, "mentioned": {}, "inferred": {},
    }

    def run_cli(self, out_dir, *args):
        env = dict(os.environ, PORT_LEDGER_OUT=str(out_dir))
        return subprocess.run([sys.executable, str(SCRIPT), *args], capture_output=True, text=True, env=env)

    def test_json_flag_read_before_or_after_the_query(self):
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "ledger.json").write_text(json.dumps(self.LEDGER), encoding="utf-8")
            for args, want_json in ((["where", "Foo", "--json"], True),
                                    (["--json", "where", "Foo"], True),
                                    (["where", "Foo"], False)):
                with self.subTest(args=args):
                    done = self.run_cli(tmp, *args)
                    self.assertNotIn("unrecognized arguments", done.stderr)
                    self.assertEqual(done.returncode, 0, done.stderr)
                    if want_json:
                        self.assertEqual(json.loads(done.stdout)[0]["path"], "net/minecraft/world/Foo")
                    else:
                        with self.assertRaises(json.JSONDecodeError):
                            json.loads(done.stdout)
                        self.assertIn("class: Foo", done.stdout)
            # The query's own help offers the flag, so it is where a reader looks for it.
            helped = self.run_cli(tmp, "where", "--help")
            self.assertEqual(helped.returncode, 0, helped.stderr)
            self.assertIn("--json", helped.stdout)


if __name__ == "__main__":
    unittest.main()
