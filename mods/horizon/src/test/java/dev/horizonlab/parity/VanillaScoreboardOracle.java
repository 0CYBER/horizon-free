package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.nbt.*;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.ComponentSerialization;
import net.minecraft.network.chat.numbers.NumberFormat;
import net.minecraft.network.chat.numbers.NumberFormatTypes;
import net.minecraft.world.scores.*;
import net.minecraft.world.scores.criteria.ObjectiveCriteria;

/**
 * Runs vanilla's own {@link Scoreboard} (the model {@code ServerScoreboard} extends: objectives, criteria lists,
 * scores, locks, display slots, teams) on scripted operations and records each result, dumps of every collection
 * in the order vanilla iterates it, and the {@code ScoreboardSaveData.Packed} codec output that
 * {@code data/minecraft/scoreboard.dat} stores, before and after loading it back.
 *
 * <pre>
 *   export DIR                 registries the Rust fixture needs (DIR/registries.json)
 *   run INPUTS OUTPUT          one JSON line per case
 *   reload INPUTS OUTPUT       load each line's hex NBT ({"case", "nbt"}) and pack it again
 * </pre>
 */
public final class VanillaScoreboardOracle {
    /** {@code ServerScoreboard.load} and {@code store} without the server. */
    static final class Board extends Scoreboard {
        void load(ScoreboardSaveData.Packed data) {
            data.objectives().forEach(this::loadObjective);
            data.scores().forEach(this::loadPlayerScore);
            data.displaySlots().forEach((slot, name) -> {
                Objective objective = this.getObjective(name);
                this.setDisplayObjective(slot, objective);
            });
            data.teams().forEach(this::loadPlayerTeam);
        }

        ScoreboardSaveData.Packed store() {
            return new ScoreboardSaveData.Packed(this.packObjectives(), this.packPlayerScores(), this.packDisplaySlots(), this.packPlayerTeams());
        }
    }

    static JsonElement json(Component c) {
        return c == null ? JsonNull.INSTANCE : ComponentSerialization.CODEC.encodeStart(JsonOps.INSTANCE, c).getOrThrow();
    }

    static JsonElement json(NumberFormat f) {
        return f == null ? JsonNull.INSTANCE : NumberFormatTypes.CODEC.encodeStart(JsonOps.INSTANCE, f).getOrThrow();
    }

    static Component component(JsonElement e) {
        return e == null || e.isJsonNull() ? null : ComponentSerialization.CODEC.parse(JsonOps.INSTANCE, e).getOrThrow();
    }

    static NumberFormat format(JsonElement e) {
        return e == null || e.isJsonNull() ? null : NumberFormatTypes.CODEC.parse(JsonOps.INSTANCE, e).getOrThrow();
    }

    /** NBT as JSON: compounds with sorted keys, lists as arrays, and typed scalars. */
    static JsonElement canon(Tag tag) {
        if (tag instanceof CompoundTag c) {
            JsonObject o = new JsonObject();
            for (String k : new TreeSet<>(c.keySet())) {
                o.add(k, canon(c.get(k)));
            }
            return o;
        }
        if (tag instanceof ListTag l) {
            JsonArray a = new JsonArray();
            for (Tag t : l) {
                a.add(canon(t));
            }
            return a;
        }
        if (tag instanceof StringTag(String s)) {
            return new JsonPrimitive("'" + s);
        }
        if (tag instanceof ByteTag b) {
            return new JsonPrimitive(b.value() + "b");
        }
        if (tag instanceof ShortTag s) {
            return new JsonPrimitive(s.value() + "s");
        }
        if (tag instanceof IntTag i) {
            return new JsonPrimitive(i.value() + "i");
        }
        if (tag instanceof LongTag l) {
            return new JsonPrimitive(l.value() + "L");
        }
        if (tag instanceof FloatTag f) {
            return new JsonPrimitive("f:" + Integer.toHexString(Float.floatToRawIntBits(f.value())));
        }
        if (tag instanceof DoubleTag d) {
            return new JsonPrimitive("d:" + Long.toHexString(Double.doubleToRawLongBits(d.value())));
        }
        if (tag instanceof IntArrayTag a) {
            JsonObject o = new JsonObject();
            JsonArray v = new JsonArray();
            for (int i : a.getAsIntArray()) {
                v.add(i);
            }
            o.add("I;", v);
            return o;
        }
        return new JsonPrimitive("?" + tag.getType().getName());
    }

    static String hex(CompoundTag tag) throws IOException {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        try (DataOutputStream out = new DataOutputStream(bytes)) {
            NbtIo.write(tag, out);
        }
        return HexFormat.of().formatHex(bytes.toByteArray());
    }

    static CompoundTag unhex(String text) throws IOException {
        try (DataInputStream in = new DataInputStream(new ByteArrayInputStream(HexFormat.of().parseHex(text)))) {
            return NbtIo.read(in);
        }
    }

    static CompoundTag pack(Board board) {
        return (CompoundTag) ScoreboardSaveData.Packed.CODEC.encodeStart(NbtOps.INSTANCE, board.store()).getOrThrow();
    }

    static Board unpack(Tag tag) {
        Board board = new Board();
        board.load(ScoreboardSaveData.Packed.CODEC.parse(NbtOps.INSTANCE, tag).getOrThrow());
        return board;
    }

    static ScoreHolder holder(JsonObject op) {
        String name = op.get("holder").getAsString();
        Component display = component(op.get("dname"));
        return new ScoreHolder() {
            @Override
            public String getScoreboardName() {
                return name;
            }

            @Override
            public Component getDisplayName() {
                return display;
            }
        };
    }

    static String criteria(String name) {
        try {
            return ObjectiveCriteria.byName(name).map(ObjectiveCriteria::getName).orElse(null);
        } catch (RuntimeException e) {
            return "!invalid";
        }
    }

    static Team.Visibility visibility(String name) {
        for (Team.Visibility v : Team.Visibility.values()) {
            if (v.getSerializedName().equals(name)) {
                return v;
            }
        }
        throw new IllegalArgumentException(name);
    }

    static Team.CollisionRule collision(String name) {
        for (Team.CollisionRule v : Team.CollisionRule.values()) {
            if (v.getSerializedName().equals(name)) {
                return v;
            }
        }
        throw new IllegalArgumentException(name);
    }

    static JsonObject score(ReadOnlyScoreInfo info) {
        JsonObject o = new JsonObject();
        Score s = (Score) info;
        o.addProperty("value", s.value());
        o.addProperty("locked", s.isLocked());
        o.add("display", json(s.display()));
        o.add("format", json(s.numberFormat()));
        return o;
    }

    static JsonObject dump(Board board, JsonArray probes) {
        JsonObject d = new JsonObject();
        JsonArray objectives = new JsonArray();
        for (Objective o : board.getObjectives()) {
            JsonObject j = new JsonObject();
            j.addProperty("name", o.getName());
            j.addProperty("criteria", o.getCriteria().getName());
            j.add("display", json(o.getDisplayName()));
            j.addProperty("render", o.getRenderType().getSerializedName());
            j.addProperty("auto", o.displayAutoUpdate());
            j.add("format", json(o.numberFormat()));
            objectives.add(j);
        }
        d.add("objectives", objectives);
        JsonArray stream = new JsonArray();
        board.getObjectives().stream().forEach(o -> stream.add(o.getName()));
        d.add("objectives_stream", stream);
        JsonArray tracked = new JsonArray();
        for (ScoreHolder h : board.getTrackedPlayers()) {
            tracked.add(h.getScoreboardName());
        }
        d.add("tracked", tracked);
        JsonObject lists = new JsonObject();
        JsonObject holders = new JsonObject();
        for (Objective o : board.getObjectives()) {
            JsonArray rows = new JsonArray();
            for (PlayerScoreEntry e : board.listPlayerScores(o)) {
                JsonArray row = new JsonArray();
                row.add(e.owner());
                row.add(e.value());
                row.add(json(e.display()));
                row.add(json(e.numberFormatOverride()));
                rows.add(row);
            }
            lists.add(o.getName(), rows);
        }
        d.add("lists", lists);
        for (ScoreHolder h : board.getTrackedPlayers()) {
            JsonObject per = new JsonObject();
            for (Objective o : board.getObjectives()) {
                ReadOnlyScoreInfo info = board.getPlayerScoreInfo(h, o);
                if (info != null) {
                    per.add(o.getName(), score(info));
                }
            }
            holders.add(h.getScoreboardName(), per);
        }
        d.add("holders", holders);
        JsonArray teams = new JsonArray();
        for (PlayerTeam t : board.getPlayerTeams()) {
            JsonObject j = new JsonObject();
            j.addProperty("name", t.getName());
            j.add("display", json(t.getDisplayName()));
            j.addProperty("color", t.getColor().map(TeamColor::getSerializedName).orElse(null));
            j.addProperty("ff", t.isAllowFriendlyFire());
            j.addProperty("si", t.canSeeFriendlyInvisibles());
            j.add("prefix", json(t.getPlayerPrefix()));
            j.add("suffix", json(t.getPlayerSuffix()));
            j.addProperty("nametag", t.getNameTagVisibility().getSerializedName());
            j.addProperty("death", t.getDeathMessageVisibility().getSerializedName());
            j.addProperty("collision", t.getCollisionRule().getSerializedName());
            JsonArray players = new JsonArray();
            for (String p : t.getPlayers()) {
                players.add(p);
            }
            j.add("players", players);
            teams.add(j);
        }
        d.add("teams", teams);
        JsonArray teamStream = new JsonArray();
        board.getPlayerTeams().stream().forEach(t -> teamStream.add(t.getName()));
        d.add("teams_stream", teamStream);
        JsonObject slots = new JsonObject();
        for (DisplaySlot slot : DisplaySlot.values()) {
            Objective o = board.getDisplayObjective(slot);
            if (o != null) {
                slots.addProperty(slot.getSerializedName(), o.getName());
            }
        }
        d.add("slots", slots);
        JsonObject teamOf = new JsonObject();
        for (JsonElement p : probes) {
            PlayerTeam t = board.getPlayersTeam(p.getAsString());
            teamOf.addProperty(p.getAsString(), t == null ? null : t.getName());
        }
        d.add("team_of", teamOf);
        return d;
    }

    static JsonElement apply(Board board, JsonObject op, JsonArray probes) {
        String kind = op.get("op").getAsString();
        switch (kind) {
            case "crit": {
                String name = criteria(op.get("name").getAsString());
                JsonObject o = new JsonObject();
                o.addProperty("name", name);
                if (name != null && !name.startsWith("!")) {
                    ObjectiveCriteria c = ObjectiveCriteria.byName(name).orElseThrow();
                    o.addProperty("readonly", c.isReadOnly());
                    o.addProperty("render", c.getDefaultRenderType().getSerializedName());
                }
                return o;
            }
            case "obj_add": {
                String name = op.get("name").getAsString();
                String criteria = criteria(op.get("criteria").getAsString());
                if (criteria == null || criteria.startsWith("!")) {
                    return new JsonPrimitive("bad");
                }
                if (board.getObjective(name) != null) {
                    return new JsonPrimitive("exists");
                }
                board.addObjective(name, ObjectiveCriteria.byName(criteria).orElseThrow(), component(op.get("display")),
                    ObjectiveCriteria.RenderType.byId(op.get("render").getAsString()), op.get("auto").getAsBoolean(), format(op.get("fmt")));
                return new JsonPrimitive("ok");
            }
            case "obj_rm": {
                Objective o = board.getObjective(op.get("name").getAsString());
                if (o == null) {
                    return new JsonPrimitive("missing");
                }
                board.removeObjective(o);
                return new JsonPrimitive("ok");
            }
            case "obj_mod": {
                Objective o = board.getObjective(op.get("name").getAsString());
                if (o == null) {
                    return new JsonPrimitive("missing");
                }
                JsonElement v = op.get("value");
                switch (op.get("field").getAsString()) {
                    case "display" -> o.setDisplayName(component(v));
                    case "render" -> o.setRenderType(ObjectiveCriteria.RenderType.byId(v.getAsString()));
                    case "auto" -> o.setDisplayAutoUpdate(v.getAsBoolean());
                    case "fmt" -> o.setNumberFormat(format(v));
                    default -> throw new IllegalArgumentException(op.toString());
                }
                return new JsonPrimitive("ok");
            }
            case "slot": {
                DisplaySlot slot = DisplaySlot.CODEC.byName(op.get("slot").getAsString());
                JsonElement name = op.get("name");
                Objective o = name.isJsonNull() ? null : board.getObjective(name.getAsString());
                if (!name.isJsonNull() && o == null) {
                    return new JsonPrimitive("missing");
                }
                board.setDisplayObjective(slot, o);
                return new JsonPrimitive("ok");
            }
            case "set":
            case "add":
            case "get":
            case "lock":
            case "unlock":
            case "sdisplay":
            case "sfmt": {
                Objective o = board.getObjective(op.get("name").getAsString());
                if (o == null) {
                    return new JsonPrimitive("missing");
                }
                ScoreAccess access = board.getOrCreatePlayerScore(holder(op), o, op.has("force") && op.get("force").getAsBoolean());
                try {
                    switch (kind) {
                        case "set" -> access.set(op.get("value").getAsInt());
                        case "add" -> {
                            return new JsonPrimitive(access.add(op.get("value").getAsInt()));
                        }
                        case "get" -> {
                            return new JsonPrimitive(access.get());
                        }
                        case "lock" -> access.lock();
                        case "unlock" -> access.unlock();
                        case "sdisplay" -> access.display(component(op.get("value")));
                        case "sfmt" -> access.numberFormatOverride(format(op.get("value")));
                        default -> throw new AssertionError();
                    }
                } catch (IllegalStateException e) {
                    return new JsonPrimitive("readonly");
                }
                return new JsonPrimitive("ok");
            }
            case "info": {
                Objective o = board.getObjective(op.get("name").getAsString());
                if (o == null) {
                    return new JsonPrimitive("missing");
                }
                ReadOnlyScoreInfo info = board.getPlayerScoreInfo(holder(op), o);
                return info == null ? JsonNull.INSTANCE : score(info);
            }
            case "reset": {
                JsonElement name = op.get("name");
                if (name.isJsonNull()) {
                    board.resetAllPlayerScores(holder(op));
                } else {
                    Objective o = board.getObjective(name.getAsString());
                    if (o == null) {
                        return new JsonPrimitive("missing");
                    }
                    board.resetSinglePlayerScore(holder(op), o);
                }
                return new JsonPrimitive("ok");
            }
            case "all": {
                ObjectiveCriteria c = ObjectiveCriteria.byName(op.get("criteria").getAsString()).orElseThrow();
                int value = op.get("value").getAsInt();
                switch (op.get("kind").getAsString()) {
                    case "inc" -> board.forAllObjectives(c, holder(op), ScoreAccess::increment);
                    case "add" -> board.forAllObjectives(c, holder(op), s -> s.add(value));
                    case "set" -> board.forAllObjectives(c, holder(op), s -> s.set(value));
                    case "reset" -> board.forAllObjectives(c, holder(op), ScoreAccess::reset);
                    default -> throw new IllegalArgumentException(op.toString());
                }
                return new JsonPrimitive("ok");
            }
            case "team_add": {
                String name = op.get("name").getAsString();
                if (board.getPlayerTeam(name) != null) {
                    return new JsonPrimitive("exists");
                }
                board.addPlayerTeam(name);
                return new JsonPrimitive("ok");
            }
            case "team_rm": {
                PlayerTeam t = board.getPlayerTeam(op.get("name").getAsString());
                if (t == null) {
                    return new JsonPrimitive("missing");
                }
                board.removePlayerTeam(t);
                return new JsonPrimitive("ok");
            }
            case "join": {
                PlayerTeam t = board.getPlayerTeam(op.get("team").getAsString());
                if (t == null) {
                    return new JsonPrimitive("missing");
                }
                return new JsonPrimitive(board.addPlayerToTeam(op.get("holder").getAsString(), t));
            }
            case "leave":
                return new JsonPrimitive(board.removePlayerFromTeam(op.get("holder").getAsString()));
            case "team_set": {
                PlayerTeam t = board.getPlayerTeam(op.get("name").getAsString());
                if (t == null) {
                    return new JsonPrimitive("missing");
                }
                JsonElement v = op.get("value");
                switch (op.get("field").getAsString()) {
                    case "display" -> t.setDisplayName(component(v));
                    case "color" -> t.setColor(v.isJsonNull() ? Optional.empty() : Optional.of(TeamColor.byName(v.getAsString())));
                    case "ff" -> t.setAllowFriendlyFire(v.getAsBoolean());
                    case "si" -> t.setSeeFriendlyInvisibles(v.getAsBoolean());
                    case "prefix" -> t.setPlayerPrefix(component(v));
                    case "suffix" -> t.setPlayerSuffix(component(v));
                    case "nametag" -> t.setNameTagVisibility(visibility(v.getAsString()));
                    case "death" -> t.setDeathMessageVisibility(visibility(v.getAsString()));
                    case "collision" -> t.setCollisionRule(collision(v.getAsString()));
                    default -> throw new IllegalArgumentException(op.toString());
                }
                return new JsonPrimitive("ok");
            }
            case "removed": {
                // Scoreboard.entityRemoved for a dead entity that is not a player.
                ScoreHolder h = holder(op);
                board.resetAllPlayerScores(h);
                board.removePlayerFromTeam(h.getScoreboardName());
                return new JsonPrimitive("ok");
            }
            case "allied": {
                PlayerTeam a = board.getPlayersTeam(op.get("a").getAsString());
                PlayerTeam b = board.getPlayersTeam(op.get("b").getAsString());
                return new JsonPrimitive(a != null && a.isAlliedTo(b));
            }
            case "dump":
                return dump(board, probes);
            default:
                throw new IllegalArgumentException(op.toString());
        }
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        Gson gson = new GsonBuilder().disableHtmlEscaping().serializeNulls().create();
        if (args[0].equals("export")) {
            JsonObject r = new JsonObject();
            JsonArray blocks = new JsonArray();
            BuiltInRegistries.BLOCK.keySet().forEach(k -> blocks.add(k.toString()));
            JsonArray items = new JsonArray();
            BuiltInRegistries.ITEM.keySet().forEach(k -> items.add(k.toString()));
            JsonArray entities = new JsonArray();
            BuiltInRegistries.ENTITY_TYPE.keySet().forEach(k -> entities.add(k.toString()));
            JsonArray custom = new JsonArray();
            BuiltInRegistries.CUSTOM_STAT.keySet().forEach(k -> custom.add(k.toString()));
            r.add("blocks", blocks);
            r.add("items", items);
            r.add("entity_types", entities);
            r.add("custom_stats", custom);
            Files.writeString(Path.of(args[1]).resolve("registries.json"), gson.toJson(r));
            return;
        }
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[2]), StandardCharsets.UTF_8));
             BufferedReader in = Files.newBufferedReader(Path.of(args[1]), StandardCharsets.UTF_8)) {
            String line;
            while ((line = in.readLine()) != null) {
                JsonObject c = JsonParser.parseString(line).getAsJsonObject();
                JsonObject result = new JsonObject();
                result.add("case", c.get("case"));
                try {
                    if (args[0].equals("reload")) {
                        Board board = unpack(unhex(c.get("nbt").getAsString()));
                        result.add("pack", canon(pack(board)));
                    } else {
                        Board board = new Board();
                        JsonArray probes = c.getAsJsonArray("probes");
                        JsonArray outs = new JsonArray();
                        for (JsonElement op : c.getAsJsonArray("ops")) {
                            outs.add(apply(board, op.getAsJsonObject(), probes));
                        }
                        result.add("out", outs);
                        CompoundTag packed = pack(board);
                        result.add("pack", canon(packed));
                        result.addProperty("nbt", hex(packed));
                        result.add("reload", canon(pack(unpack(packed))));
                        result.add("reload_dump", dump(unpack(packed), probes));
                    }
                } catch (RuntimeException e) {
                    result.addProperty("error", e.toString());
                }
                out.println(gson.toJson(result));
            }
        }
    }
}
