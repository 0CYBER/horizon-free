package dev.horizonlab.parity;

import it.unimi.dsi.fastutil.longs.LongOpenHashSet;
import it.unimi.dsi.fastutil.longs.LongSet;
import java.io.PrintWriter;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.function.Consumer;
import java.util.function.Predicate;
import net.minecraft.SharedConstants;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.level.ChunkMap;
import net.minecraft.server.level.ServerChunkCache;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.TagLoader;
import net.minecraft.util.RandomSource;
import net.minecraft.world.Difficulty;
import net.minecraft.world.DifficultyInstance;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.BiomeManager;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.border.WorldBorder;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.world.level.chunk.LevelChunk;
import net.minecraft.world.level.dimension.DimensionType;
import net.minecraft.world.level.entity.EntityTypeTest;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.levelgen.FlatLevelSource;
import net.minecraft.util.Mth;
import net.minecraft.world.level.levelgen.structure.BoundingBox;
import net.minecraft.world.level.levelgen.structure.Structure;
import net.minecraft.world.level.levelgen.structure.StructurePiece;
import net.minecraft.world.level.levelgen.structure.StructureStart;
import net.minecraft.world.level.levelgen.structure.pieces.PiecesContainer;
import net.minecraft.world.level.levelgen.structure.pieces.StructurePieceSerializationContext;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.storage.LevelData;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

/**
 * Original oracle: natural spawning as NaturalSpawner decides it, on a
 * stand-in ServerLevel built from boxes of blocks, light and biomes, with
 * structure starts (pieces), existing entities, players, the world spawn
 * point and unloaded chunks. Three modes:
 *
 * - chunk: createState over the entities, getFilteredSpawningCategories, then
 *   spawnForChunk's loop (canSpawnForCategoryLocal, getRandomPosWithin) and
 *   vanilla's spawnCategoryForPosition with the state's canSpawn and
 *   afterSpawn;
 * - pos: spawnCategoryForPosition from a given start;
 * - gen: spawnMobsForChunkGeneration.
 *
 * The log records each attempt (the nearest player lookup), each type that
 * passed isValidSpawnPostitionForType with the potential test's answer, each
 * spawn (position and yaw bits), and the next draw of the level random, which
 * pins every draw in between. finalizeSpawn reads level.getRandom(), which
 * here is a fixed "neutral" source (nextInt 0, nextFloat and nextDouble 0.75),
 * so finalizeSpawn's own outcomes stay out of scope while its effect on later
 * attempts (the finalized mob's box obstructs them) is deterministic.
 */
public final class VanillaNaturalSpawnOracle {
    static Object unsafe;
    static Method allocate, putObject, objectFieldOffset;

    static Object alloc(Class<?> c) throws Exception { return allocate.invoke(unsafe, c); }

    static void put(Object target, Class<?> owner, String field, Object value) throws Exception {
        Field f = owner.getDeclaredField(field);
        putObject.invoke(unsafe, target, (long) objectFieldOffset.invoke(unsafe, f), value);
    }

    record Box(int x0, int y0, int z0, int x1, int y1, int z1) {
        boolean has(int x, int y, int z) { return x >= x0 && x <= x1 && y >= y0 && y <= y1 && z >= z0 && z <= z1; }
        static Box parse(String s) {
            String[] p = s.split(",");
            return new Box(Integer.parseInt(p[0]), Integer.parseInt(p[1]), Integer.parseInt(p[2]),
                Integer.parseInt(p[3]), Integer.parseInt(p[4]), Integer.parseInt(p[5]));
        }
    }
    record Start(Structure structure, StructureStart start) {}

    /** finalizeSpawn's random: the same answer every time. */
    static final class Neutral implements RandomSource {
        @Override public RandomSource fork() { return this; }
        @Override public net.minecraft.world.level.levelgen.PositionalRandomFactory forkPositional() { throw new UnsupportedOperationException(); }
        @Override public void setSeed(long seed) {}
        @Override public int nextInt() { return 0; }
        @Override public int nextInt(int bound) { return 0; }
        @Override public long nextLong() { return 0L; }
        @Override public boolean nextBoolean() { return false; }
        @Override public float nextFloat() { return 0.75F; }
        @Override public double nextDouble() { return 0.75; }
        @Override public double nextGaussian() { return 0.0; }
    }

    static final class Lvl extends ServerLevel {
        Lvl() { super(null, null, null, null, null, null, false, 0L, List.of(), false); }
        RegistryAccess registries;
        net.minecraft.server.MinecraftServer fake;
        @Override public net.minecraft.server.MinecraftServer getServer() { return fake; }
        List<Map.Entry<Box, BlockState>> blocks = new ArrayList<>();
        List<Map.Entry<Box, int[]>> lights = new ArrayList<>();
        List<Map.Entry<Box, Holder<Biome>>> biomes = new ArrayList<>();
        Holder<Biome> biome;
        int sky, block, darken, sea;
        boolean thunder;
        long seed;
        Difficulty difficulty;
        DimensionType dimensionType;
        ResourceKey<Level> dimensionKey;
        LevelData.RespawnData respawn;
        Set<Long> unloaded = new HashSet<>();
        List<Start> starts = new ArrayList<>();
        List<Entity> entities = new ArrayList<>();
        List<ServerPlayer> playerList = new ArrayList<>();
        net.minecraft.world.attribute.EnvironmentAttributeSystem attributes;
        BiomeManager biomeManager;
        WorldBorder border = new WorldBorder();
        GameRules rules;
        ServerChunkCache chunkSource;
        StructureManager structures;
        RandomSource neutral = new Neutral();
        List<String> log;
        boolean logColumns;
        int next = 1;
        @Override public RegistryAccess registryAccess() { return registries; }
        @Override public Difficulty getDifficulty() { return difficulty; }
        @Override public long getGameTime() { return 0L; }
        @Override public int getNextEntityId() { return next++; }
        @Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() { return net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags(); }
        @Override public BlockState getBlockState(BlockPos pos) {
            for (int i = blocks.size() - 1; i >= 0; i--)
                if (blocks.get(i).getKey().has(pos.getX(), pos.getY(), pos.getZ())) return blocks.get(i).getValue();
            return Blocks.AIR.defaultBlockState();
        }
        @Override public FluidState getFluidState(BlockPos pos) { return getBlockState(pos).getFluidState(); }
        int[] light(BlockPos pos) {
            for (int i = lights.size() - 1; i >= 0; i--)
                if (lights.get(i).getKey().has(pos.getX(), pos.getY(), pos.getZ())) return lights.get(i).getValue();
            return new int[] {sky, block};
        }
        @Override public int getBrightness(LightLayer layer, BlockPos pos) { int[] l = light(pos); return layer == LightLayer.SKY ? l[0] : l[1]; }
        @Override public int getRawBrightness(BlockPos pos, int darkening) { int[] l = light(pos); return Math.max(l[1], l[0] - darkening); }
        @Override public int getSkyDarken() { return darken; }
        @Override public boolean isThundering() { return thunder; }
        @Override public int getSeaLevel() { return sea; }
        @Override public int getHeight(Heightmap.Types type, int x, int z) {
            if (logColumns) log.add("col " + x + " " + z);
            for (int y = getMaxY(); y >= getMinY(); y--)
                if (type.isOpaque().test(getBlockState(new BlockPos(x, y, z)))) return y + 1;
            return getMinY();
        }
        @Override public long getSeed() { return seed; }
        @Override public DimensionType dimensionType() { return dimensionType; }
        @Override public ResourceKey<Level> dimension() { return dimensionKey; }
        Holder<Biome> biomeAt(int x, int y, int z) {
            for (int i = biomes.size() - 1; i >= 0; i--)
                if (biomes.get(i).getKey().has(x, y, z)) return biomes.get(i).getValue();
            return biome;
        }
        @Override public Holder<Biome> getBiome(BlockPos pos) { return biomeAt(pos.getX(), pos.getY(), pos.getZ()); }
        @Override public BiomeManager getBiomeManager() { return biomeManager; }
        @Override public net.minecraft.world.attribute.EnvironmentAttributeSystem environmentAttributes() { return attributes; }
        @Override public WorldBorder getWorldBorder() { return border; }
        @Override public GameRules getGameRules() { return rules; }
        @Override public RandomSource getRandom() { return neutral; }
        @Override public ServerChunkCache getChunkSource() { return chunkSource; }
        @Override public StructureManager structureManager() { return structures; }
        @Override public LevelData.RespawnData getRespawnData() { return respawn; }
        @Override public boolean canSpawnEntitiesInChunk(ChunkPos pos) { return !unloaded.contains(pos.pack()); }
        @Override public DifficultyInstance getCurrentDifficultyAt(BlockPos pos) { return new DifficultyInstance(difficulty, 0L, 0L, 0.0F); }
        @Override public BlockGetter getChunkForCollisions(int chunkX, int chunkZ) { return this; }
        @Override public boolean containsAnyLiquid(AABB box) {
            BlockPos lo = BlockPos.containing(box.minX, box.minY, box.minZ), hi = BlockPos.containing(box.maxX, box.maxY, box.maxZ);
            for (BlockPos p : BlockPos.betweenClosed(lo, hi)) if (!getBlockState(p).getFluidState().isEmpty()) return true;
            return false;
        }
        @SuppressWarnings({"unchecked", "rawtypes"})
        @Override public List players() { return playerList; }
        @Override public Iterable<Entity> getAllEntities() { return entities; }
        @Override public List<Entity> getEntities(Entity except, AABB bb, Predicate<? super Entity> selector) {
            List<Entity> out = new ArrayList<>();
            for (Entity e : entities) if (e != except && e.getBoundingBox().intersects(bb) && selector.test(e)) out.add(e);
            return out;
        }
        @Override public <T extends Entity> List<T> getEntities(EntityTypeTest<Entity, T> type, AABB bb, Predicate<? super T> selector) {
            List<T> out = new ArrayList<>();
            for (Entity e : entities) {
                T t = type.tryCast(e);
                if (t != null && e.getBoundingBox().intersects(bb) && selector.test(t)) out.add(t);
            }
            return out;
        }
        @Override public boolean addFreshEntity(Entity entity) { entities.add(entity); return true; }
        @Override public void addFreshEntityWithPassengers(Entity entity) { entity.getSelfAndPassengers().forEach(this::addFreshEntity); }
        @Override public Player getNearestPlayer(double x, double y, double z, double maxDist, boolean filter) {
            if (maxDist == -1.0 && !filter) log.add("try " + (int) Math.floor(x) + " " + (int) Math.floor(z));
            return super.getNearestPlayer(x, y, z, maxDist, filter);
        }
    }

    static final class Chunk extends LevelChunk {
        Chunk() { super((Level) null, (ChunkPos) null); }
        Lvl level;
        ChunkPos at;
        @Override public ChunkPos getPos() { return at; }
        @Override public BlockState getBlockState(BlockPos pos) { return level.getBlockState(pos); }
        @Override public FluidState getFluidState(BlockPos pos) { return level.getFluidState(pos); }
        @Override public int getHeight(Heightmap.Types type, int x, int z) { return level.getHeight(type, x, z); }
    }

    static final class Source extends ServerChunkCache {
        Source() { super(null, null, null, null, null, null, 0, 0, false, null); }
        ChunkGenerator generator;
        @Override public ChunkGenerator getGenerator() { return generator; }
    }

    /** getPlayersCloseForSpawning: none past the natural spawn chunk tracker's 8 chunks, else non-spectators within 128 blocks of the chunk centre. */
    static final class Map2 extends ChunkMap {
        Map2() { super(null, null, null, null, null, null, null, null, null, null, 0, false); }
        Lvl level;
        @Override public List<ServerPlayer> getPlayersCloseForSpawning(ChunkPos pos) {
            boolean tracked = false;
            for (ServerPlayer p : level.playerList) {
                int px = Mth.floor(p.getX()) >> 4, pz = Mth.floor(p.getZ()) >> 4;
                if (Math.max(Math.abs(px - pos.x()), Math.abs(pz - pos.z())) <= 8) tracked = true;
            }
            if (!tracked) return List.of();
            List<ServerPlayer> out = new ArrayList<>();
            for (ServerPlayer p : level.playerList) {
                double xd = pos.x() * 16 + 8 - p.getX(), zd = pos.z() * 16 + 8 - p.getZ();
                if (!p.isSpectator() && xd * xd + zd * zd < 16384.0) out.add(p);
            }
            return out;
        }
    }

    static final class Structures extends StructureManager {
        Structures() { super(null, null, null); }
        Lvl level;
        List<Start> referenced(int x, int z) {
            int cx = x >> 4, cz = z >> 4;
            List<Start> out = new ArrayList<>();
            for (Start s : level.starts) {
                BoundingBox b = s.start().getBoundingBox();
                if (b.intersects(cx * 16, cz * 16, cx * 16 + 15, cz * 16 + 15)) out.add(s);
            }
            return out;
        }
        @Override public Map<Structure, LongSet> getAllStructuresAt(int x, int y, int z) {
            Map<Structure, LongSet> out = new LinkedHashMap<>();
            for (Start s : referenced(x, z)) out.computeIfAbsent(s.structure(), k -> new LongOpenHashSet()).add(level.starts.indexOf(s));
            return out;
        }
        @Override public void fillStartsForStructure(Structure structure, LongSet references, Consumer<StructureStart> consumer) {
            for (long i : references) {
                StructureStart start = level.starts.get((int) i).start();
                if (start.isValid()) consumer.accept(start);
            }
        }
        @Override public List<StructureStart> startsForStructure(int sectionX, int sectionZ, Structure structure) {
            List<StructureStart> out = new ArrayList<>();
            for (Start s : referenced(sectionX << 4, sectionZ << 4)) if (s.structure() == structure) out.add(s.start());
            return out;
        }
        @Override public RegistryAccess registryAccess() { return level.registries; }
    }

    static final class Piece extends StructurePiece {
        Piece(BoundingBox box) { super(null, 0, box); }
        @Override protected void addAdditionalSaveData(StructurePieceSerializationContext context, CompoundTag tag) {}
        @Override public void postProcess(WorldGenLevel level, StructureManager structureManager, ChunkGenerator generator, RandomSource random,
            BoundingBox chunkBB, ChunkPos chunkPos, BlockPos referencePos) {}
    }

    static final class OraclePlayer extends ServerPlayer {
        OraclePlayer() { super(null, null, null, null); }
        GameType mode;
        @Override public boolean isSpectator() { return mode == GameType.SPECTATOR; }
        @Override public boolean isCreative() { return mode == GameType.CREATIVE; }
        @Override public GameType gameMode() { return mode; }
    }

    static String bits(double v) { return Long.toHexString(Double.doubleToRawLongBits(v)); }
    static String bits(float v) { return Integer.toHexString(Float.floatToRawIntBits(v)); }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        putObject = unsafe.getClass().getMethod("putObject", Object.class, long.class, Object.class);
        objectFieldOffset = unsafe.getClass().getMethod("objectFieldOffset", Field.class);
        var repo = ServerPacksSource.createVanillaTrustedRepository();
        repo.reload();
        repo.setSelected(List.of("vanilla"));
        var resources = new MultiPackResourceManager(PackType.SERVER_DATA, repo.openAllSelected());
        var layers = RegistryLayer.createRegistryAccess();
        var staticTags = TagLoader.loadTagsForExistingRegistries(resources, layers.getLayer(RegistryLayer.STATIC));
        var context = TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD), staticTags);
        RegistryAccess.Frozen world = RegistryDataLoader.load(resources, context, RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
        var worldLayers = layers.replaceFrom(RegistryLayer.WORLD, world);
        var loaded = net.minecraft.server.ReloadableServerRegistries.reload(worldLayers, staticTags, resources, Runnable::run).join();
        HolderLookup.Provider lookup = loaded.lookupWithUpdatedTags();
        var components = BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(lookup);
        staticTags.forEach(Registry.PendingTags::apply);
        components.forEach(net.minecraft.core.component.DataComponentInitializers.PendingComponents::apply);
        RegistryAccess access = loaded.layers().compositeAccess();
        var biomes = access.lookupOrThrow(Registries.BIOME);
        var dimensions = access.lookupOrThrow(Registries.DIMENSION_TYPE);
        var blockLookup = access.lookupOrThrow(Registries.BLOCK);
        var structureLookup = access.lookupOrThrow(Registries.STRUCTURE);

        // A server that only answers debug subscribers (path navigation asks).
        var server = (net.minecraft.server.MinecraftServer) alloc(net.minecraft.server.dedicated.DedicatedServer.class);
        put(server, net.minecraft.server.MinecraftServer.class, "debugSubscribers", alloc(net.minecraft.util.debug.ServerDebugSubscribers.class));
        Lvl level = (Lvl) alloc(Lvl.class);
        level.fake = server;
        put(level, ServerLevel.class, "server", server);
        level.registries = access;
        level.blocks = new ArrayList<>();
        level.lights = new ArrayList<>();
        level.biomes = new ArrayList<>();
        level.unloaded = new HashSet<>();
        level.starts = new ArrayList<>();
        level.entities = new ArrayList<>();
        level.playerList = new ArrayList<>();
        level.border = new WorldBorder();
        level.neutral = new Neutral();
        level.rules = new GameRules(net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags());
        level.biomeManager = new BiomeManager((x, y, z) -> level.biome, 0L) {
            @Override public Holder<Biome> getBiome(BlockPos pos) { return level.biomeAt(pos.getX(), pos.getY(), pos.getZ()); }
            @Override public Holder<Biome> getBiome(int x, int y, int z) { return level.biomeAt(x, y, z); }
        };
        Source source = (Source) alloc(Source.class);
        source.generator = (ChunkGenerator) alloc(FlatLevelSource.class);
        level.chunkSource = source;
        Structures structures = (Structures) alloc(Structures.class);
        structures.level = level;
        level.structures = structures;
        Map2 chunkMap = (Map2) alloc(Map2.class);
        chunkMap.level = level;

        Method createState = NaturalSpawner.class.getMethod("createState", int.class, ServerLevel.class, NaturalSpawner.ChunkGetter.class, LocalMobCapCalculator.class);
        Method canSpawn = NaturalSpawner.SpawnState.class.getDeclaredMethod("canSpawn", EntityType.class, Level.class, BlockPos.class, net.minecraft.world.level.chunk.ChunkAccess.class);
        canSpawn.setAccessible(true);
        Method afterSpawn = NaturalSpawner.SpawnState.class.getDeclaredMethod("afterSpawn", Mob.class, net.minecraft.world.level.chunk.ChunkAccess.class);
        afterSpawn.setAccessible(true);
        Method local = NaturalSpawner.SpawnState.class.getDeclaredMethod("canSpawnForCategoryLocal", MobCategory.class, ChunkPos.class);
        local.setAccessible(true);
        Method randomPos = NaturalSpawner.class.getDeclaredMethod("getRandomPosWithin", Level.class, LevelChunk.class);
        randomPos.setAccessible(true);
        Field entityId = Entity.class.getDeclaredField("id");
        entityId.setAccessible(true);

        List<String> lines = Files.readAllLines(Path.of(args[0]));
        try (var w = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            for (String raw : lines) {
                if (raw.isBlank()) continue;
                Map<String, String> c = new LinkedHashMap<>();
                for (String token : raw.trim().split("\\s+")) {
                    int eq = token.indexOf('=');
                    if (eq < 0) c.put("name", token); else c.put(token.substring(0, eq), token.substring(eq + 1));
                }
                List<String> log = new ArrayList<>();
                level.log = log;
                level.logColumns = false;
                try {
                    level.dimensionKey = ResourceKey.create(Registries.DIMENSION, Identifier.parse(c.getOrDefault("dim", "minecraft:overworld")));
                    level.dimensionType = dimensions.getOrThrow(ResourceKey.create(Registries.DIMENSION_TYPE, level.dimensionKey.identifier())).value();
                    level.blocks.clear();
                    for (int i = 0; c.containsKey("k" + i); i++) {
                        String[] p = c.get("k" + i).split("/", 2);
                        level.blocks.add(Map.entry(Box.parse(p[0]), net.minecraft.commands.arguments.blocks.BlockStateParser.parseForBlock(blockLookup, p[1], false).blockState()));
                    }
                    level.lights.clear();
                    for (int i = 0; c.containsKey("l" + i); i++) {
                        String[] p = c.get("l" + i).split("/", 2);
                        String[] v = p[1].split(",");
                        level.lights.add(Map.entry(Box.parse(p[0]), new int[] {Integer.parseInt(v[0]), Integer.parseInt(v[1])}));
                    }
                    level.biome = biomes.getOrThrow(ResourceKey.create(Registries.BIOME, Identifier.parse(c.getOrDefault("biome", "minecraft:plains"))));
                    level.biomes.clear();
                    for (int i = 0; c.containsKey("b" + i); i++) {
                        String[] p = c.get("b" + i).split("/", 2);
                        level.biomes.add(Map.entry(Box.parse(p[0]), biomes.getOrThrow(ResourceKey.create(Registries.BIOME, Identifier.parse(p[1])))));
                    }
                    level.sky = Integer.parseInt(c.getOrDefault("sky", "15"));
                    level.block = Integer.parseInt(c.getOrDefault("light", "0"));
                    level.darken = Integer.parseInt(c.getOrDefault("darken", "0"));
                    level.thunder = c.getOrDefault("thunder", "0").equals("1");
                    level.sea = Integer.parseInt(c.getOrDefault("sea", "63"));
                    level.seed = Long.parseLong(c.getOrDefault("wseed", "0"));
                    level.difficulty = Difficulty.byName(c.getOrDefault("difficulty", "normal"));
                    level.rules.set(net.minecraft.world.level.gamerules.GameRules.SPAWN_MOBS, !c.getOrDefault("rules", "1").equals("0"), null);
                    // The dimension and biome layers; the moon timeline's surface slime chance as a constant.
                    level.attributes = net.minecraft.world.attribute.EnvironmentAttributeSystem.builder().addStaticLayers(level)
                        .addConstantLayer(net.minecraft.world.attribute.EnvironmentAttributeMap.builder()
                            .set(net.minecraft.world.attribute.EnvironmentAttributes.SURFACE_SLIME_SPAWN_CHANCE, Float.parseFloat(c.getOrDefault("slime", "0.0"))).build())
                        .build();
                    level.unloaded.clear();
                    if (c.containsKey("unloaded")) for (String u : c.get("unloaded").split(";")) {
                        String[] p = u.split(",");
                        level.unloaded.add(ChunkPos.pack(Integer.parseInt(p[0]), Integer.parseInt(p[1])));
                    }
                    level.starts.clear();
                    for (int i = 0; c.containsKey("s" + i); i++) {
                        String[] p = c.get("s" + i).split("/", 2);
                        Structure structure = structureLookup.getValue(Identifier.parse(p[0]));
                        List<StructurePiece> pieces = new ArrayList<>();
                        for (String b : p[1].split(";")) {
                            Box bx = Box.parse(b);
                            pieces.add(new Piece(new BoundingBox(bx.x0(), bx.y0(), bx.z0(), bx.x1(), bx.y1(), bx.z1())));
                        }
                        level.starts.add(new Start(structure, new StructureStart(structure, new ChunkPos(0, 0), 0, new PiecesContainer(pieces))));
                    }
                    String[] r = c.getOrDefault("respawn", "0,-1000,0/minecraft:overworld").split("/");
                    String[] rp = r[0].split(",");
                    level.respawn = new LevelData.RespawnData(GlobalPos.of(ResourceKey.create(Registries.DIMENSION, Identifier.parse(r[1])),
                        new BlockPos(Integer.parseInt(rp[0]), Integer.parseInt(rp[1]), Integer.parseInt(rp[2]))), 0.0F, 0.0F);
                    level.playerList.clear();
                    for (int i = 0; c.containsKey("p" + i); i++) {
                        String[] p = c.get("p" + i).split("/");
                        String[] v = p[0].split(",");
                        OraclePlayer player = (OraclePlayer) alloc(OraclePlayer.class);
                        player.mode = GameType.byName(p.length > 1 ? p[1] : "survival");
                        entityId.setInt(player, 100000 + i);
                        put(player, Entity.class, "position", new Vec3(Double.parseDouble(v[0]), Double.parseDouble(v[1]), Double.parseDouble(v[2])));
                        level.playerList.add(player);
                    }
                    level.entities.clear();
                    for (int i = 0; c.containsKey("e" + i); i++) {
                        String[] p = c.get("e" + i).split("/");
                        String[] v = p[1].split(",");
                        Entity e = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(p[0])).create(level, EntitySpawnReason.LOAD);
                        if (e == null) continue; // monsters do not exist in peaceful
                        e.setPos(Double.parseDouble(v[0]), Double.parseDouble(v[1]), Double.parseDouble(v[2]));
                        if (p.length > 2 && p[2].equals("p") && e instanceof Mob mob) mob.setPersistenceRequired();
                        level.entities.add(e);
                    }
                    RandomSource mainRandom = RandomSource.create(Long.parseLong(c.getOrDefault("seed", "0")));
                    put(level, Level.class, "random", mainRandom);
                    String mode = c.getOrDefault("mode", "chunk");
                    String[] cp = c.getOrDefault("chunk", "0,0").split(",");
                    Chunk chunk = (Chunk) alloc(Chunk.class);
                    chunk.level = level;
                    chunk.at = new ChunkPos(Integer.parseInt(cp[0]), Integer.parseInt(cp[1]));
                    if (mode.equals("gen")) {
                        level.logColumns = true;
                        int n = level.entities.size();
                        RandomSource random = RandomSource.create(Long.parseLong(c.getOrDefault("gseed", "0")));
                        BlockPos sourcePos = new BlockPos(chunk.at.getMinBlockX(), level.getMaxY(), chunk.at.getMinBlockZ());
                        NaturalSpawner.spawnMobsForChunkGeneration(level, sourcePos, chunk.at, random);
                        level.logColumns = false;
                        for (Entity e : level.entities.subList(n, level.entities.size()))
                            if (e.getVehicle() == null)
                                log.add("spawn " + BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()) + " " + bits(e.getX()) + " " + bits(e.getY()) + " " + bits(e.getZ()) + " " + bits(e.getYRot()));
                        log.add("next " + random.nextInt(1000));
                    } else {
                        Map<Long, Chunk> chunks = new HashMap<>();
                        NaturalSpawner.ChunkGetter getter = (key, consumer) -> {
                            if (level.unloaded.contains(key)) return;
                            Chunk k = chunks.computeIfAbsent(key, kk -> {
                                try {
                                    Chunk made = (Chunk) alloc(Chunk.class);
                                    made.level = level;
                                    made.at = ChunkPos.unpack(kk);
                                    return made;
                                } catch (Exception ex) { throw new RuntimeException(ex); }
                            });
                            consumer.accept(k);
                        };
                        var state = (NaturalSpawner.SpawnState) createState.invoke(null, Integer.parseInt(c.getOrDefault("chunkcount", "289")), level, getter, new LocalMobCapCalculator(chunkMap));
                        NaturalSpawner.SpawnPredicate extra = (type, lvl, pos, ch) -> {
                            try {
                                boolean ok = (boolean) canSpawn.invoke(state, type, lvl, pos, ch);
                                log.add("test " + BuiltInRegistries.ENTITY_TYPE.getKey(type) + " " + pos.getX() + " " + pos.getY() + " " + pos.getZ() + " " + ok);
                                return ok;
                            } catch (Exception ex) { throw new RuntimeException(ex); }
                        };
                        NaturalSpawner.AfterSpawnCallback after = (mob, ch) -> {
                            log.add("spawn " + BuiltInRegistries.ENTITY_TYPE.getKey(mob.getType()) + " " + bits(mob.getX()) + " " + bits(mob.getY()) + " " + bits(mob.getZ()) + " " + bits(mob.getYRot()));
                            try { afterSpawn.invoke(state, mob, ch); } catch (Exception ex) { throw new RuntimeException(ex); }
                        };
                        if (mode.equals("pos")) {
                            MobCategory category = MobCategory.valueOf(c.get("cat").toUpperCase(Locale.ROOT));
                            String[] sp = c.get("start").split(",");
                            BlockPos start = new BlockPos(Integer.parseInt(sp[0]), Integer.parseInt(sp[1]), Integer.parseInt(sp[2]));
                            NaturalSpawner.spawnCategoryForPosition(category, level, chunk, start, extra, after);
                        } else {
                            List<MobCategory> categories = NaturalSpawner.getFilteredSpawningCategories(state, c.getOrDefault("enemies", "1").equals("1"), c.getOrDefault("persistent", "1").equals("1"));
                            StringBuilder names = new StringBuilder();
                            for (MobCategory m : categories) names.append(names.isEmpty() ? "" : ",").append(m.getSerializedName());
                            log.add("cats " + names);
                            // spawnForChunk, then spawnCategoryForChunk with its start logged.
                            for (MobCategory category : categories) {
                                boolean ok = (boolean) local.invoke(state, category, chunk.at);
                                log.add("local " + category.getSerializedName() + " " + ok);
                                if (!ok) continue;
                                BlockPos start = (BlockPos) randomPos.invoke(null, level, chunk);
                                log.add("start " + category.getSerializedName() + " " + start.getX() + " " + start.getY() + " " + start.getZ());
                                if (start.getY() >= level.getMinY() + 1) NaturalSpawner.spawnCategoryForPosition(category, level, chunk, start, extra, after);
                            }
                        }
                        log.add("next " + mainRandom.nextInt(1000));
                    }
                    w.println(c.get("name") + " " + String.join(" ; ", log));
                } catch (Throwable t) {
                    Throwable cause = t;
                    while ((cause instanceof InvocationTargetException || cause instanceof RuntimeException && cause.getCause() != null) && cause.getCause() != null) cause = cause.getCause();
                    StringBuilder line = new StringBuilder(c.get("name") + " error " + cause);
                    if (System.getenv("ORACLE_TRACE") != null) for (var frame : cause.getStackTrace()) line.append("\n  at ").append(frame);
                    w.println(line);
                }
            }
        }
        System.exit(0);
    }
}
