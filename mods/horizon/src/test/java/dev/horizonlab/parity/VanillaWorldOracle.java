package dev.horizonlab.parity;

import com.mojang.serialization.Codec;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.IdMapper;
import net.minecraft.nbt.*;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.PalettedContainer;
import net.minecraft.world.level.chunk.Strategy;
import net.minecraft.world.level.chunk.storage.RegionFile;
import net.minecraft.world.level.chunk.storage.RegionFileVersion;
import net.minecraft.world.level.chunk.storage.RegionStorageInfo;

/** Original interoperability adapter. Game serialization is always performed by vanilla classes. */
public final class VanillaWorldOracle {
    private static final int[] SIZES = {1,2,16,17,32,33,64,65,128,129,256,257,512,513,4096};
    static { SharedConstants.tryDetectVersion(); Bootstrap.bootStrap(); }
    private static final IdMapper<String> IDS = new IdMapper<>();
    static { for (int i = 0; i < 8192; i++) IDS.add("state" + i); }
    private static final RegionStorageInfo INFO = new RegionStorageInfo("horizon-fixture", Level.OVERWORLD, "chunk");
    private static final Strategy<BlockState> BLOCKS = Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY);
    private static final Codec<PalettedContainer<BlockState>> BLOCK_CODEC = PalettedContainer.codecRW(BlockState.CODEC, BLOCKS, Blocks.AIR.defaultBlockState());

    private static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }
    private static RegionFileVersion codec(int id) { return RegionFileVersion.fromId(id); }
    private static Tag read(Path path, int compression, boolean network) throws Exception {
        try (DataInputStream in = new DataInputStream(codec(compression).wrap(Files.newInputStream(path)))) {
            return network ? NbtIo.readAnyTag(in, NbtAccounter.unlimitedHeap()) : NbtIo.readUnnamedTag(in, NbtAccounter.unlimitedHeap());
        }
    }
    private static void write(Tag tag, Path path, int compression, boolean network) throws Exception {
        Files.createDirectories(path.getParent());
        try (DataOutputStream out = new DataOutputStream(codec(compression).wrap(Files.newOutputStream(path)))) {
            if (network) NbtIo.writeAnyTag(tag, out); else NbtIo.writeUnnamedTag(tag, out);
        }
    }
    private static CompoundTag specimen() {
        CompoundTag root = new CompoundTag();
        root.putByte("byte", Byte.MIN_VALUE);
        root.putShort("short", Short.MIN_VALUE);
        root.putInt("int", Integer.MIN_VALUE);
        root.putLong("long", Long.MIN_VALUE);
        root.putFloat("float", Float.intBitsToFloat(0x7f800001));
        root.putDouble("double", Double.NEGATIVE_INFINITY);
        root.put("negative-zero-float", new FloatTag(-0.0f));
        root.put("negative-zero-double", new DoubleTag(-0.0));
        root.putString("text\0\ud800", "nul\0 / unicode \u20ac \ud83c\udf0d / unpaired \ud800x\udfff");
        root.putString("max-ascii", "a".repeat(65535));
        root.putIntArray("ints", new int[]{0, -1, Integer.MIN_VALUE, Integer.MAX_VALUE});
        root.putLongArray("longs", new long[]{0, -1, Long.MIN_VALUE, Long.MAX_VALUE});
        byte[] bytes = new byte[1_200_013];
        new Random(173).nextBytes(bytes);
        root.putByteArray("bytes", bytes);
        ListTag mixed = new ListTag();
        mixed.add(IntTag.valueOf(42));
        mixed.add(StringTag.valueOf("different type"));
        CompoundTag wrapper = new CompoundTag();
        wrapper.putInt("", 19);
        mixed.add(wrapper);
        CompoundTag normal = new CompoundTag();
        normal.putString("kept", "nested");
        mixed.add(normal);
        mixed.add(new ListTag());
        root.put("mixed", mixed);
        CompoundTag branch = new CompoundTag();
        for (int i = 0; i < 40; i++) { CompoundTag next = new CompoundTag(); next.put("child", branch); branch = next; }
        root.put("deep", branch);
        return root;
    }

    private static Codec<PalettedContainer<String>> paletteCodec(boolean biome) {
        // IdMapper keys use object identity. Decoding must return registry-owned
        // objects, just as the real BlockState codec does.
        Codec<String> stateCodec = Codec.STRING.xmap(value -> IDS.byId(Integer.parseInt(value.substring(5))), value -> value);
        return PalettedContainer.codecRW(stateCodec, biome ? Strategy.createForBiomes(IDS) : Strategy.createForBlockStates(IDS), IDS.byId(0));
    }
    private static PalettedContainer<String> palette(boolean biome, int distinct) {
        PalettedContainer<String> result = new PalettedContainer<>(IDS.byId(0), biome ? Strategy.createForBiomes(IDS) : Strategy.createForBlockStates(IDS));
        int axis = biome ? 2 : 4, mask = (1 << axis) - 1, len = biome ? 64 : 4096;
        for (int i = 0; i < len; i++) result.set(i & mask, i >> (axis * 2), (i >> axis) & mask, IDS.byId(i % distinct));
        return result;
    }
    private static CompoundTag chunk(ChunkPos pos, boolean oversized) {
        CompoundTag root = new CompoundTag();
        root.putInt("DataVersion", SharedConstants.getCurrentVersion().dataVersion().version());
        root.putInt("xPos", pos.x()); root.putInt("zPos", pos.z()); root.putInt("yPos", -4);
        root.putString("Status", "minecraft:full");
        root.putString("unknown-future-field", "preserve this exactly");
        root.putLong("LastUpdate", 123456789L);
        ListTag sections = new ListTag();
        CompoundTag section = new CompoundTag(); section.putByte("Y", (byte)-4);
        PalettedContainer<BlockState> blocks = new PalettedContainer<>(Blocks.AIR.defaultBlockState(), BLOCKS);
        blocks.set(1, 0, 0, Blocks.OAK_LOG.defaultBlockState());
        section.put("block_states", BLOCK_CODEC.encodeStart(NbtOps.INSTANCE, blocks).getOrThrow());
        section.putString("unknown-section-field", "retain");
        sections.add(section); root.put("sections", sections);
        root.put("block_entities", new ListTag());
        if (oversized) { byte[] bytes = new byte[1_200_013]; new Random(173).nextBytes(bytes); root.putByteArray("large-fixture", bytes); }
        return root;
    }

    private static void generate(Path base) throws Exception {
        CompoundTag specimen = specimen();
        for (int codec = 1; codec <= 4; codec++) {
            write(specimen, base.resolve("nbt/" + codec + ".bin"), codec, false);
            write(specimen, base.resolve("network/" + codec + ".bin"), codec, true);
            Path folder = base.resolve("regions/" + codec); Files.createDirectories(folder);
            try (RegionFile region = new RegionFile(INFO, folder.resolve("r.-1.0.mca"), folder, codec(codec), false)) {
                for (ChunkPos pos : List.of(new ChunkPos(-32, 0), new ChunkPos(-1, 31))) {
                    try (DataOutputStream out = region.getChunkDataOutputStream(pos)) { NbtIo.write(chunk(pos, pos.x() == -1), out); }
                }
            }
        }
        for (boolean biome : new boolean[]{false, true}) {
            for (int size : SIZES) {
                if (biome && size > 64) continue;
                Tag data = paletteCodec(biome).encodeStart(NbtOps.INSTANCE, palette(biome, size)).getOrThrow();
                write(data, base.resolve("palettes/" + (biome ? "biome" : "block") + "-" + size + ".nbt"), 3, false);
            }
        }
        CompoundTag registry = new CompoundTag();
        registry.putInt("DataVersion", SharedConstants.getCurrentVersion().dataVersion().version());
        ListTag states = new ListTag();
        CompoundTag defaults = new CompoundTag();
        for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
            check(Block.BLOCK_STATE_REGISTRY.getId(state) == states.size(), "registry IDs are not dense");
            states.add(BlockState.FULL_CODEC.encodeStart(NbtOps.INSTANCE, state).getOrThrow());
            if (state == state.getBlock().defaultBlockState()) {
                Tag encoded = BlockState.FULL_CODEC.encodeStart(NbtOps.INSTANCE, state).getOrThrow();
                defaults.putInt(((CompoundTag)encoded).getStringOr("id", ""), Block.BLOCK_STATE_REGISTRY.getId(state));
            }
        }
        registry.put("states", states); registry.put("defaults", defaults);
        write(registry, base.resolve("block-registry.nbt"), 3, false);
        System.out.println("HORIZON_WORLD_GENERATED states=" + states.size());
    }

    private static void verify(Path base) throws Exception {
        int nbt = 0, palettes = 0, regions = 0, blocks = 0;
        for (boolean network : new boolean[]{false, true}) for (int source = 1; source <= 4; source++) for (int target = 1; target <= 4; target++) {
            String directory = network ? "network" : "nbt";
            Tag original = read(base.resolve(directory + "/" + source + ".bin"), source, network);
            Tag actual = read(base.resolve("rust/" + directory + "/" + source + "-" + target + ".bin"), target, network);
            check(original.equals(actual), "NBT semantic mismatch " + directory + " " + source + " " + target);
            nbt++;
        }
        for (boolean biome : new boolean[]{false, true}) for (int size : SIZES) {
            if (biome && size > 64) continue;
            String name = (biome ? "biome" : "block") + "-" + size + ".nbt";
            PalettedContainer<String> control = paletteCodec(biome).parse(NbtOps.INSTANCE, read(base.resolve("palettes/" + name), 3, false)).getOrThrow();
            int controlAxis = biome ? 2 : 4, controlMask = (1 << controlAxis) - 1;
            for (int i = 0; i < (biome ? 64 : 4096); i++)
                check(control.get(i & controlMask, i >> (controlAxis * 2), (i >> controlAxis) & controlMask).equals("state" + (i % size)), "vanilla palette control failed");
            for (String variant : List.of("edited", "compact")) {
                Tag tag = read(base.resolve("rust/palettes/" + variant + "-" + name), 3, false);
                PalettedContainer<String> actual = paletteCodec(biome).parse(NbtOps.INSTANCE, tag).getOrThrow();
                int axis = biome ? 2 : 4, mask = (1 << axis) - 1, len = biome ? 64 : 4096;
                for (int i = 0; i < len; i++) {
                    String expected = "state" + (i == len - 1 ? 4096 : i % size);
                    check(actual.get(i & mask, i >> (axis * 2), (i >> axis) & mask).equals(expected), "palette index mismatch " + name + " " + i);
                    blocks++;
                }
                palettes++;
            }
        }
        for (int source = 1; source <= 4; source++) for (String variant : List.of("copy", "1", "2", "3", "4", "edited")) {
            Path folder = base.resolve("rust/regions/" + source + "-" + variant);
            try (RegionFile region = new RegionFile(INFO, folder.resolve("r.-1.0.mca"), folder, false)) {
                for (ChunkPos pos : List.of(new ChunkPos(-32, 0), new ChunkPos(-1, 31))) {
                    try (DataInputStream in = region.getChunkDataInputStream(pos)) {
                        check(in != null, "chunk missing");
                        CompoundTag actual = NbtIo.read(in);
                        CompoundTag expected = chunk(pos, pos.x() == -1);
                        if (variant.equals("edited") && pos.x() == -32) {
                            CompoundTag section = actual.getListOrEmpty("sections").getCompoundOrEmpty(0);
                            PalettedContainer<BlockState> states = BLOCK_CODEC.parse(NbtOps.INSTANCE, section.get("block_states")).getOrThrow();
                            check(states.get(0, 0, 0) == Blocks.STONE.defaultBlockState(), "edited block differs");
                            check(states.get(1, 0, 0) == Blocks.OAK_LOG.defaultBlockState(), "neighbor changed");
                            section.put("block_states", expected.getListOrEmpty("sections").getCompoundOrEmpty(0).get("block_states"));
                        }
                        check(expected.equals(actual), "region data changed " + source + " " + variant + " " + pos);
                        regions++;
                    }
                }
                check(!region.hasChunk(new ChunkPos(-31, 0)), "unexpected extra chunk");
            }
        }
        Tag actualRegistry = read(base.resolve("rust/block-registry.nbt"), 3, false);
        check(read(base.resolve("block-registry.nbt"), 3, false).equals(actualRegistry), "registry roundtrip differs");
        PalettedContainer<BlockState> resident = BLOCK_CODEC.parse(NbtOps.INSTANCE, read(base.resolve("rust/runtime-section.nbt"), 3, false)).getOrThrow();
        for (int i = 0; i < 4096; i++) check(resident.get(i & 15, i >> 8, (i >> 4) & 15) == Block.BLOCK_STATE_REGISTRY.byId(i * 7 % Block.BLOCK_STATE_REGISTRY.size()), "resident block state differs");
        int[] expectedStates = new int[4096];
        long random = 0x31415926L;
        for (int step = 0; step < 100_000; step++) {
            random = random * 6364136223846793005L + 1;
            int index = (int) ((random >>> 32) % 4096);
            expectedStates[index] = step % 3 == 0 ? 0 : Integer.remainderUnsigned((int) (random >>> 16), Block.BLOCK_STATE_REGISTRY.size());
        }
        PalettedContainer<BlockState> mutated = BLOCK_CODEC.parse(NbtOps.INSTANCE, read(base.resolve("rust/runtime-mutated.nbt"), 3, false)).getOrThrow();
        for (int i = 0; i < 4096; i++) check(mutated.get(i & 15, i >> 8, (i >> 4) & 15) == Block.BLOCK_STATE_REGISTRY.byId(expectedStates[i]), "mutated resident block state differs");
        System.out.printf("HORIZON_WORLD_RESULT {\"nbt_pairs\":%d,\"palettes\":%d,\"palette_values\":%d,\"region_chunks\":%d,\"registry_states\":%d,\"resident_values\":8192}%n", nbt, palettes, blocks, regions, Block.BLOCK_STATE_REGISTRY.size());
    }

    public static void main(String[] args) throws Exception {
        if (args[0].equals("generate")) generate(Path.of(args[1]));
        else if (args[0].equals("verify")) verify(Path.of(args[1]));
        else throw new IllegalArgumentException("unknown mode");
    }
}
