package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.levelgen.*;

/** Native world-generation bit adapter, forks, counters and seeding methods. */
public final class VanillaWorldgenRandomOracle {
    public static void main(String[] args) throws Exception {
        Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();long seed=f.get("seed").getAsLong();
                RandomSource source=f.get("legacy").getAsBoolean()?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                source.consumeCount(f.get("warmup").getAsInt());
                for(int i=1;i<f.get("layers").getAsInt();i++)source=new WorldgenRandom(source);
                var root=new WorldgenRandom(source);RandomSource random=root;
                for(var value:f.getAsJsonArray("ops")) {
                    var op=value.getAsJsonArray();String kind=op.get(0).getAsString();
                    switch(kind) {
                        case "root"->{random=root;out.println("root");}
                        case "bits"->out.println("bits "+root.next(op.get(1).getAsInt()));
                        case "decoration"->out.println("decoration "+root.setDecorationSeed(op.get(1).getAsLong(),op.get(2).getAsInt(),op.get(3).getAsInt()));
                        case "feature"->{root.setFeatureSeed(op.get(1).getAsLong(),op.get(2).getAsInt(),op.get(3).getAsInt());out.println("feature");}
                        case "large"->{root.setLargeFeatureSeed(op.get(1).getAsLong(),op.get(2).getAsInt(),op.get(3).getAsInt());out.println("large");}
                        case "salt"->{root.setLargeFeatureWithSalt(op.get(1).getAsLong(),op.get(2).getAsInt(),op.get(3).getAsInt(),op.get(4).getAsInt());out.println("salt");}
                        case "slime"->{var r=WorldgenRandom.seedSlimeChunk(op.get(1).getAsInt(),op.get(2).getAsInt(),op.get(3).getAsLong(),op.get(4).getAsLong());out.println("slime "+r.nextLong()+" "+r.nextInt(10));}
                        case "int"->out.println("int "+random.nextInt());
                        case "long"->out.println("long "+random.nextLong());
                        case "bool"->out.println("bool "+random.nextBoolean());
                        case "float"->out.println("float "+Integer.toUnsignedString(Float.floatToRawIntBits(random.nextFloat()),16));
                        case "double"->out.println("double "+Long.toUnsignedString(Double.doubleToRawLongBits(random.nextDouble()),16));
                        case "bound"->{try{out.println("bound "+random.nextInt(op.get(1).getAsInt()));}catch(IllegalArgumentException e){out.println("bound error");}}
                        case "seed"->{random.setSeed(op.get(1).getAsLong());out.println("seed");}
                        case "consume"->{random.consumeCount(op.get(1).getAsInt());out.println("consume");}
                        case "fork"->{random=random.fork();out.println("fork");}
                        case "pos"->{random=random.forkPositional().at(op.get(1).getAsInt(),op.get(2).getAsInt(),op.get(3).getAsInt());out.println("pos");}
                        case "pos_seed"->{random=random.forkPositional().fromSeed(op.get(1).getAsLong());out.println("pos_seed");}
                        case "pos_hash"->{var values=op.get(1).getAsJsonArray();char[] units=new char[values.size()];for(int i=0;i<units.length;i++)units[i]=(char)values.get(i).getAsInt();random=random.forkPositional().fromHashOf(new String(units));out.println("pos_hash");}
                        default->throw new AssertionError(kind);
                    }
                    out.println("count "+root.getCount());
                }
                out.println("tail "+root.nextLong());out.println("count "+root.getCount());
            }
        }
    }
}
