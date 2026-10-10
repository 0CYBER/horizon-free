package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.sounds.SoundEvent;
import net.minecraft.sounds.SoundSource;
import net.minecraft.util.RandomSource;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.LevelChunk;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.ticks.LevelTicks;

/**
 * Drives vanilla block behaviour (updateShape, randomTick, tick, neighbour
 * updates, setBlock cascades and multi-tick runs) over a map-backed headless
 * ServerLevel. Shared by tools/test_block_update_parity.py,
 * test_random_tick_parity.py and test_redstone_parity.py. Block entities do not
 * exist here; entities never exist; every side effect is appended to an event
 * log compared against the Rust port.
 */
public final class VanillaBlockUpdateOracle {
    static Object unsafe;
    static Method allocate;

    @SuppressWarnings("unchecked")
    static <T> T make(Class<T> type) {
        try {
            return (T) allocate.invoke(unsafe, type);
        } catch (Exception e) {
            throw new AssertionError(e);
        }
    }

    static void set(Class<?> owner, Object target, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        f.set(target, value);
    }

    static BlockState AIR;

    static String pos(BlockPos p) {
        return p.getX() + "," + p.getY() + "," + p.getZ();
    }

    static final class Chunk extends LevelChunk {
        World world;
        Chunk() {
            super(null, null);
        }
        @Override
        public BlockState setBlockState(BlockPos pos, BlockState state, int flags) {
            BlockState old = world.cells.getOrDefault(pos.immutable(), AIR);
            if (old == state) {
                return null;
            }
            if (state == AIR) {
                world.cells.remove(pos);
            } else {
                world.cells.put(pos.immutable(), state);
            }
            Block newBlock = state.getBlock();
            boolean blockChanged = !old.is(newBlock);
            boolean movedByPiston = (flags & 64) != 0;
            if ((blockChanged || newBlock instanceof net.minecraft.world.level.block.BaseRailBlock) && ((flags & 1) != 0 || movedByPiston)) {
                old.affectNeighborsAfterRemoval(world, pos, movedByPiston);
            }
            if (!world.getBlockState(pos).is(newBlock)) {
                return null;
            }
            if ((flags & 512) == 0) {
                state.onPlace(world, pos, old, movedByPiston);
            }
            return old;
        }
        @Override
        public net.minecraft.world.level.chunk.status.ChunkStatus getPersistedStatus() {
            return net.minecraft.world.level.chunk.status.ChunkStatus.FULL;
        }
        @Override
        public net.minecraft.server.level.FullChunkStatus getFullStatus() {
            return net.minecraft.server.level.FullChunkStatus.ENTITY_TICKING;
        }
    }

    static final class World extends net.minecraft.server.level.ServerLevel {
        Map<BlockPos, BlockState> cells = new HashMap<>();
        RandomSource rng;
        long time;
        long subTick;
        Chunk chunk;
        LevelTicks<Block> blockTicks;
        LevelTicks<Fluid> fluidTicks;
        StringBuilder log = new StringBuilder();
        int skyLight = 15;
        int blockLight = 0;
        GameRules rules;
        Server server;
        boolean raining, thundering;
        World() {
            super(null, null, null, null, null, null, false, 0L, List.of(), false);
        }
        java.util.LinkedHashSet<String> blockEvents;
        void emit(String line) {
            log.append(line).append('\n');
        }
        @Override public BlockState getBlockState(BlockPos p) { return cells.getOrDefault(p, AIR); }
        @Override public FluidState getFluidState(BlockPos p) { return getBlockState(p).getFluidState(); }
        // A real level always holds the block entity of an entity block; this
        // one creates it on first use and replaces it when the block changes.
        Map<BlockPos, BlockEntity> blockEntities;
        @Override public BlockEntity getBlockEntity(BlockPos p) {
            BlockState state = getBlockState(p);
            if (!(state.getBlock() instanceof net.minecraft.world.level.block.EntityBlock entityBlock)) {
                return null;
            }
            if (blockEntities == null) blockEntities = new HashMap<>();
            BlockEntity be = blockEntities.get(p);
            if (be == null || !be.getType().isValid(state)) {
                be = entityBlock.newBlockEntity(p.immutable(), state);
                if (be == null) return null;
                be.setLevel(this);
                blockEntities.put(p.immutable(), be);
            } else {
                be.setBlockState(state);
            }
            return be;
        }
        @Override public LevelChunk getChunkAt(BlockPos p) { return chunk; }
        @Override public LevelChunk getChunk(int x, int z) { return chunk; }
        @Override public boolean isLoaded(BlockPos p) { return true; }
        @Override public boolean hasChunkAt(BlockPos p) { return true; }
        @Override public boolean hasChunksAt(int x0, int z0, int x1, int z1) { return true; }
        @Override public boolean hasChunksAt(int x0, int y0, int z0, int x1, int y1, int z1) { return true; }
        @Override public boolean hasChunk(int x, int z) { return true; }
        @Override public boolean isClientSide() { return false; }
        @Override public RandomSource getRandom() { return rng; }
        @Override public int getMinY() { return -64; }
        @Override public int getHeight() { return 384; }
        @Override public long getGameTime() { return time; }
        @Override public long nextSubTickCount() { return subTick++; }
        @Override public LevelTicks<Block> getBlockTicks() { return blockTicks; }
        @Override public LevelTicks<Fluid> getFluidTicks() { return fluidTicks; }
        @Override public void sendBlockUpdated(BlockPos p, BlockState a, BlockState b, int f) { }
        @Override public void setBlocksDirty(BlockPos p, BlockState a, BlockState b) { }
        @Override public net.minecraft.core.RegistryAccess registryAccess() { return RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY); }
        @Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() { return FeatureFlags.DEFAULT_FLAGS; }
        @Override public GameRules getGameRules() { return rules; }
        @Override public net.minecraft.server.MinecraftServer getServer() { return server; }
        @Override public net.minecraft.world.level.border.WorldBorder getWorldBorder() { return new net.minecraft.world.level.border.WorldBorder(); }
        @Override public int getSeaLevel() { return 63; }
        @Override public int getBrightness(net.minecraft.world.level.LightLayer layer, BlockPos p) { return layer == net.minecraft.world.level.LightLayer.SKY ? skyLight : blockLight; }
        @Override public int getRawBrightness(BlockPos p, int darkening) { return Math.max(skyLight - darkening, blockLight); }
        @Override public int getSkyDarken() { return 0; }
        @Override public boolean isRaining() { return raining; }
        @Override public boolean isThundering() { return thundering; }
        @Override public boolean isRainingAt(BlockPos p) { return raining; }
        @Override public boolean isUnobstructed(BlockState state, BlockPos pos, net.minecraft.world.phys.shapes.CollisionContext c) { return true; }
        @Override public boolean isUnobstructed(Entity entity) { return true; }
        @Override public List<Entity> getEntities(Entity except, AABB box, java.util.function.Predicate<? super Entity> filter) { return List.of(); }
        @Override public <T extends Entity> List<T> getEntitiesOfClass(Class<T> type, AABB box, java.util.function.Predicate<? super T> filter) { return List.of(); }
        @Override public List<net.minecraft.server.level.ServerPlayer> players() { return List.of(); }
        @Override public net.minecraft.world.entity.player.Player getNearestPlayer(double x, double y, double z, double d, boolean b) { return null; }
        @Override public boolean addFreshEntity(Entity entity) {
            emit("E " + BuiltInRegistries.ENTITY_TYPE.getKey(entity.getType()));
            return true;
        }
        @Override public void levelEvent(Entity source, int type, BlockPos p, int data) { emit("L " + type + " " + pos(p) + " " + data); }
        @Override public void playSeededSound(Entity except, double x, double y, double z, net.minecraft.core.Holder<SoundEvent> sound, SoundSource source, float volume, float pitch, long seed) {
            emit("S " + pos(BlockPos.containing(x, y, z)) + " " + sound.value().location());
        }
        @Override public void playSeededSound(Entity except, Entity src, net.minecraft.core.Holder<SoundEvent> sound, SoundSource source, float volume, float pitch, long seed) {
            emit("S entity " + sound.value().location());
        }
        @Override public void gameEvent(net.minecraft.core.Holder<net.minecraft.world.level.gameevent.GameEvent> e, Vec3 p, net.minecraft.world.level.gameevent.GameEvent.Context c) {
            emit("G " + pos(BlockPos.containing(p)) + " " + e.getRegisteredName());
        }
        @Override public <T extends net.minecraft.core.particles.ParticleOptions> int sendParticles(T p, double x, double y, double z, int n, double a, double b, double c, double s) { return 0; }
        @Override public void blockEvent(BlockPos p, Block block, int a, int b) { blockEvents.add("B " + pos(p) + " " + BuiltInRegistries.BLOCK.getKey(block) + " " + a + " " + b); }
        @Override public void updatePOIOnBlockStateChange(BlockPos p, BlockState a, BlockState b) { }
        @Override public boolean isPositionEntityTicking(BlockPos p) { return true; }
        @Override public boolean shouldTickBlocksAt(BlockPos p) { return true; }
        @Override public boolean shouldTickBlocksAt(long p) { return true; }
        // Allocated without a constructor, so the sound seed generator, attribute
        // system and chunk source are missing. Sound seeds never touch the level
        // RNG, the overworld keeps every attribute default and entity ids are not
        // compared.
        int nextEntityId;
        net.minecraft.world.attribute.EnvironmentAttributeSystem attributes;
        @Override public int getNextEntityId() { return ++nextEntityId; }
        @Override public net.minecraft.world.attribute.EnvironmentAttributeSystem environmentAttributes() {
            if (attributes == null) attributes = net.minecraft.world.attribute.EnvironmentAttributeSystem.builder().build();
            return attributes;
        }
        @Override public void playSound(Entity except, double x, double y, double z, SoundEvent sound, SoundSource source, float volume, float pitch) {
            playSeededSound(except, x, y, z, BuiltInRegistries.SOUND_EVENT.wrapAsHolder(sound), source, volume, pitch, 0L);
        }
        @Override public void playSound(Entity except, double x, double y, double z, net.minecraft.core.Holder<SoundEvent> sound, SoundSource source, float volume, float pitch) {
            playSeededSound(except, x, y, z, sound, source, volume, pitch, 0L);
        }
        @Override public void playSound(Entity except, Entity src, SoundEvent sound, SoundSource source, float volume, float pitch) {
            playSeededSound(except, src, BuiltInRegistries.SOUND_EVENT.wrapAsHolder(sound), source, volume, pitch, 0L);
        }
    }

    static final class Loot extends net.minecraft.server.ReloadableServerRegistries.Holder {
        World world;
        Loot() {
            super(null);
        }
        @Override
        public net.minecraft.world.level.storage.loot.LootTable getLootTable(net.minecraft.resources.ResourceKey<net.minecraft.world.level.storage.loot.LootTable> id) {
            world.emit("D " + id.identifier());
            return net.minecraft.world.level.storage.loot.LootTable.EMPTY;
        }
    }

    static final class Server extends net.minecraft.server.MinecraftServer {
        Loot loot;
        Server() {
            super(null, null, null, null, null, null, null, null, null, false, null);
        }
        @Override public net.minecraft.server.ReloadableServerRegistries.Holder reloadableRegistries() { return loot; }
        @Override protected boolean initServer() { return false; }
        @Override public int getMaxPlayers() { return 0; }
        @Override public net.minecraft.server.permissions.LevelBasedPermissionSet operatorUserPermissions() { return null; }
        @Override public net.minecraft.server.permissions.PermissionSet getFunctionCompilationPermissions() { return null; }
        @Override public boolean shouldRconBroadcast() { return false; }
        @Override protected net.minecraft.util.debugchart.SampleLogger getTickTimeLogger() { return null; }
        @Override public boolean isTickTimeLoggingEnabled() { return false; }
        @Override public net.minecraft.SystemReport fillServerSystemReport(net.minecraft.SystemReport r) { return r; }
        @Override public boolean isDedicatedServer() { return true; }
        @Override public int getRateLimitPacketsPerSecond() { return 0; }
        @Override public int getCommandSpamThresholdSeconds() { return 0; }
        @Override public int getChatSpamThresholdSeconds() { return 0; }
        @Override public boolean useNativeTransport() { return false; }
        @Override public boolean isPublished() { return false; }
        @Override public boolean shouldInformAdmins() { return false; }
        @Override public boolean isSingleplayerOwner(net.minecraft.server.players.NameAndId id) { return false; }
    }

    static List<String> summary(World w, Map<BlockPos, BlockState> before) {
        List<String> out = new ArrayList<>();
        Set<BlockPos> all = new HashSet<>(before.keySet());
        all.addAll(w.cells.keySet());
        List<BlockPos> sorted = new ArrayList<>(all);
        sorted.sort(Comparator.<BlockPos>comparingInt(q -> q.getX()).thenComparingInt(q -> q.getY()).thenComparingInt(q -> q.getZ()));
        for (BlockPos p : sorted) {
            BlockState a = before.getOrDefault(p, AIR), b = w.cells.getOrDefault(p, AIR);
            if (a != b) {
                out.add("set " + pos(p) + " " + Block.getId(b));
            }
        }
        return out;
    }

    @SuppressWarnings("unchecked")
    static void pending(World w, StringBuilder out) throws Exception {
        for (var pair : List.of(new Object[]{"bt", w.blockTicks}, new Object[]{"ft", w.fluidTicks})) {
            Field f = LevelTicks.class.getDeclaredField("allContainers");
            f.setAccessible(true);
            var map = (it.unimi.dsi.fastutil.longs.Long2ObjectMap<net.minecraft.world.ticks.LevelChunkTicks<Object>>) f.get(pair[1]);
            List<String> rows = new ArrayList<>();
            for (var container : map.values()) {
                container.getAll().forEach(t -> {
                    String type = t.type() instanceof Block b ? BuiltInRegistries.BLOCK.getKey(b).toString() : BuiltInRegistries.FLUID.getKey((Fluid) t.type()).toString();
                    rows.add(pair[0] + " " + pos(t.pos()) + " " + type + " " + (t.triggerTick() - w.time) + " " + t.priority().getValue());
                });
            }
            Collections.sort(rows);
            for (String r : rows) out.append(r).append('\n');
        }
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        AIR = Blocks.AIR.defaultBlockState();
        Path resources = Path.of(args[0]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(resources.resolve("block-tags.json"))).getAsJsonObject());
        VanillaLevelShapeOracle.bindFluidTags(resources.resolve("data/minecraft/tags/fluid"));
        var allocator = Class.forName("sun.misc.Unsafe");
        var field = allocator.getDeclaredField("theUnsafe");
        field.setAccessible(true);
        unsafe = field.get(null);
        allocate = allocator.getMethod("allocateInstance", Class.class);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[2]))); var lines = Files.lines(Path.of(args[1]))) {
            int index = 0;
            for (var line : (Iterable<String>) lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject();
                StringBuilder result = new StringBuilder("case " + index++ + "\n");
                try {
                    run(f, result);
                } catch (Throwable e) {
                    Throwable c = e;
                    while (c.getCause() != null) c = c.getCause();
                    StackTraceElement[] st = c.getStackTrace();
                    result.append("error ").append(c.getClass().getSimpleName()).append(' ').append(st.length > 0 ? st[0].toString() : "").append('\n');
                }
                out.print(result);
            }
        }
    }

    static BlockPos readPos(JsonArray a) {
        return new BlockPos(a.get(0).getAsInt(), a.get(1).getAsInt(), a.get(2).getAsInt());
    }

    static World build(JsonObject f) throws Exception {
        World w = make(World.class);
        w.cells = new HashMap<>();
        w.log = new StringBuilder();
        w.blockEvents = new java.util.LinkedHashSet<>();
        for (var cell : f.getAsJsonArray("cells")) {
            var c = cell.getAsJsonArray();
            BlockState s = Block.stateById(c.get(3).getAsInt());
            if (s != AIR) w.cells.put(new BlockPos(c.get(0).getAsInt(), c.get(1).getAsInt(), c.get(2).getAsInt()), s);
        }
        w.rng = new LegacyRandomSource(f.get("seed").getAsLong());
        w.time = f.has("time") ? f.get("time").getAsLong() : 0;
        w.rules = new GameRules(FeatureFlags.DEFAULT_FLAGS);
        if (f.has("light")) {
            w.skyLight = f.getAsJsonArray("light").get(0).getAsInt();
            w.blockLight = f.getAsJsonArray("light").get(1).getAsInt();
        }
        w.raining = f.has("raining") && f.get("raining").getAsBoolean();
        w.thundering = f.has("thundering") && f.get("thundering").getAsBoolean();
        set(Level.class, w, "random", w.rng);
        set(Level.class, w, "neighborUpdater", new net.minecraft.world.level.redstone.CollectingNeighborUpdater(w, 512));
        w.server = make(Server.class);
        w.server.loot = make(Loot.class);
        w.server.loot.world = w;
        w.chunk = make(Chunk.class);
        w.chunk.world = w;
        w.blockTicks = new LevelTicks<>(l -> true);
        w.fluidTicks = new LevelTicks<>(l -> true);
        for (int x = -3; x <= 3; x++) {
            for (int z = -3; z <= 3; z++) {
                w.blockTicks.addContainer(new net.minecraft.world.level.ChunkPos(x, z), new net.minecraft.world.ticks.LevelChunkTicks<>());
                w.fluidTicks.addContainer(new net.minecraft.world.level.ChunkPos(x, z), new net.minecraft.world.ticks.LevelChunkTicks<>());
            }
        }
        return w;
    }

    static void run(JsonObject f, StringBuilder result) throws Exception {
        World w = build(f);
        Map<BlockPos, BlockState> before = new HashMap<>(w.cells);
        String mode = f.get("mode").getAsString();
        BlockPos p = readPos(f.getAsJsonArray("pos"));
        BlockState state = w.getBlockState(p);
        switch (mode) {
            case "shape" -> {
                Direction d = Direction.from3DDataValue(f.get("dir").getAsInt());
                BlockPos np = p.relative(d);
                BlockState n = Block.stateById(f.get("neighbor").getAsInt());
                BlockState s = state.updateShape(w, w, p, d, np, n, w.rng);
                result.append("shape ").append(Block.getId(s)).append('\n');
            }
            case "random" -> {
                if (state.isRandomlyTicking()) state.randomTick(w, p, w.rng);
                FluidState fs = state.getFluidState();
                if (fs.isRandomlyTicking()) fs.randomTick(w, p, w.rng);
            }
            case "tick" -> state.tick(w, p, w.rng);
            case "ftick" -> state.getFluidState().tick(w, p, state);
            case "neighbor" -> {
                BlockPos from = readPos(f.getAsJsonArray("from"));
                w.getBlockState(from);
                state.handleNeighborChanged(w, p, w.getBlockState(from).getBlock(), null, false);
            }
            case "run" -> {
                for (var op : f.getAsJsonArray("ops")) {
                    var o = op.getAsJsonObject();
                    switch (o.get("op").getAsString()) {
                        case "set" -> w.setBlock(readPos(o.getAsJsonArray("pos")), Block.stateById(o.get("state").getAsInt()), o.get("flags").getAsInt());
                        case "ticks" -> {
                            for (int i = 0; i < o.get("n").getAsInt(); i++) {
                                w.time++;
                                w.blockTicks.tick(w.time, 65536, (bp, b) -> { if (w.getBlockState(bp).is(b)) w.getBlockState(bp).tick(w, bp, w.rng); });
                                w.fluidTicks.tick(w.time, 65536, (bp, fl) -> {
                                    if (w.getFluidState(bp).getType() == fl) w.getFluidState(bp).tick(w, bp, w.getBlockState(bp));
                                });
                                if (o.has("snap") && o.get("snap").getAsBoolean()) {
                                    result.append("tick ").append(w.time).append('\n');
                                    for (String s : summary(w, before)) result.append(s).append('\n');
                                }
                            }
                        }
                        default -> throw new IllegalStateException(o.toString());
                    }
                }
            }
            default -> throw new IllegalStateException(mode);
        }
        for (String s : summary(w, before)) result.append(s).append('\n');
        pending(w, result);
        result.append(w.log);
        for (String s : w.blockEvents) result.append(s).append('\n');
        result.append("rng ").append(w.rng.nextLong()).append('\n');
    }
}
