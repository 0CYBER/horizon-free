package dev.horizonlab.parity;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.ParseResults;
import com.mojang.brigadier.StringReader;
import com.mojang.brigadier.exceptions.CommandSyntaxException;
import com.mojang.brigadier.context.CommandContextBuilder;
import com.mojang.brigadier.context.ParsedCommandNode;
import com.mojang.brigadier.context.ContextChain;
import com.mojang.brigadier.suggestion.Suggestion;
import com.mojang.brigadier.suggestion.Suggestions;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.Set;
import java.util.TreeSet;
import net.minecraft.SharedConstants;
import net.minecraft.commands.CommandBuildContext;
import net.minecraft.commands.CommandSource;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.commands.SharedSuggestionProvider;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.ReloadableServerRegistries;
import net.minecraft.server.ServerScoreboard;
import net.minecraft.server.dedicated.DedicatedServer;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.server.permissions.PermissionLevel;
import net.minecraft.server.permissions.PermissionSet;
import net.minecraft.server.players.PlayerList;
import net.minecraft.tags.TagLoader;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.phys.Vec2;
import net.minecraft.world.phys.Vec3;

/**
 * Runs a corpus of command strings through vanilla's real Brigadier command
 * tree and prints, per input, whether it parses, the error (message and
 * cursor), the parsed nodes and the completion suggestions at the end of the
 * input. Also exports the registry contents the Rust side needs.
 *
 * usage: registries|run OUT_DIR [CORPUS PERMISSION_LEVEL]
 */
public final class VanillaCommandParseOracle {
    static sun.misc.Unsafe unsafe;
    static final String[] PLAYERS = {"Player", "Steve", "Alex"};
    static final String[] TEAMS = {"red", "blue"};
    static final String[] OBJECTIVES = {"kills", "health", "obj"};
    static final String[] TRACKED = {"#total", "Steve", "fake"};

    static final class Src extends CommandSourceStack {
        final RegistryAccess access;
        final HolderLookup.Provider lookup;
        Src(MinecraftServer server, PermissionSet perms, RegistryAccess access, HolderLookup.Provider lookup) {
            super(CommandSource.NULL, Vec3.ZERO, Vec2.ZERO, null, perms, Component.literal("Player"), server);
            this.access = access;
            this.lookup = lookup;
        }
        @Override public Collection<String> getOnlinePlayerNames() { return new ArrayList<>(List.of(PLAYERS)); }
        @Override public Collection<String> getAllTeams() { return new ArrayList<>(List.of(TEAMS)); }
        @Override public RegistryAccess registryAccess() { return access; }
        @Override public Set<ResourceKey<Level>> levels() { return Set.of(Level.OVERWORLD, Level.NETHER, Level.END); }
        @Override public FeatureFlagSet enabledFeatures() { return FeatureFlags.VANILLA_SET; }
        @Override public <E> java.util.concurrent.CompletableFuture<Suggestions> suggestRegistryElements(ResourceKey<? extends Registry<E>> key, SharedSuggestionProvider.ElementSuggestionType elements, com.mojang.brigadier.suggestion.SuggestionsBuilder builder, com.mojang.brigadier.context.CommandContext<?> context, java.util.function.Predicate<E> filter) {
            java.util.Optional<? extends HolderLookup<E>> registry = access.lookup(key);
            if (registry.isEmpty()) registry = lookup.lookup(key);
            if (registry.isEmpty()) return Suggestions.empty();
            this.suggestRegistryElements(registry.get(), elements, builder, filter);
            return builder.buildFuture();
        }
    }

    static void put(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        long offset = unsafe.objectFieldOffset(f);
        unsafe.putObject(target, offset, value);
    }

    static String message(CommandSyntaxException e) {
        com.mojang.brigadier.Message m = e.getRawMessage();
        return m.getString();
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var unsafeField = sun.misc.Unsafe.class.getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = (sun.misc.Unsafe) unsafeField.get(null);
        Path out = Path.of(args[1]);
        var repo = ServerPacksSource.createVanillaTrustedRepository();
        repo.reload();
        repo.setSelected(List.of("vanilla"));
        var resources = new MultiPackResourceManager(PackType.SERVER_DATA, repo.openAllSelected());
        var layers = RegistryLayer.createRegistryAccess();
        var staticTags = TagLoader.loadTagsForExistingRegistries(resources, layers.getLayer(RegistryLayer.STATIC));
        var context = TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD), staticTags);
        RegistryAccess.Frozen world = RegistryDataLoader.load(resources, context, RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
        var dims = RegistryDataLoader.load(resources, java.util.stream.Stream.concat(context.stream(), world.listRegistries()).toList(), RegistryDataLoader.DIMENSION_REGISTRIES, Runnable::run).join();
        var worldLayers = layers.replaceFrom(RegistryLayer.WORLD, world, dims);
        var loaded = ReloadableServerRegistries.reload(worldLayers, staticTags, resources, Runnable::run).join();
        HolderLookup.Provider lookup = loaded.lookupWithUpdatedTags();
        var components = BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(lookup);
        staticTags.forEach(Registry.PendingTags::apply);
        components.forEach(net.minecraft.core.component.DataComponentInitializers.PendingComponents::apply);
        RegistryAccess access = loaded.layers().compositeAccess();

        if (args[0].equals("registries")) {
            exportRegistries(out, access, lookup);
            return;
        }
        FeatureFlagSet features = FeatureFlags.VANILLA_SET;
        CommandBuildContext buildContext = CommandBuildContext.simple(lookup, features);
        Commands commands = new Commands(Commands.CommandSelection.INTEGRATED, buildContext);
        CommandDispatcher<CommandSourceStack> dispatcher = commands.getDispatcher();

        if (args[0].equals("tree")) {
            Files.writeString(out.resolve("tree.json"), net.minecraft.commands.synchronization.ArgumentUtils.serializeNodeToJson(dispatcher, dispatcher.getRoot()).toString());
            return;
        }
        MinecraftServer server = (MinecraftServer) unsafe.allocateInstance(DedicatedServer.class);
        PlayerList playerList = (PlayerList) unsafe.allocateInstance(net.minecraft.server.dedicated.DedicatedPlayerList.class);
        put(playerList, PlayerList.class, "players", new ArrayList<>());
        put(server, MinecraftServer.class, "playerList", playerList);
        ServerScoreboard scoreboard = new ServerScoreboard(server);
        put(server, MinecraftServer.class, "scoreboard", scoreboard);
        for (String team : TEAMS) scoreboard.addPlayerTeam(team);
        for (String objective : OBJECTIVES)
            scoreboard.addObjective(objective, net.minecraft.world.scores.criteria.ObjectiveCriteria.DUMMY, Component.literal(objective),
                net.minecraft.world.scores.criteria.ObjectiveCriteria.RenderType.INTEGER, true, null);
        Object holder = null;
        for (String tracked : TRACKED)
            scoreboard.getOrCreatePlayerScore(net.minecraft.world.scores.ScoreHolder.forNameOnly(tracked), scoreboard.getObjective("kills"));
        PermissionSet perms = LevelBasedPermissionSet.forLevel(PermissionLevel.byId(Integer.parseInt(args[3])));
        Src source = new Src(server, perms, access, lookup);

        List<String> lines = Files.readAllLines(Path.of(args[2]), StandardCharsets.UTF_8);
        java.io.BufferedWriter sb = Files.newBufferedWriter(out.resolve("vanilla.jsonl"));
        for (String raw : lines) {
            String input = raw;
            JsonObject row = new JsonObject();
            row.addProperty("in", input);
            try {
                ParseResults<CommandSourceStack> parse = dispatcher.parse(new StringReader(input), source);
                JsonObject result = new JsonObject();
                String error = null;
                int cursor = 0;
                if (parse.getReader().canRead()) {
                    if (parse.getExceptions().size() == 1) {
                        CommandSyntaxException e = parse.getExceptions().values().iterator().next();
                        error = message(e);
                        cursor = e.getCursor();
                    } else if (parse.getContext().getRange().isEmpty()) {
                        error = Component.translatable("command.unknown.command").getString();
                        cursor = parse.getReader().getCursor();
                    } else {
                        error = Component.translatable("command.unknown.argument").getString();
                        cursor = parse.getReader().getCursor();
                    }
                } else {
                    var ctx = parse.getContext().build(input);
                    if (ContextChain.tryFlatten(ctx).isEmpty()) {
                        error = Component.translatable("command.unknown.command").getString();
                        cursor = parse.getReader().getCursor();
                    }
                }
                if (error == null) {
                    result.addProperty("ok", true);
                    JsonArray nodes = new JsonArray();
                    CommandContextBuilder<CommandSourceStack> b = parse.getContext();
                    while (b != null) {
                        for (ParsedCommandNode<CommandSourceStack> n : b.getNodes()) nodes.add(n.getNode().getName());
                        b = b.getChild();
                    }
                    result.add("nodes", nodes);
                } else {
                    result.addProperty("ok", false);
                    result.addProperty("error", error);
                    result.addProperty("cursor", cursor);
                }
                row.add("parse", result);
                Suggestions suggestions = dispatcher.getCompletionSuggestions(parse, input.length()).get();
                JsonObject sg = new JsonObject();
                sg.addProperty("start", suggestions.getRange().getStart());
                sg.addProperty("end", suggestions.getRange().getEnd());
                JsonArray list = new JsonArray();
                for (Suggestion s : suggestions.getList()) {
                    String text = s.getText();
                    if (s.getTooltip() != null) text += "\t" + s.getTooltip().getString();
                    list.add(text);
                }
                sg.add("list", list);
                row.add("sugg", sg);
            } catch (Throwable t) {
                Throwable root = t;
                while (root.getCause() != null) root = root.getCause();
                StackTraceElement[] st = root.getStackTrace();
                row.addProperty("crash", root + (st.length > 0 ? " @ " + st[0] : ""));
            }
            sb.write(row.toString());
            sb.write("\n");
        }
        sb.close();
    }

    static void exportRegistries(Path out, RegistryAccess access, HolderLookup.Provider lookup) throws Exception {
        JsonObject o = new JsonObject();
        JsonArray items = new JsonArray();
        for (var item : BuiltInRegistries.ITEM) {
            if (FeatureFlags.VANILLA_SET.isSubsetOf(item.requiredFeatures()))
                items.add(BuiltInRegistries.ITEM.getKey(item).toString());
        }
        o.add("items", items);
        JsonObject blocks = new JsonObject();
        for (Block block : BuiltInRegistries.BLOCK) {
            if (!FeatureFlags.VANILLA_SET.isSubsetOf(block.requiredFeatures())) continue;
            JsonArray props = new JsonArray();
            for (Property<?> p : block.getStateDefinition().getProperties()) {
                JsonArray values = new JsonArray();
                for (Object v : p.getPossibleValues()) values.add(((Property) p).getName((Comparable) v));
                JsonArray pair = new JsonArray();
                pair.add(p.getName());
                pair.add(values);
                props.add(pair);
            }
            blocks.add(BuiltInRegistries.BLOCK.getKey(block).toString(), props);
        }
        o.add("blocks", blocks);
        o.add("item_tags", tags(lookup, Registries.ITEM));
        o.add("block_tags", tags(lookup, Registries.BLOCK));
        JsonArray summonable = new JsonArray();
        JsonArray types = new JsonArray();
        for (var type : BuiltInRegistries.ENTITY_TYPE) {
            if (!FeatureFlags.VANILLA_SET.isSubsetOf(type.requiredFeatures())) continue;
            String id = BuiltInRegistries.ENTITY_TYPE.getKey(type).toString();
            types.add(id);
            if (type.canSummon()) summonable.add(id);
        }
        o.add("entity_types", types);
        o.add("summonable", summonable);
        o.add("effects", ids(BuiltInRegistries.MOB_EFFECT));
        o.add("attributes", ids(BuiltInRegistries.ATTRIBUTE));
        o.add("sounds", ids(BuiltInRegistries.SOUND_EVENT));
        o.add("enchantments", dynamic(access, Registries.ENCHANTMENT));
        o.add("structures", dynamic(access, Registries.STRUCTURE));
        o.add("biomes", dynamic(access, Registries.BIOME));
        o.add("damage_types", dynamic(access, Registries.DAMAGE_TYPE));
        o.add("clocks", dynamic(access, Registries.WORLD_CLOCK));
        o.add("timelines", dynamic(access, Registries.TIMELINE));
        o.add("dimensions", arr("minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"));
        o.add("players", arr(PLAYERS));
        o.add("teams", arr(TEAMS));
        o.add("objectives", arr(OBJECTIVES));
        o.add("tracked", arr(TRACKED));
        JsonObject by = new JsonObject();
        lookup.listRegistryKeys().sorted(java.util.Comparator.comparing(k -> k.identifier().toString())).forEach(key -> {
            var reg = lookup.lookupOrThrow(key);
            JsonObject entry = new JsonObject();
            java.util.List<String> ids = reg.listElementIds().map(k -> k.identifier().toString()).sorted().toList();
            JsonArray idArr = new JsonArray();
            ids.forEach(idArr::add);
            entry.add("ids", idArr);
            JsonArray tagArr = new JsonArray();
            reg.listTagIds().map(t -> t.location().toString()).sorted().forEach(tagArr::add);
            entry.add("tags", tagArr);
            by.add(key.identifier().toString(), entry);
        });
        o.add("by_registry", by);
        Files.writeString(out.resolve("registries.json"), o.toString());
    }

    static JsonArray arr(String... values) {
        JsonArray a = new JsonArray();
        for (String v : values) a.add(v);
        return a;
    }

    static <T> JsonArray ids(Registry<T> registry) {
        Set<String> sorted = new TreeSet<>();
        for (var key : registry.keySet()) sorted.add(key.toString());
        JsonArray a = new JsonArray();
        sorted.forEach(a::add);
        return a;
    }

    static <T> JsonArray dynamic(RegistryAccess access, ResourceKey<? extends Registry<T>> key) {
        return ids(access.lookupOrThrow(key));
    }

    static <T> JsonArray tags(HolderLookup.Provider lookup, ResourceKey<? extends Registry<T>> key) {
        Set<String> sorted = new TreeSet<>();
        lookup.lookupOrThrow(key).listTagIds().forEach(t -> sorted.add(t.location().toString()));
        JsonArray a = new JsonArray();
        sorted.forEach(a::add);
        return a;
    }
}
