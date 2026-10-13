package dev.horizonlab.parity;

import com.google.gson.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.gui.screens.packs.PackSelectionModel;
import net.minecraft.network.chat.Component;
import net.minecraft.server.packs.PackLocationInfo;
import net.minecraft.server.packs.PackSelectionConfig;
import net.minecraft.server.packs.repository.Pack;
import net.minecraft.server.packs.repository.PackCompatibility;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.PackSource;
import net.minecraft.world.flag.FeatureFlagSet;

/**
 * Runs the resource pack screen's selection logic: a PackRepository of the
 * case's packs (required, default position, fixed position, compatibility)
 * with an initial selection, a PackSelectionModel over it, and the actions
 * TransferableSelectionList.PackEntry performs on a click or key press:
 * select (an incompatible pack only once its ConfirmScreen is accepted),
 * unselect, move up, move down. After each action: both lists and every
 * entry's canSelect, canUnselect, canMoveUp and canMoveDown; at the end the
 * repository's selection after commit.
 *
 * Input lines: case JSON. Output lines: {steps:[...], committed:[...]} or {error}.
 */
public final class VanillaPackSelectionOracle {
    static JsonObject lists(PackSelectionModel model) {
        var o = new JsonObject();
        for (String side : new String[] {"unselected", "selected"}) {
            var a = new JsonArray();
            var entries = side.equals("selected") ? model.getSelected().toList() : model.getUnselected().toList();
            for (var e : entries) {
                var row = new JsonArray();
                row.add(e.getId());
                row.add((e.canSelect() ? "s" : "") + (e.canUnselect() ? "u" : "") + (e.canMoveUp() ? "^" : "") + (e.canMoveDown() ? "v" : ""));
                a.add(row);
            }
            o.add(side, a);
        }
        return o;
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var gson = new Gson();
        var out = new StringBuilder();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.isBlank()) continue;
            JsonObject c = JsonParser.parseString(line).getAsJsonObject();
            var result = new JsonObject();
            try {
                List<Pack> packs = new ArrayList<>();
                for (var p : c.getAsJsonArray("packs")) {
                    var o = p.getAsJsonObject();
                    String id = o.get("id").getAsString();
                    var location = new PackLocationInfo(id, Component.literal(id), PackSource.DEFAULT, Optional.empty());
                    var compatibility = switch (o.get("compat").getAsString()) {
                        case "old" -> PackCompatibility.TOO_OLD;
                        case "new" -> PackCompatibility.TOO_NEW;
                        default -> PackCompatibility.COMPATIBLE;
                    };
                    var metadata = new Pack.Metadata(Component.literal(id), compatibility, FeatureFlagSet.of(), List.of());
                    var config = new PackSelectionConfig(o.get("required").getAsBoolean(),
                        o.get("position").getAsString().equals("top") ? Pack.Position.TOP : Pack.Position.BOTTOM, o.get("fixed").getAsBoolean());
                    packs.add(new Pack(location, null, metadata, config));
                }
                var repository = new PackRepository(consumer -> packs.forEach(consumer));
                repository.reload();
                List<String> initial = new ArrayList<>();
                for (var s : c.getAsJsonArray("selected")) initial.add(s.getAsString());
                repository.setSelected(initial);
                var model = new PackSelectionModel(e -> { }, p -> null, repository, r -> { });
                var steps = new JsonArray();
                steps.add(lists(model));
                for (var step : c.getAsJsonArray("ops")) {
                    var o = step.getAsJsonObject();
                    var entries = o.get("list").getAsString().equals("selected") ? model.getSelected().toList() : model.getUnselected().toList();
                    if (!entries.isEmpty()) {
                        var e = entries.get(Math.floorMod(o.get("index").getAsInt(), entries.size()));
                        switch (o.get("op").getAsString()) {
                            // PackEntry.keyPressed(select) / a click on the icon: handlePackSelection or unselect.
                            case "activate" -> {
                                if (e.canSelect()) {
                                    if (e.getCompatibility().isCompatible() || o.get("confirm").getAsBoolean()) e.select();
                                } else if (e.canUnselect()) {
                                    e.unselect();
                                }
                            }
                            case "up" -> { if (e.canMoveUp()) e.moveUp(); }
                            case "down" -> { if (e.canMoveDown()) e.moveDown(); }
                            default -> throw new IllegalArgumentException(o.toString());
                        }
                    }
                    steps.add(lists(model));
                }
                result.add("steps", steps);
                model.commit();
                var committed = new JsonArray();
                for (var p : repository.getSelectedPacks()) committed.add(p.getId());
                result.add("committed", committed);
            } catch (Throwable error) {
                result.addProperty("error", String.valueOf(error));
            }
            out.append(gson.toJson(result)).append('\n');
        }
        Files.writeString(Path.of(args[1]), out.toString(), StandardCharsets.UTF_8);
    }
}
