package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.resources.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.levelgen.densityfunction.*;

/** Original transport calling actual fixed/checkerboard/End biome resolvers. */
public final class VanillaBiomeSourceOracle {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var ids=new IdMapper<Holder<Biome>>();for(int i=0;i<512;i++)ids.add(Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},ResourceKey.create(Registries.BIOME,Identifier.parse("horizon:biome_"+i))));
        var constructor=TheEndBiomeSource.class.getDeclaredConstructor(Holder.class,Holder.class,Holder.class,Holder.class,Holder.class);constructor.setAccessible(true);
        var end=constructor.newInstance(ids.byId(0),ids.byId(1),ids.byId(2),ids.byId(3),ids.byId(4));
        Path dir=Path.of(args[1]);try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();BiomeResolver resolver;
                switch(f.get("kind").getAsString()) {
                    case "fixed" -> resolver=new FixedBiomeSource(ids.byId(7)).createResolver(null);
                    case "checkerboard" -> {var values=new ArrayList<Holder<Biome>>();for(var v:f.getAsJsonArray("biomes"))values.add(ids.byId(v.getAsInt()));resolver=new CheckerboardColumnBiomeSource(HolderSet.direct(values),f.get("scale").getAsInt()).createResolver(null);}
                    case "end" -> {
                        var sampler=new DensitySampler(){
                            public float sampleValue(SamplerContext context,int x,int y,int z){out.println("sample "+x+" "+y+" "+z);if(f.get("fail").getAsBoolean())throw new IllegalStateException("synthetic sampler failure");return Float.intBitsToFloat(Integer.parseUnsignedInt(f.get("bits").getAsString(),16));}
                            public void sampleVolume(SamplerContext context,DensityBuffer buffer,DensityVolume volume){throw new AssertionError("unexpected bulk query");}
                        }.bind(SamplerContext.builder().enableCaches().build());
                        resolver=end.createResolver(new Climate.Sampler(sampler,sampler,sampler,sampler,sampler,sampler));
                    }
                    default -> throw new IllegalArgumentException("invalid source");
                }
                for(var p:f.getAsJsonArray("positions")){var q=p.getAsJsonArray();try{out.println("biome "+ids.getId(resolver.getNoiseBiome(q.get(0).getAsInt(),q.get(1).getAsInt(),q.get(2).getAsInt())));}catch(ArithmeticException|IllegalStateException error){out.println("error");}}
            }
        }
    }
}
