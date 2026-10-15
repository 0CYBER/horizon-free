import struct
import unittest

from tools import class_index, vanilla_map


class Pool:
    """A constant pool built in the order entries are asked for; `pad` shifts every index."""

    def __init__(self, pad=0):
        self.entries, self.index = [], {}
        for i in range(pad):
            self.utf8(f"padding{i}")

    def add(self, key, data):
        if key not in self.index:
            self.entries.append(data)
            self.index[key] = len(self.entries)
        return self.index[key]

    def utf8(self, text):
        raw = text.encode()
        return self.add(("u", text), b"\x01" + struct.pack(">H", len(raw)) + raw)

    def cls(self, name):
        return self.add(("c", name), b"\x07" + struct.pack(">H", self.utf8(name)))

    def string(self, text):
        return self.add(("s", text), b"\x08" + struct.pack(">H", self.utf8(text)))

    def method(self, owner, name, descriptor):
        nat = self.add(("nt", name, descriptor),
                       b"\x0c" + struct.pack(">HH", self.utf8(name), self.utf8(descriptor)))
        return self.add(("m", owner, name, descriptor), b"\x0a" + struct.pack(">HH", self.cls(owner), nat))

    def bytes(self):
        return struct.pack(">H", len(self.entries) + 1) + b"".join(self.entries)


def class_file(methods, name="a/B", pad=0, wide=False):
    """A class with the given methods: {name: (descriptor, [instructions])}, each instruction
    ("ldc", text) | ("call", owner, name, descriptor) | ("ifnull", +instructions) | (opcode,)."""
    pool = Pool(pad)
    this, parent = pool.cls(name), pool.cls("java/lang/Object")
    code_name = pool.utf8("Code")
    encoded = []
    for method, (descriptor, instructions) in methods.items():
        sizes = [3 if i[0] in ("call", "ifnull") or (i[0] == "ldc" and wide) else 2 if i[0] == "ldc" else 1
                 for i in instructions]
        body = b""
        for at, instruction in enumerate(instructions):
            if instruction[0] == "ldc":
                index = pool.string(instruction[1])
                body += bytes([19]) + struct.pack(">H", index) if wide else bytes([18, index])
            elif instruction[0] == "call":
                body += bytes([184]) + struct.pack(">H", pool.method(*instruction[1:]))
            elif instruction[0] == "ifnull":
                offset = sum(sizes[at:at + instruction[1]])
                body += bytes([198]) + struct.pack(">h", offset)
            else:
                body += bytes([instruction[0]])
        code = struct.pack(">HHI", 2, 1, len(body)) + body + struct.pack(">HH", 0, 0)
        encoded.append(struct.pack(">HHHH", 0x0009, pool.utf8(method), pool.utf8(descriptor), 1)
                       + struct.pack(">HI", code_name, len(code)) + code)
    return (b"\xca\xfe\xba\xbe" + struct.pack(">HH", 0, 69) + pool.bytes()
            + struct.pack(">HHHH", 0x0021, this, parent, 0) + struct.pack(">H", 0)
            + struct.pack(">H", len(encoded)) + b"".join(encoded) + struct.pack(">H", 0))


def index(methods, **options):
    return class_index.ClassFile(class_file(methods, **options)).index()


GREET = {"greet": ("()Ljava/lang/String;", [("ldc", "hi"), ("ifnull", 2), (1,), (176,)])}


class ClassIndexTests(unittest.TestCase):
    def test_constant_pool_layout_and_ldc_width_do_not_change_hashes(self):
        plain = index(GREET)
        self.assertEqual(index(GREET, pad=3), plain)
        self.assertEqual(index(GREET, pad=300, wide=True), plain)

    def test_a_changed_literal_or_call_changes_the_hash(self):
        plain = index(GREET)["methods"]
        other = index({"greet": ("()Ljava/lang/String;", [("ldc", "hello"), ("ifnull", 2), (1,), (176,)])})
        self.assertNotEqual(other["methods"], plain)
        call = {"run": ("()V", [("call", "a/C", "go", "()V"), (177,)])}
        moved = {"run": ("()V", [("call", "b/C", "go", "()V"), (177,)])}
        self.assertNotEqual(index(call)["methods"], index(moved)["methods"])

    def test_lambdas_fold_into_their_declaring_method(self):
        body = ("()V", [(177,)])
        first = index({"run": body, "lambda$run$0": body, "lambda$run$1": ("()V", [(1,), (87,), (177,)])})
        renumbered = index({"run": body, "lambda$run$4": ("()V", [(1,), (87,), (177,)]), "lambda$run$5": body})
        self.assertEqual(set(first["methods"]), {"run()V", "lambda$run"})
        self.assertEqual(first["methods"], renumbered["methods"])

    def test_diff_with_moves(self):
        def entry(**methods):
            return {"access": 33, "super": "java/lang/Object", "interfaces": [], "fields": {}, "methods": methods}
        old = {"a/Same": entry(f="1"), "a/Edit": entry(f="1", g="2"), "a/Gone": entry(),
               "a/Moved": entry(f="1"), "a/x/Twin": entry(), "a/y/Twin": entry()}
        new = {"a/Same": entry(f="1"), "a/Edit": entry(f="9", h="3"), "b/Moved": entry(f="2"), "a/New": entry(),
               "b/Twin": entry()}
        result = class_index.diff(old, new)
        self.assertEqual(result["moved"], {"a/Moved": "b/Moved"})
        self.assertEqual(result["added"], ["a/New", "b/Twin"])
        self.assertEqual(result["removed"], ["a/Gone", "a/x/Twin", "a/y/Twin"])
        self.assertEqual(result["changed"]["a/Edit"],
                         {"methods": {"added": ["h"], "removed": ["g"], "changed": ["f"]}})
        self.assertEqual(result["changed"]["b/Moved"]["moved_from"], "a/Moved")
        self.assertEqual(class_index.changed_methods(result["changed"]["a/Edit"]), 3)


CLASSES = ["net/minecraft/world/food/FoodData", "net/minecraft/world/level/Level",
           "com/mojang/realmsclient/Level", "net/minecraft/world/entity/Entity",
           "net/minecraft/world/entity/Entity$RemovalReason", "net/minecraft/a/Thing$1",
           "net/minecraft/server/level/ServerLevel"]


class VanillaMapTests(unittest.TestCase):
    def setUp(self):
        self.names = vanilla_map.Names(CLASSES)

    def test_resolve(self):
        self.assertEqual(self.names.resolve("FoodData"), "net/minecraft/world/food/FoodData")
        self.assertEqual(self.names.resolve("Level"), "net/minecraft/world/level/Level")
        self.assertEqual(self.names.resolve("RemovalReason"), "net/minecraft/world/entity/Entity$RemovalReason")
        self.assertEqual(self.names.resolve("Entity.RemovalReason"), "net/minecraft/world/entity/Entity$RemovalReason")
        self.assertIsNone(self.names.resolve("Missing"))
        self.assertEqual(self.names.java("net.minecraft.world.entity.Entity.RemovalReason"),
                         "net/minecraft/world/entity/Entity$RemovalReason")

    def test_rust_comments(self):
        files = {"crates/x/src/food.rs": "/// FoodData.tick, as vanilla\nfn tick() {} // ServerLevel too\n"
                                         "let FoodData = 1;\n// a Level, a Level.tick() and Entity.RemovalReason\n"}
        found = vanilla_map.rust_references(files, self.names, read=files.get)
        self.assertEqual(found["net/minecraft/world/food/FoodData"], {"crates/x/src/food.rs": ["tick"]})
        self.assertEqual(found["net/minecraft/server/level/ServerLevel"], {"crates/x/src/food.rs": []})
        # A one-word name counts only with a member: "a Level" alone is English.
        self.assertEqual(found["net/minecraft/world/level/Level"], {"crates/x/src/food.rs": ["tick"]})
        self.assertIn("net/minecraft/world/entity/Entity$RemovalReason", found)

    def test_java_references(self):
        text = ("import net.minecraft.world.food.*;import net.minecraft.world.entity.Entity;\n"
                "import static net.minecraft.world.level.Level.OVERWORLD;\n"
                "class X { FoodData f; net.minecraft.server.level.ServerLevel l; }")
        self.assertEqual(vanilla_map.java_references(text, self.names), {
            "net/minecraft/world/food/FoodData", "net/minecraft/world/entity/Entity",
            "net/minecraft/world/level/Level", "net/minecraft/server/level/ServerLevel"})

    def test_probe_modules(self):
        sources = {"crates/horizon-world/src/food.rs", "crates/horizon-world/src/level/mod.rs",
                   "crates/horizon-world/src/level/tick.rs"}
        text = "use horizon_world::food::FoodData;\nuse horizon_world::level::tick::{a, b};\nuse crate::level;\n"
        self.assertEqual(vanilla_map.module_files("crates/horizon-world/examples/p.rs", text, sources),
                         {"crates/horizon-world/src/food.rs", "crates/horizon-world/src/level/tick.rs",
                          "crates/horizon-world/src/level/mod.rs"})
        self.assertEqual(vanilla_map.probe_files("run(['food_fixture'])", {"food_fixture": "e.rs", "food": "f.rs"}),
                         {"food_fixture": "e.rs"})
        self.assertEqual(vanilla_map.harness_name("tools/test_food_parity.py"), "food")

    def test_lookup_falls_back_to_the_outer_class(self):
        mapping = {"classes": {"a/B": {"rust": {"x.rs": []}, "harnesses": ["h"]}}}
        self.assertEqual(vanilla_map.lookup(mapping, "a/B$C")["harnesses"], ["h"])
        self.assertEqual(vanilla_map.lookup(mapping, "a/D"), {"rust": {}, "harnesses": []})


if __name__ == "__main__":
    unittest.main()
