package dev.horizonlab.parity;

import com.mojang.serialization.Codec;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.nbt.*;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.chunk.storage.RegionFile;
import net.minecraft.world.level.chunk.storage.RegionStorageInfo;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.ticks.*;

/** Original adapter for actual block/fluid SavedTick codecs and region persistence. */
public final class VanillaTickDataOracle {
    static { SharedConstants.tryDetectVersion(); Bootstrap.bootStrap(); }
    private static final ChunkPos POS = new ChunkPos(0, 0);
    private static final Codec<List<SavedTick<Block>>> BLOCKS = SavedTick.codec(BuiltInRegistries.BLOCK.byNameCodec()).listOf();
    private static final Codec<List<SavedTick<Fluid>>> FLUIDS = SavedTick.codec(BuiltInRegistries.FLUID.byNameCodec()).listOf();
    private static <T> List<SavedTick<T>> samples(List<T> types) {
        List<SavedTick<T>> ticks = new ArrayList<>();
        int[] delays = {Integer.MIN_VALUE, -1, 0, 1, 100, Integer.MAX_VALUE};
        for (int i = 0; i < 256; i++) {
            SavedTick<T> tick = new SavedTick<>(types.get(i % types.size()), new BlockPos((i & 15) + (i % 17 == 0 ? 16 : 0), (i % 9 - 4) * 16, (i >>> 4) & 15), delays[i % delays.length], TickPriority.byValue(i % 11 - 5));
            ticks.add(tick);
            if (i % 23 == 0) ticks.add(tick);
        }
        return ticks;
    }
    private static <T> CompoundTag ids(List<T> types, Registry<T> registry) {
        CompoundTag tag = new CompoundTag();
        for (T type : types) tag.putInt(registry.getKey(type).toString(), registry.getId(type));
        return tag;
    }
    private static <T> Tag encode(Codec<List<SavedTick<T>>> codec, List<SavedTick<T>> ticks) { return codec.encodeStart(NbtOps.INSTANCE, ticks).getOrThrow(); }
    private static <T> List<SavedTick<T>> changed(Codec<List<SavedTick<T>>> codec, Tag input, long time) {
        List<SavedTick<T>> saved = SavedTick.filterTickListForChunk(codec.parse(NbtOps.INSTANCE, input).getOrThrow(), POS);
        LevelChunkTicks<T> ticks = new LevelChunkTicks<>(saved);
        ticks.unpack(time);
        ScheduledTick<T> first = ticks.poll();
        if (first != null) ticks.schedule(new ScheduledTick<>(first.type(), first.pos(), time + 123, TickPriority.HIGH, 77));
        return ticks.pack(time);
    }
    private static void generate(Path base) throws Exception {
        List<Block> blocks = List.of(Blocks.STONE, Blocks.OAK_LOG, Blocks.REPEATER, Blocks.WATER, Blocks.CHEST, Blocks.REDSTONE_WIRE);
        List<Fluid> fluids = List.of(Fluids.EMPTY, Fluids.WATER, Fluids.FLOWING_WATER, Fluids.LAVA, Fluids.FLOWING_LAVA);
        CompoundTag root = new CompoundTag();
        root.putInt("DataVersion", 5023); root.putInt("xPos", 0); root.putInt("zPos", 0); root.put("sections", new ListTag());
        root.put("block_ticks", encode(BLOCKS, samples(blocks))); root.put("fluid_ticks", encode(FLUIDS, samples(fluids)));
        for (String field : List.of("block_ticks", "fluid_ticks")) {
            ListTag list = (ListTag) root.get(field);
            CompoundTag a = (CompoundTag) list.get(3), b = (CompoundTag) list.get(4), c = (CompoundTag) list.get(5);
            a.putLong("t", Long.MAX_VALUE); a.putFloat("p", Float.NaN);
            b.putDouble("t", Double.POSITIVE_INFINITY); b.putShort("p", (short) 999); b.putByte("y", (byte) -7);
            c.putDouble("x", 4.75); c.putDouble("p", Double.NEGATIVE_INFINITY);
        }
        root.put("block_ids", ids(blocks, BuiltInRegistries.BLOCK)); root.put("fluid_ids", ids(fluids, BuiltInRegistries.FLUID));
        root.putString("unknown-root-data", "preserved through scheduled-tick mutation");
        NbtIo.write(root, base.resolve("ticks-input.nbt"));
    }
    private static void verify(Path base) throws Exception {
        CompoundTag original = NbtIo.read(base.resolve("ticks-input.nbt"));
        List<SavedTick<Block>> blocks = changed(BLOCKS, original.get("block_ticks"), 1000);
        List<SavedTick<Fluid>> fluids = changed(FLUIDS, original.get("fluid_ticks"), Long.MAX_VALUE - 8);
        CompoundTag expected = original.copy();
        expected.put("block_ticks", encode(BLOCKS, blocks)); expected.put("fluid_ticks", encode(FLUIDS, fluids));
        CompoundTag actual = NbtIo.read(base.resolve("ticks-output.nbt"));
        if (!actual.equals(expected)) throw new AssertionError("Rust saved tick NBT differs from vanilla");
        Path regionPath = base.resolve("tick-region/r.0.0.mca");
        RegionStorageInfo info = new RegionStorageInfo("horizon-tick-fixture", Level.OVERWORLD, "chunk");
        try (RegionFile region = new RegionFile(info, regionPath, regionPath.getParent(), false);
             DataInputStream input = region.getChunkDataInputStream(POS)) {
            if (input == null || !NbtIo.read(input).equals(expected)) throw new AssertionError("Rust region tick data differs from vanilla");
        }
        System.out.printf("HORIZON_TICK_DATA_RESULT {\"block_ticks\":%d,\"fluid_ticks\":%d,\"region_chunks\":1}%n", blocks.size(), fluids.size());
    }
    public static void main(String[] args) throws Exception {
        Path base = Path.of(args[1]);
        switch (args[0]) { case "generate" -> generate(base); case "verify" -> verify(base); default -> throw new AssertionError("unknown mode"); }
    }
}
