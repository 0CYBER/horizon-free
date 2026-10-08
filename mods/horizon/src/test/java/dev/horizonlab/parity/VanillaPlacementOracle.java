package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.lang.reflect.Proxy;
import java.lang.reflect.InvocationHandler;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.densityfunction.DensityFunctions;
import net.minecraft.world.level.levelgen.material.rule.BlockRule;
import net.minecraft.world.level.levelgen.placement.*;

/** Original transport invoking actual placement codecs and modifier callbacks. */
public final class VanillaPlacementOracle {
    private static final class Stop extends RuntimeException {}
    private static PlacementContext context(JsonObject f) {return context(f,null);}
    private static PlacementContext context(JsonObject f,PrintWriter out) {
        var v=f.getAsJsonArray("context");int min=v.get(0).getAsInt(),height=v.get(1).getAsInt(),sea=v.get(2).getAsInt();
        var zero=DensityFunctions.zero();
        var settings=new NoiseGeneratorSettings(new NoiseSettings(min,height),Blocks.STONE.defaultBlockState(),Blocks.WATER.defaultBlockState(),new NoiseRouter(zero,zero,zero,zero,zero,zero,zero,zero),Holder.direct(new BlockRule(Blocks.STONE.defaultBlockState())),List.of(),sea,false,Optional.empty(),false,NoiseGeneratorSettings.DebugFunctions.EMPTY);
        var biome=Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},Biomes.PLAINS);
        var generator=new NoiseBasedChunkGenerator(new FixedBiomeSource(biome),Holder.direct(settings));
        int worldMin=f.has("world_min")?f.get("world_min").getAsInt():min;int mode=f.has("height_mode")?f.get("height_mode").getAsInt():0;int fail=f.has("query_fail")?f.get("query_fail").getAsInt():-1;int[] calls={0};boolean layerWorld=f.has("layer_world")&&f.get("layer_world").getAsBoolean();
        var level=(WorldGenLevel)Proxy.newProxyInstance(WorldGenLevel.class.getClassLoader(),new Class<?>[]{WorldGenLevel.class},(proxy,method,args)->switch(method.getName()) {
            case "isInsideBuildHeight"->{
                if(args[0] instanceof BlockPos p) {
                    if(out!=null)out.println("inside "+p.getX()+" "+p.getY()+" "+p.getZ());
                    if(calls[0]++==fail)throw new IllegalArgumentException("bounds query failure");
                    var result=InvocationHandler.invokeDefault(proxy,method,args);
                    if(out!=null)out.println("bounds_result "+result);yield result;
                }
                yield InvocationHandler.invokeDefault(proxy,method,args);
            }
            case "isOutsideBuildHeight"->{
                if(out!=null)out.println("outside "+args[0]);
                if(calls[0]++==fail)throw new IllegalArgumentException("bounds query failure");
                var result=InvocationHandler.invokeDefault(proxy,method,args);
                if(out!=null)out.println("bounds_result "+result);yield result;
            }
            case "getMaxY"->InvocationHandler.invokeDefault(proxy,method,args);
            case "getMinY"->worldMin;
            case "getHeight"->{
                if(args==null||args.length==0)yield height;
                var kind=(Heightmap.Types)args[0];int x=(Integer)args[1],z=(Integer)args[2];
                if(out!=null)out.println("height "+kind.name()+" "+x+" "+z);
                if(calls[0]++==fail)throw new IllegalArgumentException("height query failure");
                int result;
                if(layerWorld)result=min+24+((x^z)&3);
                else if(mode==0)result=min+Math.floorMod(x*31+z*17+kind.ordinal()*7,129)-64;
                else {int[] values={Integer.MIN_VALUE,Integer.MAX_VALUE,0,-1,min,min+1};result=values[(kind.ordinal()+((x^z)&3))%6];}
                if(out!=null)out.println("height_result "+result);yield result;
            }
            case "getBlockState"->{
                var p=(BlockPos)args[0];if(out!=null)out.println("block "+p.getX()+" "+p.getY()+" "+p.getZ());
                if(calls[0]++==fail)throw new IllegalArgumentException("block query failure");
                var blocks=new net.minecraft.world.level.block.Block[]{Blocks.BEDROCK,Blocks.STONE,Blocks.AIR,Blocks.WATER,Blocks.DIRT,Blocks.CAVE_AIR,Blocks.LAVA,Blocks.BUBBLE_COLUMN,Blocks.VOID_AIR,Blocks.GRASS_BLOCK,Blocks.AIR,Blocks.WATER};
                int index=p.getY()<=min?0:Math.floorMod(p.getY()-min+((p.getX()*31+p.getZ()*17)&3),12);
                var block=blocks[index];if(out!=null)out.println("block_result "+net.minecraft.core.registries.BuiltInRegistries.BLOCK.getKey(block));yield block.defaultBlockState();
            }
            case "getSeaLevel"->sea;
            default->throw new AssertionError("unexpected world method "+method);
        });
        var context=new PlacementContext(level,generator,Optional.empty());return context;
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();var context=context(f,out);long seed=f.get("seed").getAsLong();
                RandomSource random=f.get("legacy").getAsBoolean()?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                Optional<PlacementModifier> decoded;
                try {decoded=PlacementModifier.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(f.get("definition").getAsString())).result();}catch(IllegalArgumentException error){decoded=Optional.empty();}
                if(decoded.isEmpty())out.println("decode error");else {
                    out.println("decode ok");var provider=decoded.get();
                    for(var value:f.getAsJsonArray("positions")) {
                        var p=value.getAsJsonArray();var position=new BlockPos(p.get(0).getAsInt(),p.get(1).getAsInt(),p.get(2).getAsInt());
                        if(f.has("count_only")&&f.get("count_only").getAsBoolean()) {
                            try {out.println("count "+((RepeatingPlacement)provider).count(random,position));}catch(IllegalArgumentException error){out.println("count error");}
                            out.println("next "+random.nextLong());continue;
                        }
                        int[] count={0};int fail=f.get("fail").getAsInt();boolean draw=f.get("draw").getAsBoolean();
                        try {provider.modify(context,random,position,result->{
                            if(count[0]++==fail)throw new Stop();
                            out.println("position "+result.getX()+" "+result.getY()+" "+result.getZ());
                            if(draw)out.println("consumer "+random.nextLong());
                        });out.println("modify ok");}catch(Stop|IllegalArgumentException error){out.println("modify error");}
                        out.println("next "+random.nextLong());
                    }
                }
                out.println("tail "+random.nextLong());
            }
        }
    }
}
