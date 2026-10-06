package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.lang.reflect.Proxy;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.densityfunction.DensityFunctions;
import net.minecraft.world.level.levelgen.material.rule.BlockRule;

/** Original transport for vertical anchor codecs and generation contexts. */
public final class VanillaAnchorOracle {
    private static WorldGenerationContext context(JsonObject f) {
        var v=f.getAsJsonArray("context");int min=v.get(0).getAsInt(),height=v.get(1).getAsInt(),sea=v.get(2).getAsInt();
        if(f.has("generator")) {
            var g=f.getAsJsonArray("generator");var zero=DensityFunctions.zero();
            var settings=new NoiseGeneratorSettings(new NoiseSettings(g.get(0).getAsInt(),g.get(1).getAsInt()),Blocks.STONE.defaultBlockState(),Blocks.WATER.defaultBlockState(),new NoiseRouter(zero,zero,zero,zero,zero,zero,zero,zero),Holder.direct(new BlockRule(Blocks.STONE.defaultBlockState())),List.of(),g.get(2).getAsInt(),false,Optional.empty(),false,NoiseGeneratorSettings.DebugFunctions.EMPTY);
            var biome=Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},Biomes.PLAINS);
            var generator=new NoiseBasedChunkGenerator(new FixedBiomeSource(biome),Holder.direct(settings));
            return new WorldGenerationContext(generator,LevelHeightAccessor.create(min,height));
        }
        var accessor=(LevelAccessor)Proxy.newProxyInstance(LevelAccessor.class.getClassLoader(),new Class<?>[]{LevelAccessor.class},(proxy,method,args)->switch(method.getName()) {
            case "getMinY"->min;case "getHeight"->height;case "getSeaLevel"->sea;
            default->throw new AssertionError("unexpected accessor method "+method);
        });
        return WorldGenerationContext.of(accessor);
    }
    private static void anchor(VerticalAnchor anchor,WorldGenerationContext context,PrintWriter out) {
        var encoded=VerticalAnchor.CODEC.encodeStart(JsonOps.INSTANCE,anchor).result();
        out.println("anchor "+anchor.resolveY(context)+" "+encoded.map(Object::toString).orElse("error"));
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(String line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();var context=context(f);
                out.println("context "+context.getMinGenY()+" "+context.getGenDepth()+" "+context.seaLevel());
                anchor(VerticalAnchor.bottom(),context,out);anchor(VerticalAnchor.top(),context,out);anchor(VerticalAnchor.seaLevel(),context,out);
                for(var e:f.getAsJsonArray("values")) {
                    int value=e.getAsInt();
                    for(var a:List.of(VerticalAnchor.absolute(value),VerticalAnchor.aboveBottom(value),VerticalAnchor.belowTop(value),VerticalAnchor.relativeToSeaLevel(value)))anchor(a,context,out);
                }
                for(var e:f.getAsJsonArray("json")) {
                    var result=VerticalAnchor.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(e.getAsString())).result();
                    if(result.isPresent())anchor(result.get(),context,out);else out.println("error");
                }
            }
        }
    }
}
