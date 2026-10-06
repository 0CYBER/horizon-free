package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.carver.*;

/** Original transport calling actual decoded canyon carvers and tracing mask writes. */
public final class VanillaCanyonCarverOracle {
    private static final class OutputFailure extends RuntimeException {}
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
        var contextMethod=VanillaAnchorOracle.class.getDeclaredMethod("context",JsonObject.class);contextMethod.setAccessible(true);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();long seed=f.get("seed").getAsLong();
                var context=(WorldGenerationContext)contextMethod.invoke(null,f);
                RandomSource random=f.get("legacy").getAsBoolean()?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                var decoded=WorldCarver.DIRECT_CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(f.get("definition").getAsString()));
                if(decoded.result().isEmpty())out.println("decode error");else {
                    var carver=decoded.result().get();out.println("decode ok");
                    var mask=new CarvingMask(context.getMinGenY()+1,context.getMinGenY()+context.getGenDepth()-8);
                    var t=f.getAsJsonArray("target");var o=f.getAsJsonArray("origin");var target=new ChunkPos(t.get(0).getAsInt(),t.get(1).getAsInt());var origin=new ChunkPos(o.get(0).getAsInt(),o.get(1).getAsInt());
                    for(int repeat=0;repeat<f.get("repeats").getAsInt();repeat++) {
                        out.println("start "+carver.isStartChunk(random));int[] calls={0};
                        var output=new CarverOutput(){public int minY(){return mask.minY();}public int maxY(){return mask.maxY();}public void carve(int x,int y,int z){out.println("cell "+x+" "+y+" "+z);int previous=calls[0]++;if(previous==f.get("fail").getAsInt())throw new OutputFailure();mask.carve(x,y,z);}};
                        try{out.println("carve "+carver.carve(context,random,target,origin,output)+" "+calls[0]);}catch(OutputFailure|IllegalArgumentException error){out.println("carve error "+calls[0]);}
                        mask.visit((x,z,low,high)->out.println("column "+x+" "+z+" "+low+" "+high));out.println("next "+random.nextLong());
                    }
                }
                out.println("tail "+random.nextLong());
            }
        }
    }
}
