package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.WorldGenLevel;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.feature.Feature;
import net.minecraft.world.level.levelgen.placement.*;

/** Actual FeaturePlacer executing native modifiers and an observable feature sink. */
public final class VanillaFeaturePlacementOracle {
    private static final class Stop extends RuntimeException {}
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
        var contextMethod=VanillaPlacementOracle.class.getDeclaredMethod("context",JsonObject.class,PrintWriter.class);contextMethod.setAccessible(true);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();var context=(PlacementContext)contextMethod.invoke(null,f,out);long seed=f.get("seed").getAsLong();
                RandomSource random=f.get("legacy").getAsBoolean()?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                var modifiers=new ArrayList<PlacementModifier>();boolean valid=true;
                for(var json:f.getAsJsonArray("modifiers")) {
                    try {var decoded=PlacementModifier.CODEC.parse(JsonOps.INSTANCE,json).result();if(decoded.isPresent())modifiers.add(decoded.get());else valid=false;}
                    catch(IllegalArgumentException error){valid=false;}
                }
                if(!valid)out.println("decode error");else {
                    out.println("decode ok");int[] count={0};int fail=f.get("fail").getAsInt();boolean draw=f.get("draw").getAsBoolean();int mode=f.get("result").getAsInt();
                    var feature=new Feature() {
                        public com.mojang.serialization.MapCodec<? extends Feature> codec(){return com.mojang.serialization.MapCodec.unit(this);}
                        public boolean place(WorldGenLevel level,ChunkGenerator generator,RandomSource rng,BlockPos p) {
                            if(count[0]++==fail)throw new Stop();
                            out.println("feature "+p.getX()+" "+p.getY()+" "+p.getZ());
                            long value=p.getX()+p.getY()+p.getZ();if(draw){value=rng.nextLong();out.println("consumer "+value);}
                            boolean result=mode==1||(mode==2&&(value&1)==0);out.println("result "+result);return result;
                        }
                    };
                    var placed=new PlacedFeature(Holder.direct(feature),modifiers);var placer=new FeaturePlacer(context.getLevel(),context.generator());
                    for(var value:f.getAsJsonArray("positions")) {
                        var p=value.getAsJsonArray();var position=new BlockPos(p.get(0).getAsInt(),p.get(1).getAsInt(),p.get(2).getAsInt());count[0]=0;
                        try {out.println("place "+placer.place(placed,random,position));}catch(Stop|IllegalArgumentException error){out.println("place error");}
                        out.println("next "+random.nextLong());
                    }
                }
                out.println("tail "+random.nextLong());
            }
        }
    }
}
