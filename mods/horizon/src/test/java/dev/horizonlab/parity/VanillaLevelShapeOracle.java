package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.PrintWriter;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Proxy;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.tags.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.attribute.EnvironmentAttributeReader;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.ticks.LevelTickAccess;
import net.minecraft.world.ticks.TickPriority;

/**
 * Native BlockState.updateShape and canSurvive over small generated neighbourhoods.
 * The level is a reflective LevelReader over the case cells; scheduled ticks requested
 * through ScheduledTickAccess are recorded instead of queued.
 */
public final class VanillaLevelShapeOracle {
    static void bindFluidTags(Path dir) throws Exception {
        var registry = BuiltInRegistries.FLUID;
        var loader = new TagLoader<Holder<Fluid>>((id, required) -> registry.get(id).map(value -> (Holder<Fluid>) value), "tags/fluid");
        var definitions = new HashMap<Identifier, List<TagLoader.EntryWithSource>>();
        try (var files = Files.list(dir)) {
            for (var file : (Iterable<Path>) files::iterator) {
                var name = file.getFileName().toString().replace(".json", "");
                var parsed = TagFile.CODEC.parse(JsonOps.INSTANCE, JsonParser.parseString(Files.readString(file))).getOrThrow();
                var entries = new ArrayList<TagLoader.EntryWithSource>();
                for (var entry : parsed.entries()) entries.add(new TagLoader.EntryWithSource(entry, "vanilla"));
                definitions.put(Identifier.parse("minecraft:" + name), entries);
            }
        }
        var tags = new HashMap<TagKey<Fluid>, List<Holder<Fluid>>>();
        loader.build(definitions).forEach((id, values) -> tags.put(TagKey.create(Registries.FLUID, id), List.copyOf(values)));
        registry.prepareTagReload(new TagLoader.LoadResult<>(Registries.FLUID, tags)).apply();
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        Path resources = Path.of(args[0]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(resources.resolve("block-tags.json"))).getAsJsonObject());
        bindFluidTags(resources.resolve("data/minecraft/tags/fluid"));
        var registryAccess = RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY);
        var air = net.minecraft.world.level.block.Blocks.AIR.defaultBlockState();
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[2]))); var lines = Files.lines(Path.of(args[1]))) {
            for (var line : (Iterable<String>) lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject();
                var cells = new HashMap<BlockPos, BlockState>();
                for (var cell : f.getAsJsonArray("cells")) {
                    var c = cell.getAsJsonArray();
                    cells.put(new BlockPos(c.get(0).getAsInt(), c.get(1).getAsInt(), c.get(2).getAsInt()), Block.stateById(c.get(3).getAsInt()));
                }
                int light = f.get("light").getAsInt();
                var noTicks = (LevelTickAccess<?>) Proxy.newProxyInstance(LevelTickAccess.class.getClassLoader(), new Class<?>[]{LevelTickAccess.class}, (proxy, method, values) -> {
                    if (method.getReturnType() == boolean.class) return false;
                    if (method.getReturnType() == int.class) return 0;
                    return null;
                });
                var level = (LevelReader) Proxy.newProxyInstance(LevelReader.class.getClassLoader(), new Class<?>[]{LevelReader.class}, (proxy, method, values) -> {
                    switch (method.getName()) {
                        case "getBlockState": return cells.getOrDefault((BlockPos) values[0], air);
                        case "getFluidState": return cells.getOrDefault((BlockPos) values[0], air).getFluidState();
                        case "getBlockEntity": return null;
                        case "getRawBrightness": return Math.max(0, light - (int) values[1]);
                        case "getMaxLocalRawBrightness": return light;
                        case "getBrightness": return light;
                        case "isClientSide": return false;
                        case "registryAccess": return registryAccess;
                        case "enabledFeatures": return net.minecraft.world.flag.FeatureFlags.DEFAULT_FLAGS;
                        case "environmentAttributes": return EnvironmentAttributeReader.EMPTY;
                        case "getMinY": return -64;
                        case "getHeight": return values == null || values.length == 0 ? 384 : null;
                        case "getSkyDarken": return 0;
                        case "hashCode": return System.identityHashCode(proxy);
                        case "equals": return proxy == values[0];
                        default:
                            if (method.isDefault()) return InvocationHandler.invokeDefault(proxy, method, values);
                            throw new AssertionError(method.toString());
                    }
                });
                var scheduled = new TreeSet<String>();
                var ticks = (ScheduledTickAccess) Proxy.newProxyInstance(ScheduledTickAccess.class.getClassLoader(), new Class<?>[]{ScheduledTickAccess.class}, (proxy, method, values) -> {
                    switch (method.getName()) {
                        case "scheduleTick": {
                            var pos = (BlockPos) values[0];
                            String type = values[1] instanceof Block block ? BuiltInRegistries.BLOCK.getKey(block).toString() : BuiltInRegistries.FLUID.getKey((Fluid) values[1]).toString();
                            int priority = values.length > 3 ? ((TickPriority) values[3]).getValue() : 0;
                            scheduled.add(type + "@" + pos.getX() + "," + pos.getY() + "," + pos.getZ() + "+" + values[2] + "p" + priority);
                            return null;
                        }
                        case "getBlockTicks":
                        case "getFluidTicks":
                            return noTicks;
                        default:
                            throw new AssertionError(method.toString());
                    }
                });
                RandomSource random = new LegacyRandomSource(f.get("seed").getAsLong());
                var p = f.getAsJsonArray("pos");
                var pos = new BlockPos(p.get(0).getAsInt(), p.get(1).getAsInt(), p.get(2).getAsInt());
                var state = cells.getOrDefault(pos, air);
                var direction = Direction.from3DDataValue(f.get("dir").getAsInt());
                var neighbourPos = pos.relative(direction);
                String shape;
                try {
                    var result = state.updateShape(level, ticks, pos, direction, neighbourPos, level.getBlockState(neighbourPos), random);
                    shape = Integer.toString(Block.getId(result));
                } catch (RuntimeException | AssertionError e) {
                    shape = "error " + e.getClass().getSimpleName();
                }
                String survive;
                try {
                    survive = Boolean.toString(state.canSurvive(level, pos));
                } catch (RuntimeException | AssertionError e) {
                    survive = "error";
                }
                out.println("shape " + shape + " ticks " + String.join(" ", scheduled) + " survive " + survive);
            }
        }
    }
}
