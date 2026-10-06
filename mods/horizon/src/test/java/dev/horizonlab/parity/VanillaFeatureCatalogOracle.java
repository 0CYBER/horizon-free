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
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.resources.*;
import net.minecraft.tags.*;
import com.mojang.serialization.Lifecycle;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.world.level.levelgen.placement.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.WorldGenLevel;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.feature.Feature;

/** Invokes native block-writing features through an observable mutable world. */
public final class VanillaFeatureCatalogOracle {
    private static final class Stop extends RuntimeException {}
    private static Optional<Feature> decodeRoot(RegistryOps<JsonElement> ops,JsonObject f) {
        var json=JsonParser.parseString(f.get("definition").getAsString());
        if(!f.get("placed_root").getAsBoolean())return Feature.CODEC.parse(ops,json).result().map(Holder::value);
        return PlacedFeature.CODEC.parse(ops,json).result().map(holder->new Feature(){
            public com.mojang.serialization.MapCodec<? extends Feature> codec(){return com.mojang.serialization.MapCodec.unit(this);}
            public boolean place(WorldGenLevel world,ChunkGenerator generator,RandomSource random,BlockPos p){return holder.value().place(world,generator,random,p);}
        });
    }
    @SuppressWarnings("unchecked")
    private static RegistryOps<JsonElement> loadCatalog(Path dir) throws Exception {
        var bundle=JsonParser.parseString(Files.readString(dir.resolve("catalog.json"))).getAsJsonObject();
        var features=new MappedRegistry<Feature>(Registries.FEATURE,Lifecycle.stable());
        var placed=new MappedRegistry<PlacedFeature>(Registries.PLACED_FEATURE,Lifecycle.stable());
        var ops=RegistryOps.create(JsonOps.INSTANCE,new RegistryOps.RegistryInfoLookup(){public <T> Optional<HolderGetter<T>> lookup(ResourceKey<? extends Registry<? extends T>> key){
            if(key.equals(Registries.FLUID))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(net.minecraft.core.registries.BuiltInRegistries.FLUID);
            if(key.equals(Registries.BLOCK))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(net.minecraft.core.registries.BuiltInRegistries.BLOCK);
            if(key.equals(Registries.FEATURE))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(features);
            if(key.equals(Registries.PLACED_FEATURE))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(placed);
            return Optional.empty();
        }});
        for(var entry:bundle.getAsJsonObject("features").entrySet())if(!entry.getKey().equals("horizon:composite"))features.register(ResourceKey.create(Registries.FEATURE,Identifier.parse(entry.getKey())),Feature.DIRECT_CODEC.parse(ops,JsonParser.parseString(entry.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);
        for(var entry:bundle.getAsJsonObject("placed").entrySet())placed.register(ResourceKey.create(Registries.PLACED_FEATURE,Identifier.parse(entry.getKey())),PlacedFeature.DIRECT_CODEC.parse(ops,JsonParser.parseString(entry.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);
        placed.freeze();
        var loader=new TagLoader<Holder<PlacedFeature>>((id,required)->placed.get(id).map(value->(Holder<PlacedFeature>)value),"tags/worldgen/placed_feature");
        var definitions=new HashMap<Identifier,List<TagLoader.EntryWithSource>>();
        for(var entry:bundle.getAsJsonObject("tags").entrySet()) {
            var file=TagFile.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(entry.getValue().getAsString())).getOrThrow();
            var values=new ArrayList<TagLoader.EntryWithSource>();for(var value:file.entries())values.add(new TagLoader.EntryWithSource(value,"fixture"));
            definitions.put(Identifier.parse(entry.getKey()),values);
        }
        var tags=new HashMap<TagKey<PlacedFeature>,List<Holder<PlacedFeature>>>();
        loader.build(definitions).forEach((name,values)->tags.put(TagKey.create(Registries.PLACED_FEATURE,name),List.copyOf(values)));
        placed.prepareTagReload(new TagLoader.LoadResult<>(Registries.PLACED_FEATURE,tags)).apply();
        features.register(ResourceKey.create(Registries.FEATURE,Identifier.parse("horizon:composite")),Feature.DIRECT_CODEC.parse(ops,JsonParser.parseString(bundle.getAsJsonObject("features").get("horizon:composite").getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);
        features.freeze();return ops;
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        final Block[] BLOCKS={Blocks.AIR,Blocks.STONE,Blocks.COBBLESTONE,Blocks.END_STONE,Blocks.OBSIDIAN};
        if(args[0].equals("export")) {
            VanillaBlockStateCodecOracle.main(args);return;
        }
        Path dir=Path.of(args[1]);
        var ops=loadCatalog(dir);
        var contextMethod=VanillaPlacementOracle.class.getDeclaredMethod("context",JsonObject.class);contextMethod.setAccessible(true);
        var bounds=new JsonObject();bounds.add("context",JsonParser.parseString("[-64,384,63]"));
        var generator=((PlacementContext)contextMethod.invoke(null,bounds)).generator();
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();long seed=f.get("seed").getAsLong();
                RandomSource random=f.get("legacy").getAsBoolean()?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                if(f.get("worldgen").getAsBoolean())random=new WorldgenRandom(random);
                Optional<Feature> decoded;
                try {decoded=decodeRoot(ops,f);}catch(IllegalArgumentException|IllegalStateException error){decoded=Optional.empty();}
                var modifiers=new ArrayList<PlacementModifier>();boolean valid=true;
                if(f.has("placement"))for(var json:f.getAsJsonArray("placement")) {
                    try {var modifier=PlacementModifier.CODEC.parse(JsonOps.INSTANCE,json).result();if(modifier.isPresent())modifiers.add(modifier.get());else valid=false;}
                    catch(IllegalArgumentException error){valid=false;}
                }
                if(decoded.isEmpty()||!valid)out.println("decode error");else {
                    out.println("decode ok");int[] calls={0};int fail=f.get("fail").getAsInt(),reject=f.get("reject").getAsInt();
                    var states=new TreeMap<BlockPos,BlockState>(Comparator.<BlockPos>comparingInt(BlockPos::getX).thenComparingInt(BlockPos::getY).thenComparingInt(BlockPos::getZ));
                    var world=(WorldGenLevel)Proxy.newProxyInstance(WorldGenLevel.class.getClassLoader(),new Class<?>[]{WorldGenLevel.class},(proxy,method,values)->{
                        if(method.getName().equals("getMinY"))return -64;
                        if(method.getName().equals("getHeight")&&(values==null||values.length==0))return 384;
                        if(method.getName().equals("getSeaLevel"))return 63;
                        if(Set.of("getMaxY","isInsideBuildHeight","isOutsideBuildHeight","setBlockAndUpdate").contains(method.getName()))return InvocationHandler.invokeDefault(proxy,method,values);
                        var p=((BlockPos)values[0]).immutable();int call=calls[0]++;
                        switch(method.getName()) {
                            case "getBlockState"->{
                                out.println("get "+p.getX()+" "+p.getY()+" "+p.getZ());if(call==fail)throw new Stop();
                                var state=states.getOrDefault(p,BLOCKS[Math.floorMod(p.getX()*31+p.getY()*7+p.getZ()*17,5)].defaultBlockState());out.println("state "+Block.getId(state));return state;
                            }
                            case "setBlock"->{
                                var state=(BlockState)values[1];int flags=(Integer)values[2];out.println("set "+p.getX()+" "+p.getY()+" "+p.getZ()+" "+Block.getId(state)+" "+flags);if(call==fail)throw new Stop();
                                boolean result=reject==0||call%reject!=0;if(result)states.put(p,state);out.println("written "+result);return result;
                            }
                            default->throw new AssertionError("unexpected world method "+method);
                        }
                    });
                    var p=f.getAsJsonArray("origin");var origin=new BlockPos(p.get(0).getAsInt(),p.get(1).getAsInt(),p.get(2).getAsInt());
                    var placed=new PlacedFeature(Holder.direct(decoded.get()),modifiers);var placer=new FeaturePlacer(world,generator);
                    for(int repeat=0;repeat<f.get("repeats").getAsInt();repeat++) {
                        calls[0]=0;
                        try {out.println("place "+(f.has("placement")?placer.place(placed,random,origin):decoded.get().place(world,generator,random,origin)));}catch(Stop|IllegalArgumentException error){out.println("place error");}
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
