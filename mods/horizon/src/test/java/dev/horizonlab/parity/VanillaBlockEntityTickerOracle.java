package dev.horizonlab.parity;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.util.debug.LevelDebugSynchronizers;
import net.minecraft.world.TickRateManager;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.EntityBlock;
import net.minecraft.world.level.block.JukeboxBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.TickingBlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.chunk.LevelChunk;

/**
 * The order of vanilla's `Level.blockEntityTickers`: real `LevelChunk`s of a
 * server level register, rebind and drop the tickers of real block entities
 * (`addAndRegisterBlockEntity`, `setBlockEntity`, `removeBlockEntity`,
 * `updateBlockEntityTicker` after a state change, `registerAllBlockEntitiesAfterLevelLoad`
 * after a chunk load, `clearAllBlockEntities` when it unloads), and
 * `Level.tickBlockEntities` runs with the tick rate frozen, so it only
 * promotes pending tickers and drops removed ones. After each `tick` the
 * positions of the live tickers are printed in list order.
 *
 * Input lines: `reset`, `load CX CZ BLOCK X Y Z ...` (a chunk's block entities in
 * its save order), `place BLOCK X Y Z`, `replace X Y Z`, `remove X Y Z`,
 * `record X Y Z 0|1` (a jukebox's has_record), `unload CX CZ`, `tick`.
 */
public final class VanillaBlockEntityTickerOracle {
    static Object unsafe;
    static Method allocate;

    static <T> T alloc(Class<T> c) throws Exception {
        return c.cast(allocate.invoke(unsafe, c));
    }

    static void set(Class<?> owner, Object target, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        f.set(target, value);
    }

    static Object get(Class<?> owner, Object target, String name) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        return f.get(target);
    }

    static Map<BlockPos, BlockState> cells = new HashMap<>();
    static Map<Long, Chunk> chunks = new HashMap<>();
    static World world;

    static final class World extends net.minecraft.server.level.ServerLevel {
        TickRateManager rate;
        LevelDebugSynchronizers debug;
        World() {
            super(null, null, null, null, null, null, false, 0L, List.of(), false);
        }
        @Override public boolean isClientSide() { return false; }
        @Override public TickRateManager tickRateManager() { return rate; }
        @Override public LevelDebugSynchronizers debugSynchronizers() { return debug; }
        @Override public void onBlockEntityAdded(BlockEntity blockEntity) { }
        @Override public void levelEvent(net.minecraft.world.entity.Entity source, int type, BlockPos at, int data) { }
        @Override public void gameEvent(net.minecraft.core.Holder<net.minecraft.world.level.gameevent.GameEvent> event,
                net.minecraft.world.phys.Vec3 at, net.minecraft.world.level.gameevent.GameEvent.Context context) { }
        @Override public BlockState getBlockState(BlockPos p) { return cells.getOrDefault(p, Blocks.AIR.defaultBlockState()); }
        @Override public void playSeededSound(net.minecraft.world.entity.Entity except, double x, double y, double z,
                net.minecraft.core.Holder<net.minecraft.sounds.SoundEvent> sound, net.minecraft.sounds.SoundSource source,
                float volume, float pitch, long seed) { }
        @Override public int getMinY() { return -64; }
        @Override public int getHeight() { return 384; }
    }

    static final class Chunk extends LevelChunk {
        Chunk() {
            super(null, null);
        }
        @Override public BlockState getBlockState(BlockPos p) { return cells.getOrDefault(p, Blocks.AIR.defaultBlockState()); }
        @Override public void markUnsaved() { }
    }

    static void reset() throws Exception {
        cells = new HashMap<>();
        chunks = new HashMap<>();
        world = alloc(World.class);
        set(Level.class, world, "blockEntityTickers", new ArrayList<TickingBlockEntity>());
        set(Level.class, world, "pendingBlockEntityTickers", new ArrayList<TickingBlockEntity>());
        set(Level.class, world, "soundSeedGenerator", net.minecraft.util.RandomSource.create(0));
        TickRateManager rate = new TickRateManager();
        set(TickRateManager.class, rate, "runGameElements", false);
        world.rate = rate;
        LevelDebugSynchronizers debug = alloc(LevelDebugSynchronizers.class);
        set(LevelDebugSynchronizers.class, debug, "sleeping", true);
        world.debug = debug;
    }

    static Chunk chunk(int cx, int cz) throws Exception {
        Chunk c = chunks.get(ChunkPos.pack(cx, cz));
        if (c != null) return c;
        c = alloc(Chunk.class);
        set(ChunkAccess.class, c, "blockEntities", new it.unimi.dsi.fastutil.objects.Object2ObjectOpenHashMap<BlockPos, BlockEntity>());
        set(ChunkAccess.class, c, "pendingBlockEntities", new HashMap<BlockPos, net.minecraft.nbt.CompoundTag>());
        set(ChunkAccess.class, c, "chunkPos", new ChunkPos(cx, cz));
        set(LevelChunk.class, c, "level", world);
        set(LevelChunk.class, c, "tickersInLevel", new HashMap<>());
        chunks.put(ChunkPos.pack(cx, cz), c);
        return c;
    }

    /** The chunk at `p`, in the level (a chunk first seen here was already loaded). */
    static Chunk at(BlockPos p) throws Exception {
        Chunk c = chunk(p.getX() >> 4, p.getZ() >> 4);
        c.setLoaded(true);
        return c;
    }

    /** The block's state: a jukebox's has_record from the name (`jukebox_record`). */
    static BlockState state(String name) {
        boolean record = name.equals("jukebox_record");
        Block block = BuiltInRegistries.BLOCK.getValue(Identifier.withDefaultNamespace(record ? "jukebox" : name));
        BlockState s = block.defaultBlockState();
        return block instanceof JukeboxBlock ? s.setValue(JukeboxBlock.HAS_RECORD, record) : s;
    }

    static BlockEntity make(BlockPos p) {
        BlockState s = cells.get(p);
        return ((EntityBlock) s.getBlock()).newBlockEntity(p, s);
    }

    static BlockPos pos(String[] w, int i) {
        return new BlockPos(Integer.parseInt(w[i]), Integer.parseInt(w[i + 1]), Integer.parseInt(w[i + 2]));
    }

    public static void main(String[] args) throws Exception {
        // Registries with item components bound (vaults hold item stacks).
        ParityData.init();
        var allocator = Class.forName("sun.misc.Unsafe");
        var field = allocator.getDeclaredField("theUnsafe");
        field.setAccessible(true);
        unsafe = field.get(null);
        allocate = allocator.getMethod("allocateInstance", Class.class);
        Method update = LevelChunk.class.getDeclaredMethod("updateBlockEntityTicker", BlockEntity.class);
        update.setAccessible(true);
        StringBuilder out = new StringBuilder();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            String[] w = line.trim().split(" ");
            switch (w[0]) {
                case "reset" -> reset();
                case "load" -> {
                    // SerializableChunkData.postLoadChunk, then ChunkStatusTasks.full.
                    Chunk c = chunk(Integer.parseInt(w[1]), Integer.parseInt(w[2]));
                    for (int i = 3; i + 3 < w.length; i += 4) {
                        BlockPos p = pos(w, i + 1);
                        cells.put(p, state(w[i]));
                        c.setBlockEntity(make(p));
                    }
                    c.setLoaded(true);
                    c.registerAllBlockEntitiesAfterLevelLoad();
                }
                case "place" -> {
                    // LevelChunk.setBlockState: a new block entity block.
                    BlockPos p = pos(w, 2);
                    cells.put(p, state(w[1]));
                    at(p).addAndRegisterBlockEntity(make(p));
                }
                case "replace" -> {
                    BlockPos p = pos(w, 1);
                    at(p).addAndRegisterBlockEntity(make(p));
                }
                case "remove" -> {
                    BlockPos p = pos(w, 1);
                    at(p).removeBlockEntity(p);
                    cells.remove(p);
                }
                case "record" -> {
                    // LevelChunk.setBlockState: the same block, another state.
                    BlockPos p = pos(w, 1);
                    BlockState s = state(w[4].equals("1") ? "jukebox_record" : "jukebox");
                    cells.put(p, s);
                    BlockEntity be = at(p).getBlockEntity(p);
                    be.setBlockState(s);
                    update.invoke(at(p), be);
                }
                case "unload" -> {
                    Chunk c = chunks.remove(ChunkPos.pack(Integer.parseInt(w[1]), Integer.parseInt(w[2])));
                    if (c != null) c.clearAllBlockEntities();
                }
                case "tick" -> {
                    world.tickBlockEntities();
                    @SuppressWarnings("unchecked")
                    List<TickingBlockEntity> list = (List<TickingBlockEntity>) get(Level.class, world, "blockEntityTickers");
                    out.append("tick");
                    for (TickingBlockEntity t : list) {
                        BlockPos p = t.getPos();
                        out.append(' ').append(p.getX()).append(',').append(p.getY()).append(',').append(p.getZ());
                    }
                    out.append('\n');
                }
                default -> throw new IllegalArgumentException(line);
            }
        }
        Files.writeString(Path.of(args[1]), out);
    }
}
