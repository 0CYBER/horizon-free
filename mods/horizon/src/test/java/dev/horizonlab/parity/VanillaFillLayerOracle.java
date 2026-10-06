package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.world.level.levelgen.placement.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.WorldGenLevel;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.feature.Feature;

/** Invokes native block-writing features through an observable mutable world. */
public final class VanillaFillLayerOracle {
    private static final class Stop extends RuntimeException {}
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        final Block[] BLOCKS={Blocks.AIR,Blocks.CAVE_AIR,Blocks.VOID_AIR,Blocks.STONE,Blocks.WATER,Blocks.OAK_LEAVES,Blocks.DIRT};
        if(args[0].equals("export")) {
            VanillaBlockStateCodecOracle.main(args);
            var facts=JsonParser.parseString(Files.readString(Path.of(args[1]))).getAsJsonObject();
            var air=new JsonArray();for(var state:Block.BLOCK_STATE_REGISTRY)if(state.isAir())air.add(Block.getId(state));
            facts.add("air_ids",air);facts.addProperty("height_limit",net.minecraft.world.level.dimension.DimensionType.Y_SIZE);
            Files.writeString(Path.of(args[1]),facts.toString());return;
        }
        Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();long seed=f.get("seed").getAsLong();
                RandomSource random=f.get("legacy").getAsBoolean()?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                var decoded=Feature.DIRECT_CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(f.get("definition").getAsString())).result();
                if(decoded.isEmpty())out.println("decode error");else {
                    out.println("decode ok");int[] calls={0};int fail=f.get("fail").getAsInt(),reject=f.get("reject").getAsInt();
                    var states=new TreeMap<BlockPos,BlockState>(Comparator.<BlockPos>comparingInt(BlockPos::getX).thenComparingInt(BlockPos::getY).thenComparingInt(BlockPos::getZ));
                    var world=(WorldGenLevel)Proxy.newProxyInstance(WorldGenLevel.class.getClassLoader(),new Class<?>[]{WorldGenLevel.class},(proxy,method,values)->{
                        if(method.getName().equals("getMinY")) {
                            out.println("min");int call=calls[0]++;if(call==fail)throw new Stop();
                            int minimum=f.get("world_min").getAsInt();out.println("minimum "+minimum);return minimum;
                        }
                        if(method.getName().equals("getHeight")&&(values==null||values.length==0))return 384;
                        if(method.getName().equals("getSeaLevel"))return 63;
                        if(method.getName().equals("setBlockAndUpdate"))return InvocationHandler.invokeDefault(proxy,method,values);
                        var p=((BlockPos)values[0]).immutable();int call=calls[0]++;
                        switch(method.getName()) {
                            case "getBlockState"->{
                                out.println("get "+p.getX()+" "+p.getY()+" "+p.getZ());if(call==fail)throw new Stop();
                                var state=states.getOrDefault(p,BLOCKS[Math.floorMod(p.getX()*31+p.getY()*7+p.getZ()*17,7)].defaultBlockState());out.println("state "+Block.getId(state));return state;
                            }
                            case "setBlock"->{
                                var state=(BlockState)values[1];int flags=(Integer)values[2];out.println("set "+p.getX()+" "+p.getY()+" "+p.getZ()+" "+Block.getId(state)+" "+flags);if(call==fail)throw new Stop();
                                boolean result=reject==0||call%reject!=0;if(result)states.put(p,state);out.println("written "+result);return result;
                            }
                            default->throw new AssertionError("unexpected world method "+method);
                        }
                    });
                    var p=f.getAsJsonArray("origin");var origin=new BlockPos(p.get(0).getAsInt(),p.get(1).getAsInt(),p.get(2).getAsInt());
                    for(int repeat=0;repeat<f.get("repeats").getAsInt();repeat++) {
                        calls[0]=0;
                        try {out.println("place "+decoded.get().place(world,null,random,origin));}catch(Stop|IllegalArgumentException error){out.println("place error");}
                        out.println("next "+random.nextLong());
                        for(var entry:states.entrySet()){var q=entry.getKey();out.println("stored "+q.getX()+" "+q.getY()+" "+q.getZ()+" "+Block.getId(entry.getValue()));}
                        out.println("end");
                    }
                }
                out.println("tail "+random.nextLong());
            }
        }
    }
}
