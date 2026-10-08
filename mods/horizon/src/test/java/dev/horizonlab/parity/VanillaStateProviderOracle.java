package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.resources.*;
import net.minecraft.util.RandomSource;
import net.minecraft.tags.*;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.level.material.FlowingFluid;
import net.minecraft.world.level.LevelAccessor;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.feature.stateproviders.BlockStateProvider;

/** Native direct/typed state-provider decoding and both sampling methods. */
public final class VanillaStateProviderOracle {
    private static final class Stop extends RuntimeException {}
    @SuppressWarnings({"rawtypes","unchecked"})
    private static void verifyPropertyFamilies() {
        var families=new HashMap<String,net.minecraft.world.level.block.state.properties.Property<?>>();
        for(var block:BuiltInRegistries.BLOCK)for(var property:block.getStateDefinition().getProperties()) {
            var values=new ArrayList<String>();
            for(var value:property.getPossibleValues())values.add(((net.minecraft.world.level.block.state.properties.Property)property).getName((Comparable)value));
            Collections.sort(values);
            String signature=property.getName()+"/"+(property instanceof net.minecraft.world.level.block.state.properties.IntegerProperty)+"/"+values;
            var previous=families.putIfAbsent(signature,property);
            if(previous!=null&&!previous.equals(property))throw new AssertionError("ambiguous pinned property family: "+signature);
        }
    }
    private static void bindFluidTags(JsonObject resources) {
        var registry=BuiltInRegistries.FLUID;
        var loader=new TagLoader<Holder<Fluid>>((id,required)->registry.get(id).map(value->(Holder<Fluid>)value),"tags/fluid");
        var definitions=new HashMap<Identifier,List<TagLoader.EntryWithSource>>();
        for(var item:resources.entrySet()) {
            var file=TagFile.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(item.getValue().getAsString())).getOrThrow();
            var entries=new ArrayList<TagLoader.EntryWithSource>();
            for(var entry:file.entries())entries.add(new TagLoader.EntryWithSource(entry,"vanilla"));
            definitions.put(Identifier.parse(item.getKey()),entries);
        }
        var tags=new HashMap<TagKey<Fluid>,List<Holder<Fluid>>>();
        loader.build(definitions).forEach((id,values)->tags.put(TagKey.create(Registries.FLUID,id),List.copyOf(values)));
        if(tags.size()!=resources.size())throw new AssertionError("incomplete fluid tags");
        registry.prepareTagReload(new TagLoader.LoadResult<>(Registries.FLUID,tags)).apply();
    }
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        if(args[0].equals("export")){VanillaBlockStateCodecOracle.main(args);return;}
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        verifyPropertyFamilies();
        Path dir=Path.of(args[1]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());
        bindFluidTags(JsonParser.parseString(Files.readString(dir.resolve("fluid-tags.json"))).getAsJsonObject());
        var biomeLoad=VanillaMaterialConditionOracle.class.getDeclaredMethod("loadBiomes",Path.class);biomeLoad.setAccessible(true);biomeLoad.invoke(null,dir);
        var biomeField=VanillaMaterialConditionOracle.class.getDeclaredField("biomes");biomeField.setAccessible(true);
        var biomeRegistry=(MappedRegistry<Biome>)biomeField.get(null);
        var biomeNames=new ArrayList<>(new TreeSet<>(biomeRegistry.keySet()));
        var providers=new MappedRegistry<BlockStateProvider>(Registries.BLOCK_STATE_PROVIDER,com.mojang.serialization.Lifecycle.stable());
        var ops=RegistryOps.create(JsonOps.INSTANCE,new RegistryOps.RegistryInfoLookup(){public <T> Optional<HolderGetter<T>> lookup(ResourceKey<? extends Registry<? extends T>> key){
            if(key.equals(Registries.BIOME))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(biomeRegistry);
            if(key.equals(Registries.FLUID))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(BuiltInRegistries.FLUID);
            if(key.equals(Registries.BLOCK_STATE_PROVIDER))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(providers);
            return key.equals(Registries.BLOCK)?(Optional<HolderGetter<T>>)(Optional<?>)Optional.of(BuiltInRegistries.BLOCK):Optional.empty();
        }});
        for(var entry:JsonParser.parseString(Files.readString(dir.resolve("catalog.json"))).getAsJsonObject().entrySet()) {
            var provider=BlockStateProvider.DIRECT_CODEC.parse(ops,JsonParser.parseString(entry.getValue().getAsString())).getOrThrow();
            providers.register(ResourceKey.create(Registries.BLOCK_STATE_PROVIDER,Identifier.parse(entry.getKey())),provider,RegistrationInfo.BUILT_IN);
        }
        providers.freeze();
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8));var inputs=Files.lines(dir.resolve("inputs.txt"))) {
            for(var state:Block.BLOCK_STATE_REGISTRY) {
                var fluid=state.getFluidState();boolean falling=fluid.hasProperty(FlowingFluid.FALLING)&&fluid.getValue(FlowingFluid.FALLING);
                out.println("fluid "+Block.getId(state)+" "+BuiltInRegistries.FLUID.getKey(fluid.getType())+" "+fluid.getAmount()+" "+falling+" "+fluid.isSource()+" "+Integer.toUnsignedString(Float.floatToRawIntBits(fluid.getOwnHeight())));
            }
            for(var line:(Iterable<String>)inputs::iterator) {
                var f=JsonParser.parseString(line).getAsJsonObject();long seed=f.get("seed").getAsLong();
                RandomSource random=f.get("legacy").getAsBoolean()?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                if(f.get("worldgen").getAsBoolean())random=new WorldgenRandom(random);
                Optional<BlockStateProvider> decoded;
                try {
                    var definition=JsonParser.parseString(f.get("definition").getAsString());
                    decoded=f.has("holder")&&f.get("holder").getAsBoolean()?BlockStateProvider.CODEC.parse(ops,definition).result().map(Holder::value):(f.get("typed").getAsBoolean()?BlockStateProvider.TYPED_CODEC:BlockStateProvider.DIRECT_CODEC).parse(ops,definition).result();
                }
                catch(IllegalArgumentException|IllegalStateException error){decoded=Optional.empty();}
                if(decoded.isEmpty())out.println("decode error");else {
                    out.println("decode ok");int[] calls={0};int[] fallback={0};int[] queries={0};int[] biomeIndex={0};
                    int queryFail=f.has("query_fail")?f.get("query_fail").getAsInt():-1;
                    int min=f.has("bounds")?f.getAsJsonArray("bounds").get(0).getAsInt():-64;
                    int height=f.has("bounds")?f.getAsJsonArray("bounds").get(1).getAsInt():384;
                    int sea=f.has("bounds")?f.getAsJsonArray("bounds").get(2).getAsInt():63;
                    var world=(LevelAccessor)Proxy.newProxyInstance(LevelAccessor.class.getClassLoader(),new Class<?>[]{LevelAccessor.class},(proxy,method,values)->{
                        if(method.getName().equals("getFluidState")) {
                            var at=(BlockPos)values[0];out.println("fluid "+at.getX()+" "+at.getY()+" "+at.getZ());
                            if(queries[0]++==queryFail)throw new Stop();return Block.stateById(fallback[0]).getFluidState();
                        }
                        if(method.getName().equals("getRawBrightness")) {
                            var at=(BlockPos)values[0];out.println("light "+at.getX()+" "+at.getY()+" "+at.getZ()+" "+values[1]);
                            if(queries[0]++==queryFail)throw new Stop();return f.has("brightness")?f.get("brightness").getAsInt():15;
                        }
                        if(method.getName().equals("getBlockEntity")) {
                            var p=(BlockPos)values[0];out.println("entity "+p.getX()+" "+p.getY()+" "+p.getZ());
                            if(queries[0]++==queryFail)throw new Stop();return null;
                        }
                        if(method.getName().equals("isUnobstructed")) {
                            if(values[0]!=null)throw new AssertionError("unexpected excluded entity");
                            var box=((net.minecraft.world.phys.shapes.VoxelShape)values[1]).bounds();
                            out.printf("clear %016x %016x %016x %016x %016x %016x%n",Double.doubleToRawLongBits(box.minX),Double.doubleToRawLongBits(box.minY),Double.doubleToRawLongBits(box.minZ),Double.doubleToRawLongBits(box.maxX),Double.doubleToRawLongBits(box.maxY),Double.doubleToRawLongBits(box.maxZ));
                            if(queries[0]++==queryFail)throw new Stop();return !f.has("unobstructed")||f.get("unobstructed").getAsBoolean();
                        }
                        if(method.getName().equals("getBiome")) {
                            var p=(BlockPos)values[0];out.println("biome "+p.getX()+" "+p.getY()+" "+p.getZ());
                            if(queries[0]++==queryFail)throw new Stop();
                            return biomeRegistry.getOrThrow(ResourceKey.create(Registries.BIOME,biomeNames.get(biomeIndex[0])));
                        }
                        if(method.getName().equals("getBlockState")){var p=(BlockPos)values[0];out.println("read "+p.getX()+" "+p.getY()+" "+p.getZ());if(calls[0]++==f.get("fail").getAsInt())throw new Stop();return Block.stateById(fallback[0]);}
                        if(method.getName().equals("getMinY")||method.getName().equals("getHeight")) {
                            boolean minimum=method.getName().equals("getMinY");out.println(minimum?"min":"height");
                            if(queries[0]++==queryFail)throw new Stop();return minimum?min:height;
                        }
                        if(method.getName().equals("getSeaLevel")){out.println("sea");if(queries[0]++==queryFail)throw new Stop();return sea;}
                        if(method.isDefault())return InvocationHandler.invokeDefault(proxy,method,values);
                        throw new AssertionError(method);
                    });
                    int positionIndex=0,sampleIndex=0;
                    for(int repeat=0;repeat<f.get("repeats").getAsInt();repeat++)for(var value:f.getAsJsonArray("states")) {
                        fallback[0]=value.getAsInt();
                        biomeIndex[0]=f.has("biomes")?f.getAsJsonArray("biomes").get(sampleIndex%f.getAsJsonArray("biomes").size()).getAsInt():0;sampleIndex++;
                        var p=new BlockPos(17,repeat-64,-33);
                        if(f.has("positions")){var xyz=f.getAsJsonArray("positions").get(positionIndex++%f.getAsJsonArray("positions").size()).getAsJsonArray();p=new BlockPos(xyz.get(0).getAsInt(),xyz.get(1).getAsInt(),xyz.get(2).getAsInt());}
                        for(boolean optional:new boolean[]{false,true}) {
                            try {var result=optional?decoded.get().getOptionalState(world,random,p):decoded.get().getState(world,random,p);out.println("state "+(result==null?"none":Block.getId(result)));}
                            catch(Stop|IllegalArgumentException|IllegalStateException|IndexOutOfBoundsException error){out.println("state error");}
                            out.println("next "+random.nextLong());
                        }
                    }
                }
                out.println("tail "+random.nextLong());
            }
        }
    }
}
