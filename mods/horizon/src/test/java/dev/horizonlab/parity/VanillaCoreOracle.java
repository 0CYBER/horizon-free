package dev.horizonlab.parity;

import java.io.BufferedReader;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.StringTokenizer;
import net.minecraft.core.BlockPos;
import net.minecraft.core.IdMapper;
import net.minecraft.core.SectionPos;
import net.minecraft.util.RandomSource;
import net.minecraft.util.SimpleBitStorage;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.chunk.Strategy;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.levelgen.XoroshiroRandomSource;
import net.minecraft.world.level.levelgen.PositionalRandomFactory;
import net.minecraft.world.level.levelgen.RandomSupport;
import net.minecraft.util.Mth;

/** Original test adapter. Calls downloaded vanilla classes, never reimplements them. */
public final class VanillaCoreOracle {
    private static final int[] BOUNDS = {1, 2, 3, 16, 31, 256, 65537, 1073741825, Integer.MAX_VALUE, 0, -1};
    private static final Strategy<Object> BLOCKS = Strategy.createForBlockStates(new IdMapper<>());

    private static int integer(StringTokenizer args) { return Integer.parseInt(args.nextToken()); }
    private static long longValue(StringTokenizer args) { return Long.parseLong(args.nextToken()); }
    private static String utf16(String token) {
        if(token.equals("-"))return "";
        char[] units=new char[token.length()/4];for(int i=0;i<units.length;i++)units[i]=(char)Integer.parseInt(token.substring(i*4,i*4+4),16);
        return new String(units);
    }

    private static void random(PrintWriter out, RandomSource rng, int count) {
        for (int i = 0; i < count; i++) {
            String value = switch (i % 10) {
                case 0, 9 -> Integer.toString(rng.nextInt());
                case 1 -> Long.toString(rng.nextLong());
                case 2 -> {
                    try { yield Integer.toString(rng.nextInt(BOUNDS[(i / 10) % BOUNDS.length])); }
                    catch (IllegalArgumentException e) { yield "error"; }
                }
                case 3 -> rng.nextBoolean() ? "1" : "0";
                case 4 -> Integer.toUnsignedString(Float.floatToRawIntBits(rng.nextFloat()));
                case 5 -> Long.toUnsignedString(Double.doubleToRawLongBits(rng.nextDouble()));
                case 6 -> Long.toString(rng.fork().nextLong());
                case 7 -> {
                    rng.consumeCount(i % 7 - 2);
                    yield Long.toString(rng.nextLong());
                }
                case 8 -> {
                    if (i % 101 == 8) rng.setSeed(rng.nextLong() - i);
                    yield Integer.toString(rng.nextInt(1073741825));
                }
                default -> throw new AssertionError();
            };
            out.println("rng " + i + " " + value);
        }
    }

    private static void probe(String line, PrintWriter out) {
        StringTokenizer args = new StringTokenizer(line);
        switch (args.nextToken()) {
            case "helpers" -> {
                String kind=args.nextToken();long seed=longValue(args);int min=integer(args),max=integer(args);
                double mean=Double.longBitsToDouble(Long.parseUnsignedLong(args.nextToken(),16)),deviation=Double.longBitsToDouble(Long.parseUnsignedLong(args.nextToken(),16));
                float smallMean=Float.intBitsToFloat(Integer.parseUnsignedInt(args.nextToken(),16)),smallDeviation=Float.intBitsToFloat(Integer.parseUnsignedInt(args.nextToken(),16));
                RandomSource source=kind.equals("legacy")?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                for(int i=0;i<32;i++) {
                    String inclusive,exclusive;
                    try { inclusive=Integer.toString(source.nextIntBetweenInclusive(min,max)); } catch(IllegalArgumentException e) { inclusive="error"; }
                    try { exclusive=Integer.toString(source.nextInt(min,max)); } catch(IllegalArgumentException e) { exclusive="error"; }
                    String d=Long.toUnsignedString(Double.doubleToLongBits(source.triangle(mean,deviation)));
                    String f=Integer.toUnsignedString(Float.floatToIntBits(source.triangle(smallMean,smallDeviation)));
                    out.printf("helpers %d %s %s %s %s %d%n",i,inclusive,exclusive,d,f,source.nextLong());
                }
                random(out,source,24);
            }
            case "factory" -> {
                String kind=args.nextToken();int mode=integer(args);long lo=longValue(args),hi=longValue(args);
                BlockPos position=new BlockPos(integer(args),integer(args),integer(args));long seed=longValue(args);String name=utf16(args.nextToken());int count=integer(args);
                RandomSource source=kind.equals("legacy")?new LegacyRandomSource(lo):new XoroshiroRandomSource(lo);
                PositionalRandomFactory factory=mode!=0?source.forkPositional():kind.equals("legacy")?new LegacyRandomSource.LegacyPositionalRandomFactory(lo):new XoroshiroRandomSource.XoroshiroPositionalRandomFactory(lo,hi);
                var hash=RandomSupport.seedFromHashOf(name);out.printf("seeds %d %d %d %d%n",Mth.getSeed(position),name.hashCode(),hash.seedLo(),hash.seedHi());
                out.println("factory at");random(out,factory.at(position),count);out.println("factory seed");random(out,factory.fromSeed(seed),count);out.println("factory hash");random(out,factory.fromHashOf(name),count);
                if(mode!=0)out.println("parent "+source.nextLong());
            }
            case "position" -> {
                BlockPos p = new BlockPos(integer(args), integer(args), integer(args));
                BlockPos offset = p.offset(integer(args), integer(args), integer(args));
                long packed = p.asLong();
                BlockPos decoded = BlockPos.of(packed);
                SectionPos section = SectionPos.of(p);
                long sp = section.asLong();
                SectionPos sd = SectionPos.of(sp);
                ChunkPos chunk = ChunkPos.containing(p);
                long cp = chunk.pack();
                ChunkPos cd = ChunkPos.unpack(cp);
                short relative = SectionPos.sectionRelativePos(p);
                BlockPos rebuilt = section.relativeToBlockPos(relative);
                out.printf("position %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d%n",
                    packed, decoded.getX(), decoded.getY(), decoded.getZ(), sp, sd.x(), sd.y(), sd.z(),
                    cp, cd.x(), cd.z(), chunk.getRegionX(), chunk.getRegionZ(), chunk.getRegionLocalX(), chunk.getRegionLocalZ(),
                    Short.toUnsignedInt(relative), BLOCKS.getIndex(p.getX() & 15, p.getY() & 15, p.getZ() & 15),
                    rebuilt.getX(), rebuilt.getY(), rebuilt.getZ(), offset.getX(), offset.getY(), offset.getZ());
            }
            case "legacy" -> random(out, new LegacyRandomSource(longValue(args)), integer(args));
            case "xoroshiro" -> random(out, new XoroshiroRandomSource(longValue(args)), integer(args));
            case "xoroshiro_state" -> random(out, new XoroshiroRandomSource(longValue(args), longValue(args)), integer(args));
            case "storage" -> {
                int bits = integer(args);
                int len = integer(args);
                long seed = longValue(args);
                SimpleBitStorage storage = new SimpleBitStorage(bits, len);
                RandomSource rng = new LegacyRandomSource(seed);
                long mask = Math.min((1L << bits) - 1, Integer.MAX_VALUE);
                for (int i = 0; i < len * 3; i++) {
                    int index = rng.nextInt(len);
                    int value = (int) (Integer.toUnsignedLong(rng.nextInt()) & mask);
                    int old = storage.getAndSet(index, value);
                    out.println("set " + i + " " + index + " " + Integer.toUnsignedString(old) + " " + Integer.toUnsignedString(storage.get(index)));
                }
                for (int i = 0; i < storage.getRaw().length; i++) out.println("word " + i + " " + storage.getRaw()[i]);
                int[] unpacked = new int[len];
                storage.unpack(unpacked);
                for (int i = 0; i < len; i++) out.println("value " + i + " " + unpacked[i]);
                long[] words = new long[storage.getRaw().length];
                Arrays.fill(words, -1L);
                SimpleBitStorage imported = new SimpleBitStorage(bits, len, words);
                for (int i = 0; i < len; i++) out.println("import " + i + " " + imported.get(i));
                if (len > 0) imported.set(len - 1, 0);
                for (int i = 0; i < words.length; i++) out.println("import_word " + i + " " + words[i]);
                for (int value : new int[]{0, 1, (int) mask}) {
                    imported.fill(value);
                    for (int i = 0; i < words.length; i++) out.println("fill " + value + " " + i + " " + words[i]);
                }
                long[] before = storage.getRaw().clone();
                try { storage.set(len, 0); out.println("invalid_index 0"); }
                catch (IllegalArgumentException e) { out.println("invalid_index 1"); }
                if (len > 0) {
                    try { storage.set(0, bits == 32 ? -1 : 1 << bits); out.println("invalid_value 0"); }
                    catch (IllegalArgumentException e) { out.println("invalid_value 1"); }
                }
                out.println("unchanged " + (Arrays.equals(before, storage.getRaw()) ? "1" : "0"));
            }
            default -> throw new IllegalArgumentException("unknown operation");
        }
        if (args.hasMoreTokens()) throw new IllegalArgumentException("unexpected trailing argument");
    }

    public static void main(String[] args) throws Exception {
        try (BufferedReader input = Files.newBufferedReader(Path.of(args[0]), StandardCharsets.UTF_8);
             PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            String line;
            while ((line = input.readLine()) != null) probe(line, out);
            if (out.checkError()) throw new IllegalStateException("oracle output failed");
        }
    }
}
