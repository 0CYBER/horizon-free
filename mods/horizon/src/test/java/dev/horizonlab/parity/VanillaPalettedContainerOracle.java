package dev.horizonlab.parity;

import com.google.gson.*;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import java.nio.file.*;
import java.util.*;
import java.util.stream.LongStream;
import net.minecraft.core.IdMap;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.world.level.chunk.PalettedContainer;
import net.minecraft.world.level.chunk.PalettedContainerRO;
import net.minecraft.world.level.chunk.Strategy;

/** Runs the real PalettedContainer (single-value, linear, hash-map and global palettes, resize,
 * network write/read, pack/unpack) over scripted edits and records hashes of everything
 * observable: set results, the serialized container after every few edits, the saved form and
 * the containers read back from both. */
public final class VanillaPalettedContainerOracle {
    private static final Gson JSON = new GsonBuilder().serializeNulls().create();
    /** The palettes compare entries by identity, as registry objects allow: intern the boxes. */
    private static final Map<Integer, Integer> INTERNED = new HashMap<>();
    private static Integer intern(int value) { return INTERNED.computeIfAbsent(value, v -> new Integer(v)); }
    private static final class Ids implements IdMap<Integer> {
        final int size;
        Ids(int size) { this.size = size; }
        public int getId(Integer value) { return value < size ? value : -1; }
        public Integer byId(int id) { return id >= 0 && id < size ? intern(id) : null; }
        public int size() { return size; }
        public Iterator<Integer> iterator() { return java.util.stream.IntStream.range(0, size).boxed().iterator(); }
    }
    private static long fnv(long h, int b) { return (h ^ (b & 0xFF)) * 0x100000001b3L; }
    private static final long BASIS = 0xcbf29ce484222325L;
    private static long hashBytes(byte[] bytes) {
        long h = BASIS;
        for (byte b : bytes) h = fnv(h, b);
        return h;
    }
    private static long hashValues(PalettedContainer<Integer> c, int bits) {
        long h = BASIS;
        int n = 1 << (3 * bits);
        for (int i = 0; i < n; i++) {
            int v = c.get(i & (n - 1) & ((1 << bits) - 1), i >> (2 * bits), (i >> bits) & ((1 << bits) - 1));
            for (int k = 0; k < 4; k++) h = fnv(h, v >> (8 * k));
        }
        return h;
    }
    private static byte[] write(PalettedContainer<Integer> c) {
        var buffer = new FriendlyByteBuf(Unpooled.buffer());
        c.write(buffer);
        byte[] bytes = new byte[buffer.readableBytes()];
        buffer.readBytes(bytes);
        return bytes;
    }
    private static String wire(PalettedContainer<Integer> c) {
        byte[] bytes = write(c);
        return bytes.length + ":" + hashBytes(bytes);
    }
    /** Input line: name kind initial every i v i v ...; output: one line of fields. */
    private static String scenario(String line) {
        var t = line.trim().split(" ");
        boolean blocks = t[1].equals("blocks");
        int bits = blocks ? 4 : 2;
        Strategy<Integer> strategy = blocks ? Strategy.createForBlockStates(new Ids(35723)) : Strategy.createForBiomes(new Ids(67));
        var c = new PalettedContainer<>(intern(Integer.parseInt(t[2])), strategy);
        int every = Integer.parseInt(t[3]);
        var checkpoints = new StringJoiner(",");
        long oldsHash = BASIS;
        int n = 0, mask = (1 << bits) - 1;
        for (int k = 4; k + 1 < t.length; k += 2) {
            int i = Integer.parseInt(t[k]);
            int old = c.getAndSetUnchecked(i & mask, i >> (2 * bits), (i >> bits) & mask, intern(Integer.parseInt(t[k + 1])));
            for (int j = 0; j < 4; j++) oldsHash = fnv(oldsHash, old >> (8 * j));
            if (++n % every == 0) checkpoints.add(wire(c));
        }
        var out = new StringJoiner(" ");
        out.add(t[0]).add(Long.toString(oldsHash)).add("cp=" + checkpoints).add("bits=" + c.bitsPerEntry());
        out.add("final=" + wire(c)).add("values=" + hashValues(c, bits));
        var read = new PalettedContainer<>(intern(0), strategy);
        read.read(new FriendlyByteBuf(Unpooled.wrappedBuffer(write(c))));
        out.add("read=" + hashValues(read, bits) + "/" + wire(read));
        var packed = c.pack(strategy);
        long[] storage = packed.storage().map(LongStream::toArray).orElse(null);
        long storageHash = BASIS;
        if (storage != null) for (long word : storage) for (int j = 0; j < 8; j++) storageHash = fnv(storageHash, (int) (word >> (8 * j)));
        var palette = new StringJoiner(",");
        for (int v : packed.paletteEntries()) palette.add(Integer.toString(v));
        out.add("packed=" + packed.bitsPerEntry() + "/" + storageHash + "/" + palette);
        var loaded = PalettedContainer.unpack(strategy, new PalettedContainerRO.PackedData<>(packed.paletteEntries(),
            Optional.ofNullable(storage).map(LongStream::of), -1)).getOrThrow();
        out.add("loaded=" + hashValues(loaded, bits) + "/" + wire(loaded));
        return out.toString();
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        try (var input = Files.newBufferedReader(Path.of(args[0])); var output = Files.newBufferedWriter(Path.of(args[1]))) {
            for (String line; (line = input.readLine()) != null; ) {
                output.write(scenario(line));
                output.newLine();
            }
        }
    }
}
