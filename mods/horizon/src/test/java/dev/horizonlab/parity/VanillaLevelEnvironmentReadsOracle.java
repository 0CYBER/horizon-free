package dev.horizonlab.parity;

import com.google.gson.*;
import java.lang.reflect.Proxy;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.resources.*;
import net.minecraft.tags.TagKey;
import net.minecraft.world.attribute.*;
import net.minecraft.world.clock.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.dimension.*;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.presets.*;
import net.minecraft.world.level.material.FluidState;

/** Original fixtures calling the pinned biome, floor, dimension and sky-light queries. */
public final class VanillaLevelEnvironmentReadsOracle {
    private static final String[] BIOMES = {"minecraft:plains", "minecraft:desert", "minecraft:forest"};
    /** Decode the fixture's three signed block coordinates. */
    private static int[] position(JsonArray values) {
        return new int[]{values.get(0).getAsInt(), values.get(1).getAsInt(), values.get(2).getAsInt()};
    }
    /** Encode a matched block position as three JSON integers. */
    private static JsonArray coordinates(int x, int y, int z) {
        var array = new JsonArray(); array.add(x); array.add(y); array.add(z); return array;
    }
    /** Create the fixture dimension key used to select a world-preset stem. */
    private static ResourceKey<LevelStem> dimensionKey(String name) {
        return ResourceKey.create(Registries.LEVEL_STEM, Identifier.parse(name));
    }
    /** Read the fixture palette through vanilla chunk storage and fuzzy biome zoom. */
    private static JsonElement biome(JsonObject input, DimensionType dimension) {
        var lookup = ParityData.access.lookupOrThrow(Registries.BIOME);
        var chunks = new HashMap<ChunkPos, ProtoChunk>();
        var factory = PalettedContainerFactory.create(ParityData.access);
        for (var column : input.getAsJsonArray("loaded")) {
            var pair = column.getAsJsonArray();
            var pos = new ChunkPos(pair.get(0).getAsInt(), pair.get(1).getAsInt());
            var chunk = new ProtoChunk(pos, UpgradeData.EMPTY,
                LevelHeightAccessor.create(dimension.minY(), dimension.height()), factory, null);
            chunk.fillBiomesFromNoise((x, y, z) -> lookup.getOrThrow(ResourceKey.create(
                Registries.BIOME, Identifier.parse(BIOMES[Math.floorMod(x * 31 + y * 7 + z * 17, BIOMES.length)]))));
            chunk.setPersistedStatus(net.minecraft.world.level.chunk.status.ChunkStatus.BIOMES);
            chunks.put(pos, chunk);
        }
        var manager = new BiomeManager((x, y, z) -> {
            var chunk = chunks.get(new ChunkPos(x >> 2, z >> 2));
            return chunk == null ? null : chunk.getNoiseBiome(x, y, z);
        }, input.get("zoom_seed").getAsLong());
        int[] p = position(input.getAsJsonArray("position"));
        var result = manager.getBiome(new BlockPos(p[0], p[1], p[2]));
        return result == null ? JsonNull.INSTANCE : new JsonPrimitive(result.unwrapKey().orElseThrow().identifier().toString());
    }
    /** Invoke the collision-floor query on the fixture's two block states. */
    private static JsonElement floor(JsonObject input) {
        BlockState here = BuiltInRegistries.BLOCK.getValue(Identifier.parse(input.get("here").getAsString())).defaultBlockState();
        BlockState below = BuiltInRegistries.BLOCK.getValue(Identifier.parse(input.get("below").getAsString())).defaultBlockState();
        BlockGetter blocks = new BlockGetter() {
            public BlockEntity getBlockEntity(BlockPos pos) { return null; }
            public BlockState getBlockState(BlockPos pos) {
                if (pos.equals(BlockPos.ZERO)) return here;
                return pos.equals(BlockPos.ZERO.below()) ? below : Blocks.AIR.defaultBlockState();
            }
            public FluidState getFluidState(BlockPos pos) { return getBlockState(pos).getFluidState(); }
            public int getHeight() { return 384; }
            public int getMinY() { return -64; }
        };
        double result = blocks.getBlockFloorHeight(BlockPos.ZERO);
        return Double.isInfinite(result) ? new JsonPrimitive("-Infinity") : new JsonPrimitive(result);
    }
    @SuppressWarnings("deprecation")
    /** Build dimension/weather attributes and invoke the real Level update/get pair. */
    private static JsonElement sky(JsonObject input, DimensionType dimension) {
        long time = input.get("time").getAsLong();
        float rain = input.get("rain").getAsFloat(), thunder = input.get("thunder").getAsFloat();
        ClockManager clocks = holder -> new ClockInstance() {
            public long totalTicks() { return time; }
            public float partialTick() { return 0.0F; }
            public float rate() { return 1.0F; }
            public boolean isPaused() { return false; }
        };
        var builder = EnvironmentAttributeSystem.builder();
        builder.addConstantLayer(dimension.attributes());
        dimension.timelines().forEach(t -> builder.addTimelineLayer(t, clocks));
        if (dimension.hasSkyLight() && !dimension.hasCeiling()) {
            WeatherAttributes.addBuiltinLayers(builder, new WeatherAttributes.WeatherAccess() {
                public float rainLevel() { return rain; }
                public float thunderLevel() { return thunder * rain; }
            });
        }
        // Vanilla's test setter supplies the real Level update/get pair's environment.
        ParityData.level.setEnvironmentAttributes(builder.build());
        ParityData.level.updateSkyBrightness();
        return new JsonPrimitive(ParityData.level.getSkyDarken());
    }
    /** Search the preset's real biome source with the requested selector and sampling bounds. */
    private static JsonElement search(JsonObject input, LevelStem stem) {
        var generator = (NoiseBasedChunkGenerator) stem.generator();
        var settings = generator.generatorSettings().value();
        var random = RandomState.create(ParityData.access.lookupOrThrow(Registries.NOISE), input.get("seed").getAsLong(), settings);
        DimensionType dimension = stem.type().value();
        LevelReader bounds = (LevelReader) Proxy.newProxyInstance(LevelReader.class.getClassLoader(), new Class<?>[]{LevelReader.class}, (proxy, method, args) -> {
            return switch (method.getName()) {
                case "getMinY" -> dimension.minY();
                case "getHeight" -> dimension.height();
                case "getMaxY" -> dimension.minY() + dimension.height() - 1;
                default -> throw new AssertionError("unexpected world read: " + method.getName());
            };
        });
        String selector = input.get("selector").getAsString();
        java.util.function.Predicate<Holder<Biome>> allowed = selector.startsWith("#")
            ? holder -> holder.is(TagKey.create(Registries.BIOME, Identifier.parse(selector.substring(1))))
            : holder -> holder.unwrapKey().orElseThrow().identifier().toString().equals(selector);
        int[] p = position(input.getAsJsonArray("position"));
        var result = generator.getBiomeSource().findClosestBiome3d(new BlockPos(p[0],p[1],p[2]), input.get("radius").getAsInt(), input.get("horizontal").getAsInt(), input.get("vertical").getAsInt(), allowed, random, bounds);
        if (result == null) return JsonNull.INSTANCE;
        var output = new JsonObject();
        output.addProperty("biome", result.getSecond().unwrapKey().orElseThrow().identifier().toString());
        BlockPos pos = result.getFirst(); output.add("position", coordinates(pos.getX(), pos.getY(), pos.getZ()));
        return output;
    }
    /** Emit one vanilla outcome per JSONL input with the loaded registry data. */
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var preset = ParityData.access.lookupOrThrow(Registries.WORLD_PRESET).getOrThrow(WorldPresets.NORMAL).value();
        var dimensions = preset.createWorldDimensions().dimensions();
        try (var output = Files.newBufferedWriter(Path.of(args[1]))) {
            for (String line : Files.readAllLines(Path.of(args[0]))) {
                if (line.isBlank()) continue;
                var input = JsonParser.parseString(line).getAsJsonObject();
                var row = new JsonObject(); row.add("name", input.get("name"));
                String kind = input.get("kind").getAsString();
                LevelStem stem = dimensions.get(dimensionKey(input.has("dimension")
                    ? input.get("dimension").getAsString() : "minecraft:overworld"));
                row.add("value", switch (kind) {
                    case "biome" -> biome(input, stem.type().value());
                    case "floor" -> floor(input);
                    case "sky" -> sky(input, stem.type().value());
                    case "search" -> search(input, stem);
                    case "logical_height" -> new JsonPrimitive(stem.type().value().logicalHeight());
                    default -> throw new IllegalArgumentException(kind);
                });
                output.write(row.toString()); output.newLine();
            }
        }
    }
}
