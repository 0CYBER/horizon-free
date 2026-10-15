"""Generate a production-size synthetic document; no prepared assets required."""
import json
import pathlib
import sys

woods = "oak spruce birch jungle acacia dark_oak mangrove cherry bamboo pale_oak test_wood".split()
carts = "minecart chest_minecart hopper_minecart furnace_minecart tnt_minecart command_block_minecart".split()
keys = [f"minecraft:{prefix}/{wood}#main" for prefix in ("boat", "chest_boat") for wood in woods]
keys += [f"minecraft:{cart}#main" for cart in carts]
keys += [f"minecraft:{model}#{slot}" for model in ("player", "armor_stand", "armor_stand_small")
         for slot in ("helmet", "chestplate", "leggings", "boots")]
keys += [f"minecraft:{model}#main" for model in ("armor_stand", "armor_stand_small")]

face = [[0, 0, 0, 0, 0], [16, 0, 0, 1, 0], [16, 16, 0, 1, 1], [0, 16, 0, 0, 1]]
part = {"pose": [1, 2, 3, 0.25, -0.5, 0.75, 1, 1, 1], "polygons": [face] * 6,
        "normals": [[0, 0, 1]] * 6}
model = {"pose": [0, 0, 0, 0, 0, 0, 1, 1, 1],
         "children": {f"part_{i}": part for i in range(12)}}
encoded = json.dumps(model, separators=(",", ":"))
entries = [json.dumps(key) + ":" + encoded for key in keys]
size = 2 + sum(map(len, entries)) + len(entries) - 1
index = 0
while size < 2_470_000:
    entry = json.dumps(f"minecraft:fixture_{index}#main") + ":" + encoded
    entries.append(entry)
    size += 1 + len(entry)
    index += 1
output = pathlib.Path(sys.argv[1])
output.write_text("{" + ",".join(entries) + "}")
print(json.dumps({"path": str(output), "bytes": output.stat().st_size,
                  "layers": len(entries), "selected_layers": len(keys)}))
