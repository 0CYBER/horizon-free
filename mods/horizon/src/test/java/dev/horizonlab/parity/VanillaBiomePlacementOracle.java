package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.WorldGenLevel;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.feature.Feature;
import net.minecraft.world.level.levelgen.placement.*;

/** Native biome-filter queries and per-invocation placed-feature context. */
public final class VanillaBiomePlacementOracle {
    private static final class Stop extends RuntimeException {}
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        Path dir=Path.of(args[1]);
        var contextMethod=VanillaPlacementOracle.class.getDeclaredMethod("context",JsonObject.class);contextMethod.setAccessible(true);
        var constructor=BiomeGenerationSettings.class.getDeclaredConstructor(HolderSet.class,List.class);constructor.setAccessible(true);
        var getter=ChunkGenerator.class.getDeclaredField("generationSettingsGetter");getter.setAccessible(true);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();long seed=f.get("seed").getAsLong();
                RandomSource random=f.get("legacy").getAsBoolean()?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                if(f.get("worldgen").getAsBoolean())random=new WorldgenRandom(random);
                var modifiers=new ArrayList<PlacementModifier>();for(var value:f.getAsJsonArray("placement"))modifiers.add(PlacementModifier.CODEC.parse(JsonOps.INSTANCE,value).getOrThrow());
                int[] queries={0},calls={0};int queryFail=f.get("query_fail").getAsInt(),featureFail=f.get("feature_fail").getAsInt();
                var bounds=new JsonObject();bounds.add("context",JsonParser.parseString("[-64,384,63]"));
                var generator=((PlacementContext)contextMethod.invoke(null,bounds)).generator();
                var biomes=List.of(Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},ResourceKey.create(Registries.BIOME,Identifier.parse("horizon:a"))),Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},ResourceKey.create(Registries.BIOME,Identifier.parse("horizon:b"))));
                var leaf=new Feature(){
                    public MapCodec<? extends Feature> codec(){return MapCodec.unit(this);}
                    public boolean place(WorldGenLevel world,ChunkGenerator gen,RandomSource rng,BlockPos p){
                        out.println("feature "+p.getX()+" "+p.getY()+" "+p.getZ());
                        if(f.get("draw").getAsBoolean())out.println("draw "+rng.nextLong());
                        if(calls[0]++==featureFail)throw new Stop();
                        if(f.get("nested").getAsBoolean())return new FeaturePlacer(world,gen).place(new PlacedFeature(Holder.direct(this),List.of(BiomeFilter.biome())),rng,p);
                        return f.get("return_mode").getAsInt()==0||(f.get("return_mode").getAsInt()==2&&Math.floorMod(p.getX()+p.getY()+p.getZ(),2)==0);
                    }
                };
                var root=new PlacedFeature(Holder.direct(leaf),modifiers);
                var copy=new PlacedFeature(root.feature(),root.placement());
                var other=new PlacedFeature(Holder.direct(new Feature(){public MapCodec<? extends Feature> codec(){return MapCodec.unit(this);}public boolean place(WorldGenLevel w,ChunkGenerator g,RandomSource r,BlockPos p){return true;}}),modifiers);
                var settings=new ArrayList<BiomeGenerationSettings>();
                for(var value:f.getAsJsonArray("membership")) {
                    int mode=value.getAsInt();var values=mode==0?List.<Holder<PlacedFeature>>of():List.of(Holder.direct(mode==1?root:mode==2?copy:other));
                    settings.add(constructor.newInstance(HolderSet.direct(List.of()),List.of(HolderSet.direct(values))));
                }
                getter.set(generator,(java.util.function.Function<Holder<Biome>,BiomeGenerationSettings>)(biome)->{
                    int index=biome==biomes.get(0)?0:1;out.println("settings "+index);if(queries[0]++==queryFail)throw new Stop();return settings.get(index);
                });
                var world=(WorldGenLevel)Proxy.newProxyInstance(WorldGenLevel.class.getClassLoader(),new Class<?>[]{WorldGenLevel.class},(proxy,method,values)->switch(method.getName()) {
                    case "getMinY"->-64;case "getHeight"->384;case "getSeaLevel"->63;
                    case "getBiome"->{var p=(BlockPos)values[0];out.println("biome "+p.getX()+" "+p.getY()+" "+p.getZ());if(queries[0]++==queryFail)throw new Stop();int index=Math.floorMod(p.getX()*31+p.getY()*7+p.getZ()*17,2);out.println("selected "+index);yield biomes.get(index);}
                    default->throw new AssertionError(method);
                });
                var p=f.getAsJsonArray("origin");var origin=new BlockPos(p.get(0).getAsInt(),p.get(1).getAsInt(),p.get(2).getAsInt());
                boolean top=f.get("top").getAsBoolean(),direct=f.get("direct").getAsBoolean();
                var context=new PlacementContext(world,generator,top?Optional.of(root):Optional.empty());var placer=new FeaturePlacer(world,generator);
                for(int repeat=0;repeat<3;repeat++) {
                    queries[0]=0;calls[0]=0;
                    try {
                        if(direct){final RandomSource rng=random;modifiers.get(0).modify(context,random,origin,q->leaf.place(world,generator,rng,q));out.println("place true");}
                        else out.println("place "+(top?placer.placeWithBiomeCheck(root,random,origin):placer.place(root,random,origin)));
                    } catch(Stop|IllegalStateException|IndexOutOfBoundsException error){out.println("place error");}
                    out.println("next "+random.nextLong());
                }
                out.println("tail "+random.nextLong());
            }
        }
    }
}
