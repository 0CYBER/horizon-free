package dev.horizonlab.parity;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonElement;
import com.google.gson.JsonParser;
import com.mojang.serialization.Dynamic;
import com.mojang.serialization.JsonOps;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.stream.Stream;
import net.minecraft.SharedConstants;
import net.minecraft.core.registries.Registries;
import net.minecraft.nbt.*;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.level.ChunkMap;
import net.minecraft.util.datafix.DataFixTypes;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.util.filefix.FileFix;
import net.minecraft.util.filefix.FileFixerUpper;
import net.minecraft.util.worldupdate.UpgradeProgress;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.chunk.storage.RegionFile;
import net.minecraft.world.level.chunk.storage.RegionStorageInfo;
import net.minecraft.world.level.storage.LevelStorageSource;

/**
 * Opens a world saved by an older game version the way 26.3 does and writes out
 * what 26.3 reads: the file fixes (FileFixerUpper) on the folder, level.dat fixed
 * to the current DataVersion, then every chunk, entity chunk and POI chunk
 * (SimpleRegionStorage.upgradeChunkTag with ChunkMap's context), player file
 * (PlayerDataStorage.load), saved data file (SavedDataStorage.readTagFromDisk),
 * advancements and statistics through the real DataFixers.
 *
 *   VanillaDataFixOracle IN_WORLD OUT_WORLD
 *
 * Files the game rewrites after loading get the current DataVersion so that the
 * output compares with what the game would save. Nothing is answered by hand.
 *
 * OUT_WORLD is replaced. The oracle exits as soon as everything is written:
 * the level storage starts the game's IO worker pool, whose idle threads would
 * otherwise keep the JVM alive for a minute.
 */
public final class VanillaDataFixOracle {
    private static final Gson GSON = new GsonBuilder().setPrettyPrinting().create();

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Path in = Path.of(args[0]);
        Path out = Path.of(args[1]);
        delete(out);
        copy(in, out);
        if (args.length > 2) {
            band(out, Integer.parseInt(args[2]));
            System.exit(0);
        }
        int current = SharedConstants.getCurrentVersion().dataVersion().version();
        var fixer = DataFixers.getDataFixer();
        // FileFixerUpper.fix through a real level storage access, as the game opens a
        // world: the file fixes on a copy-on-write view swapped in, then level.dat
        // fixed to the current version and stamped (DataVersion, LastPlayed, Version).
        LevelStorageSource source = LevelStorageSource.createDefault(out.getParent());
        try (LevelStorageSource.LevelStorageAccess access = source.createAccess(out.getFileName().toString())) {
            Dynamic<?> levelData = access.getUnfixedDataTag(false);
            Dynamic<?> fixed = DataFixers.getFileFixer().fix(access, levelData, new UpgradeProgress());
            access.saveLevelData(fixed);
        }
        Files.deleteIfExists(out.resolve("level.dat_old"));
        Map<String, String> generators = generators(out);
        Path dimensions = out.resolve("dimensions");
        if (Files.isDirectory(dimensions)) {
            List<Path> dims;
            try (Stream<Path> s = Files.walk(dimensions, 2)) {
                dims = s.filter(p -> dimensions.relativize(p).getNameCount() == 2).sorted().toList();
            }
            for (Path dim : dims) {
                String id = dim.getParent().getFileName() + ":" + dim.getFileName();
                ResourceKey<Level> key = ResourceKey.create(Registries.DIMENSION, Identifier.parse(id));
                String generator = generators.get(id);
                CompoundTag context = ChunkMap.getChunkDataFixContextTag(
                    key, Optional.ofNullable(generator).map(Identifier::parse));
                upgradeRegions(dim.resolve("region"), key, "chunk", DataFixTypes.CHUNK, -1, context, current);
                upgradeRegions(dim.resolve("entities"), key, "entities", DataFixTypes.ENTITY_CHUNK, -1, null, current);
                upgradeRegions(dim.resolve("poi"), key, "poi", DataFixTypes.POI_CHUNK, 1945, null, current);
                upgradeSavedData(dim.resolve("data"), current);
            }
        }
        upgradeSavedData(out.resolve("data"), current);
        for (Path file : list(out.resolve("players/data"), ".dat")) {
            CompoundTag tag = NbtIo.readCompressed(file, NbtAccounter.unlimitedHeap());
            tag = DataFixTypes.PLAYER.updateToCurrentVersion(fixer, tag, NbtUtils.getDataVersion(tag));
            NbtIo.writeCompressed(NbtUtils.addDataVersion(tag, current), file);
        }
        for (Path file : list(out.resolve("players/advancements"), ".json")) {
            // PlayerAdvancements: DataFixTypes.ADVANCEMENTS.wrapCodec, default version 1343.
            JsonElement json = JsonParser.parseString(Files.readString(file, StandardCharsets.UTF_8));
            var dynamic = new Dynamic<>(JsonOps.INSTANCE, json);
            int from = dynamic.get("DataVersion").asInt(1343);
            var fixed = DataFixTypes.ADVANCEMENTS.updateToCurrentVersion(fixer, dynamic.remove("DataVersion"), from);
            fixed = fixed.set("DataVersion", fixed.createInt(current));
            Files.writeString(file, GSON.toJson(fixed.getValue()), StandardCharsets.UTF_8);
        }
        for (Path file : list(out.resolve("players/stats"), ".json")) {
            // ServerStatsCounter.parse: the version field stays in the data.
            JsonElement json = JsonParser.parseString(Files.readString(file, StandardCharsets.UTF_8));
            var dynamic = new Dynamic<>(JsonOps.INSTANCE, json);
            var fixed = DataFixTypes.STATS.updateToCurrentVersion(fixer, dynamic, NbtUtils.getDataVersion(dynamic, 1343));
            fixed = fixed.set("DataVersion", fixed.createInt(current));
            Files.writeString(file, GSON.toJson(fixed.getValue()), StandardCharsets.UTF_8);
        }
        System.exit(0);
    }

    /**
     * The data fixes up to a target DataVersion on a world in the layout of before
     * 1.16, without the file fixes: lets the fixes of a range of versions be checked
     * on their own, whatever the later fixes do to the same data.
     */
    private static void band(Path out, int target) throws Exception {
        var fixer = DataFixers.getDataFixer();
        // The file fixes of the range (1493, LegacyStructureFileFix), as FileFixerUpper runs them
        // (in place; the copy-on-write view and the level.dat rewrite are not what is checked).
        var fileFixes = FileFixerUpper.class.getDeclaredField("fileFixes");
        fileFixes.setAccessible(true);
        for (Object o : (List<?>) fileFixes.get(DataFixers.getFileFixer())) {
            FileFix fileFix = (FileFix) o;
            if (fileFix.getVersion() <= target) {
                fileFix.runFixOperations(out, new UpgradeProgress());
            }
        }
        Path levelFile = out.resolve("level.dat");
        if (Files.exists(levelFile)) {
            CompoundTag root = NbtIo.readCompressed(levelFile, NbtAccounter.unlimitedHeap());
            CompoundTag data = root.getCompoundOrEmpty("Data");
            int from = NbtUtils.getDataVersion(data, -1);
            data = DataFixTypes.LEVEL.update(fixer, data, from, target);
            root.put("Data", NbtUtils.addDataVersion(data, target));
            NbtIo.writeCompressed(root, levelFile);
        }
        String[][] dims = {{"", "minecraft:overworld"}, {"DIM-1", "minecraft:the_nether"}, {"DIM1", "minecraft:the_end"}};
        for (String[] d : dims) {
            Path dim = d[0].isEmpty() ? out : out.resolve(d[0]);
            ResourceKey<Level> key = ResourceKey.create(Registries.DIMENSION, Identifier.parse(d[1]));
            CompoundTag context = ChunkMap.getChunkDataFixContextTag(key, Optional.empty());
            upgradeRegions(dim.resolve("region"), key, "chunk", DataFixTypes.CHUNK, -1, context, target);
            upgradeRegions(dim.resolve("entities"), key, "entities", DataFixTypes.ENTITY_CHUNK, -1, null, target);
            upgradeRegions(dim.resolve("poi"), key, "poi", DataFixTypes.POI_CHUNK, 1945, null, target);
        }
        for (Path file : list(out.resolve("playerdata"), ".dat")) {
            CompoundTag tag = NbtIo.readCompressed(file, NbtAccounter.unlimitedHeap());
            tag = DataFixTypes.PLAYER.update(fixer, tag, NbtUtils.getDataVersion(tag), target);
            NbtIo.writeCompressed(NbtUtils.addDataVersion(tag, target), file);
        }
        upgradeJson(out.resolve("advancements"), DataFixTypes.ADVANCEMENTS, true, target);
        upgradeJson(out.resolve("stats"), DataFixTypes.STATS, false, target);
        // The saved data of the old layout (data/scoreboard.dat, data/map_N.dat).
        for (Path file : list(out.resolve("data"), ".dat")) {
            String name = file.getFileName().toString().replace(".dat", "");
            DataFixTypes type = name.equals("scoreboard") ? DataFixTypes.SAVED_DATA_SCOREBOARD
                : name.matches("map_\\d+") ? DataFixTypes.SAVED_DATA_MAP_DATA : null;
            if (type != null) {
                CompoundTag tag = NbtIo.readCompressed(file, NbtAccounter.unlimitedHeap());
                tag = type.update(fixer, tag, NbtUtils.getDataVersion(tag, 1343), target);
                NbtIo.writeCompressed(NbtUtils.addDataVersion(tag, target), file);
            }
        }
    }

    private static void upgradeJson(Path folder, DataFixTypes type, boolean removeVersion, int target) throws Exception {
        var fixer = DataFixers.getDataFixer();
        for (Path file : list(folder, ".json")) {
            JsonElement json = JsonParser.parseString(Files.readString(file, StandardCharsets.UTF_8));
            var dynamic = new Dynamic<>(JsonOps.INSTANCE, json);
            int from = NbtUtils.getDataVersion(dynamic, 1343);
            var input = removeVersion ? dynamic.remove("DataVersion") : dynamic;
            var fixed = type.update(fixer, input, from, target);
            fixed = fixed.set("DataVersion", fixed.createInt(target));
            Files.writeString(file, GSON.toJson(fixed.getValue()), StandardCharsets.UTF_8);
        }
    }

    /** Chunk generator type per dimension, from the saved world generation settings. */
    private static Map<String, String> generators(Path world) throws Exception {
        Map<String, String> out = new HashMap<>();
        Path file = world.resolve("data/minecraft/world_gen_settings.dat");
        if (!Files.exists(file)) {
            return out;
        }
        CompoundTag tag = NbtIo.readCompressed(file, NbtAccounter.unlimitedHeap());
        CompoundTag dims = tag.getCompoundOrEmpty("data").getCompoundOrEmpty("dimensions");
        for (String dim : dims.keySet()) {
            dims.getCompoundOrEmpty(dim).getCompoundOrEmpty("generator").getString("type").ifPresent(t -> out.put(dim, t));
        }
        return out;
    }

    private static void upgradeRegions(Path folder, ResourceKey<Level> dimension, String type, DataFixTypes fixType,
                                       int defaultVersion, CompoundTag context, int current) throws Exception {
        if (!Files.isDirectory(folder)) {
            return;
        }
        var fixer = DataFixers.getDataFixer();
        for (Path file : list(folder, ".mca")) {
            String[] parts = file.getFileName().toString().split("\\.");
            int rx = Integer.parseInt(parts[1]);
            int rz = Integer.parseInt(parts[2]);
            RegionStorageInfo info = new RegionStorageInfo("oracle", dimension, type);
            Map<ChunkPos, CompoundTag> chunks = new LinkedHashMap<>();
            try (RegionFile region = new RegionFile(info, file, folder, false)) {
                for (int z = 0; z < 32; z++) {
                    for (int x = 0; x < 32; x++) {
                        ChunkPos pos = new ChunkPos(rx * 32 + x, rz * 32 + z);
                        if (!region.hasChunk(pos)) {
                            continue;
                        }
                        try (DataInputStream in = region.getChunkDataInputStream(pos)) {
                            if (in != null) {
                                chunks.put(pos, NbtIo.read(in));
                            }
                        }
                    }
                }
            }
            Files.delete(file);
            try (RegionFile region = new RegionFile(info, file, folder, false)) {
                for (var entry : chunks.entrySet()) {
                    CompoundTag tag = entry.getValue();
                    int version = NbtUtils.getDataVersion(tag, defaultVersion);
                    if (version < current) {
                        // SimpleRegionStorage.upgradeChunkTag
                        if (context != null) {
                            tag.put("__context", context.copy());
                        }
                        tag = fixType.update(fixer, tag, version, current);
                        tag.remove("__context");
                        NbtUtils.addDataVersion(tag, current);
                    }
                    try (DataOutputStream o = region.getChunkDataOutputStream(entry.getKey())) {
                        NbtIo.write(tag, o);
                    }
                }
            }
        }
    }

    /** SavedDataStorage.readTagFromDisk for every known saved data file under a data folder. */
    private static void upgradeSavedData(Path data, int current) throws Exception {
        if (!Files.isDirectory(data)) {
            return;
        }
        List<Path> files;
        try (Stream<Path> s = Files.walk(data)) {
            files = s.filter(p -> p.toString().endsWith(".dat")).sorted().toList();
        }
        var fixer = DataFixers.getDataFixer();
        for (Path file : files) {
            String rel = data.relativize(file).toString().replace('\\', '/');
            DataFixTypes type = savedDataType(rel);
            if (type == null) {
                continue;
            }
            // SavedDataStorage.readTagFromDisk: a file that does not start with the gzip header is plain NBT.
            byte[] bytes = Files.readAllBytes(file);
            boolean gzip = bytes.length >= 2 && (bytes[0] & 255) == 0x1f && (bytes[1] & 255) == 0x8b;
            CompoundTag tag = gzip ? NbtIo.readCompressed(file, NbtAccounter.unlimitedHeap())
                : NbtIo.read(new java.io.DataInputStream(new java.io.ByteArrayInputStream(bytes)));
            int version = NbtUtils.getDataVersion(tag, 1343);
            tag = type.update(fixer, tag, version, current);
            NbtIo.writeCompressed(NbtUtils.addDataVersion(tag, current), file);
        }
    }

    /** The SavedDataType registered for a file below data/ (namespace/path.dat). */
    static DataFixTypes savedDataType(String rel) {
        String name = rel.endsWith(".dat") ? rel.substring(0, rel.length() - 4) : rel;
        if (name.endsWith("/command_storage")) {
            return DataFixTypes.SAVED_DATA_COMMAND_STORAGE;
        }
        if (name.matches("minecraft/maps/\\d+")) {
            return DataFixTypes.SAVED_DATA_MAP_DATA;
        }
        return switch (name) {
            case "minecraft/custom_boss_events" -> DataFixTypes.SAVED_DATA_CUSTOM_BOSS_EVENTS;
            case "minecraft/scoreboard" -> DataFixTypes.SAVED_DATA_SCOREBOARD;
            case "minecraft/chunk_tickets" -> DataFixTypes.SAVED_DATA_FORCED_CHUNKS;
            case "minecraft/world_border" -> DataFixTypes.SAVED_DATA_WORLD_BORDER;
            case "minecraft/game_rules" -> DataFixTypes.SAVED_DATA_GAME_RULES;
            case "minecraft/ender_dragon_fight" -> DataFixTypes.SAVED_DATA_ENDER_DRAGON_FIGHT;
            case "minecraft/wandering_trader" -> DataFixTypes.SAVED_DATA_WANDERING_TRADER;
            case "minecraft/maps/last_id" -> DataFixTypes.SAVED_DATA_MAP_INDEX;
            case "minecraft/weather" -> DataFixTypes.SAVED_DATA_WEATHER;
            case "minecraft/scheduled_events" -> DataFixTypes.SAVED_DATA_SCHEDULED_EVENTS;
            case "minecraft/world_gen_settings" -> DataFixTypes.SAVED_DATA_WORLD_GEN_SETTINGS;
            case "minecraft/random_sequences" -> DataFixTypes.SAVED_DATA_RANDOM_SEQUENCES;
            case "minecraft/stopwatches" -> DataFixTypes.SAVED_DATA_STOPWATCHES;
            case "minecraft/raids" -> DataFixTypes.SAVED_DATA_RAIDS;
            case "minecraft/world_clocks" -> DataFixTypes.SAVED_DATA_WORLD_CLOCKS;
            default -> null;
        };
    }

    private static List<Path> list(Path folder, String suffix) throws Exception {
        if (!Files.isDirectory(folder)) {
            return List.of();
        }
        try (Stream<Path> s = Files.list(folder)) {
            return s.filter(p -> p.getFileName().toString().endsWith(suffix)).sorted().toList();
        }
    }

    private static void delete(Path path) throws Exception {
        if (!Files.exists(path)) {
            return;
        }
        try (Stream<Path> s = Files.walk(path)) {
            for (Path p : s.sorted(Comparator.reverseOrder()).toList()) {
                Files.delete(p);
            }
        }
    }

    private static void copy(Path from, Path to) throws Exception {
        try (Stream<Path> s = Files.walk(from)) {
            for (Path p : s.sorted().toList()) {
                Path target = to.resolve(from.relativize(p).toString());
                if (Files.isDirectory(p)) {
                    Files.createDirectories(target);
                } else if (!p.getFileName().toString().equals("session.lock")) {
                    Files.copy(p, target, StandardCopyOption.REPLACE_EXISTING);
                }
            }
        }
    }
}
