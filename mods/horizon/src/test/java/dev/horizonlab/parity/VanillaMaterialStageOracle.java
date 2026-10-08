package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.lang.reflect.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.resources.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.level.block.state.properties.IntegerProperty;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.blending.Blender;
import net.minecraft.world.level.levelgen.densityfunction.*;
import net.minecraft.world.level.levelgen.material.rule.MaterialRule;
import net.minecraft.world.level.levelgen.synth.NormalNoise;

/** Original transport calling the actual MaterialSystem.buildSurface on ProtoChunk. */
public final class VanillaMaterialStageOracle {
    private static MappedRegistry<NormalNoise> noises;
    private static MappedRegistry<Biome> biomes;
    private static RegistryOps<JsonElement> ops;
    private static PalettedContainerFactory factory;
    private static Object field(String name) throws Exception {var f=VanillaMaterialConditionOracle.class.getDeclaredField(name);f.setAccessible(true);return f.get(null);}
    @SuppressWarnings("unchecked")
    private static void load(Path dir) throws Exception {
        noises=new MappedRegistry<>(Registries.NOISE,Lifecycle.stable());
        var data=JsonParser.parseString(Files.readString(dir.resolve("noises.json"))).getAsJsonObject();
        for(var e:data.entrySet())noises.register(ResourceKey.create(Registries.NOISE,Identifier.parse(e.getKey())),NormalNoise.DIRECT_CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(e.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);noises.freeze();
        var field=VanillaMaterialConditionOracle.class.getDeclaredField("noises");field.setAccessible(true);field.set(null,noises);
        for(var name:new String[]{"loadBiomes","loadConditions","loadRules"}){var method=VanillaMaterialConditionOracle.class.getDeclaredMethod(name,Path.class);method.setAccessible(true);method.invoke(null,dir);}
        biomes=(MappedRegistry<Biome>)field("biomes");ops=(RegistryOps<JsonElement>)field("ops");
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("block_tags.json"))).getAsJsonObject());
        var ids=new IdMapper<Holder<Biome>>();for(var id:biomes.keySet())ids.add(biome(id.toString()));
        factory=new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),Blocks.AIR.defaultBlockState(),null,Strategy.createForBiomes(ids),biome("minecraft:plains"),null);
        var carvers=new MappedRegistry<net.minecraft.world.level.levelgen.carver.WorldCarver>(Registries.CARVER,Lifecycle.stable());
        var carverJson=JsonParser.parseString(Files.readString(dir.resolve("carvers.json"))).getAsJsonObject();
        for(var e:carverJson.entrySet())carvers.register(ResourceKey.create(Registries.CARVER,Identifier.parse(e.getKey())),net.minecraft.world.level.levelgen.carver.WorldCarver.DIRECT_CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(e.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);carvers.freeze();
        var carverOps=RegistryOps.create(JsonOps.INSTANCE,new RegistryOps.RegistryInfoLookup(){public <T> Optional<HolderGetter<T>> lookup(ResourceKey<? extends Registry<? extends T>> key){if(key.equals(Registries.CARVER))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(carvers);return Optional.empty();}});
        var generationField=Biome.class.getDeclaredField("generationSettings");generationField.setAccessible(true);var biomeJson=JsonParser.parseString(Files.readString(dir.resolve("biomes.json"))).getAsJsonObject();
        for(var e:biomeJson.entrySet()) {
            var config=new JsonObject();config.add("carvers",JsonParser.parseString(e.getValue().getAsString()).getAsJsonObject().get("carvers"));config.add("features",new JsonArray());
            generationField.set(biome(e.getKey()).value(),BiomeGenerationSettings.CODEC.codec().parse(carverOps,config).getOrThrow());
        }
    }
    @SuppressWarnings("unchecked")
    private static RegistryOps<JsonElement> biomeOps() {
        var presets=new MappedRegistry<MultiNoiseBiomeSourceParameterList>(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST,Lifecycle.stable());
        for(var preset:List.of(MultiNoiseBiomeSourceParameterList.Preset.OVERWORLD,MultiNoiseBiomeSourceParameterList.Preset.NETHER))presets.register(ResourceKey.create(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST,preset.id()),new MultiNoiseBiomeSourceParameterList(preset,biomes),RegistrationInfo.BUILT_IN);presets.freeze();
        return RegistryOps.create(JsonOps.INSTANCE,new RegistryOps.RegistryInfoLookup(){public <T> Optional<HolderGetter<T>> lookup(ResourceKey<? extends Registry<? extends T>> key){
            if(key.equals(Registries.BIOME))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(biomes);
            if(key.equals(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(presets);return Optional.empty();}});
    }
    private static Holder<Biome> biome(String name){return biomes.getOrThrow(ResourceKey.create(Registries.BIOME,Identifier.parse(name)));}
    private static void export(Path path) throws Exception {
        var method=VanillaHeightmapOracle.class.getDeclaredMethod("facts");method.setAccessible(true);var facts=(JsonObject)method.invoke(null);
        var integers=new JsonArray();for(var id:new TreeSet<>(BuiltInRegistries.BLOCK.keySet()))for(var p:BuiltInRegistries.BLOCK.getValue(id).getStateDefinition().getProperties())if(p instanceof IntegerProperty){var a=new JsonArray();a.add(id.toString());a.add(p.getName());integers.add(a);}
        facts.add("integers",integers);Files.writeString(path,facts.toString(),StandardCharsets.UTF_8);
    }
    private static void fixture(JsonObject f,PrintWriter out) throws Exception {
        int cx=f.get("cx").getAsInt(),cz=f.get("cz").getAsInt(),min=f.get("min").getAsInt(),height=f.get("height").getAsInt(),sea=f.get("sea").getAsInt();
        long seed=f.get("seed").getAsLong();boolean legacy=f.get("legacy").getAsBoolean();
        var rule=MaterialRule.CODEC.parse(ops,new JsonPrimitive(f.get("rule").getAsString())).getOrThrow();
        var routerJson=new JsonObject();for(var name:new String[]{"chunk_surface_level","continents","depth","erosion","final_density","ridges","temperature","vegetation"})routerJson.addProperty(name,0);routerJson.add("chunk_surface_level",f.get("preliminary"));
        if(f.has("router"))routerJson=f.getAsJsonObject("router");
        var router=NoiseRouter.CODEC.parse(ops,routerJson).getOrThrow();var defaultBlock=Block.stateById(f.get("default").getAsInt());
        Optional<Aquifer.Config> aquifers=f.has("aquifers")&&!f.get("aquifers").isJsonNull()?Optional.of(Aquifer.Config.CODEC.parse(ops,f.get("aquifers")).getOrThrow()):Optional.empty();
        var fluid=f.has("fluid")?Block.stateById(f.get("fluid").getAsInt()):Blocks.WATER.defaultBlockState();
        var settings=new NoiseGeneratorSettings(NoiseSettings.create(f.get("noise_min").getAsInt(),f.get("noise_height").getAsInt()),defaultBlock,fluid,router,Holder.direct(rule),List.of(),sea,false,aquifers,legacy,NoiseGeneratorSettings.DebugFunctions.EMPTY);
        var random=RandomState.create(noises,seed,legacy,defaultBlock,sea,router);
        var bounds=LevelHeightAccessor.create(f.get("write_min").getAsInt(),f.get("write_max").getAsInt()-f.get("write_min").getAsInt()+1);
        var chunk=new ProtoChunk(new ChunkPos(cx,cz),UpgradeData.EMPTY,LevelHeightAccessor.create(min,height),factory,null){@Override public LevelHeightAccessor getHeightAccessorForGeneration(){return bounds;}};
        chunk.setPersistedStatus(ChunkStatus.BIOMES);
        for(var value:f.getAsJsonArray("initial")){var a=value.getAsJsonArray();int x=a.get(0).getAsInt(),z=a.get(2).getAsInt();for(int y=a.get(1).getAsInt();y<=a.get(3).getAsInt();y++)chunk.getSection(chunk.getSectionIndex(y)).setBlockState(x&15,y&15,z&15,Block.stateById(a.get(4).getAsInt()),false);}
        if(f.get("prime").getAsBoolean())Heightmap.primeHeightmaps(chunk,EnumSet.of(Heightmap.Types.WORLD_SURFACE_WG,Heightmap.Types.OCEAN_FLOOR_WG));
        var neighborhood=new HashMap<Long,ProtoChunk>();
        BiomeSource source=null;
        if(f.has("biome_source")) {
            source=BiomeSource.CODEC.parse(biomeOps(),f.get("biome_source_json")).getOrThrow();
            var generator=new NoiseBasedChunkGenerator(source,Holder.direct(settings));
            var create=ChunkGenerator.class.getDeclaredMethod("doCreateBiomes",Blender.class,RandomState.class,ChunkAccess.class);create.setAccessible(true);
            for(int dx=-1;dx<=1;dx++)for(int dz=-1;dz<=1;dz++) {
                int x=cx+dx,z=cz+dz;var neighbor=dx==0&&dz==0?chunk:new ProtoChunk(new ChunkPos(x,z),UpgradeData.EMPTY,LevelHeightAccessor.create(min,height),factory,null);
                neighbor.setPersistedStatus(ChunkStatus.BIOMES);create.invoke(generator,Blender.empty(),random,neighbor);
                out.println("biomechunk "+x+" "+z);
                for(int section=0;section<height/16;section++){var data=neighbor.getSection(section);for(int y=0;y<4;y++)for(int lz=0;lz<4;lz++)for(int lx=0;lx<4;lx++)out.println("stored "+biomes.getKey(data.getNoiseBiome(lx,y,lz).value()));}
                neighborhood.put(ChunkPos.pack(x,z),neighbor);
            }
        }
        var names=f.getAsJsonArray("biome_choices");boolean zoom=f.has("biome_zoom")&&f.get("biome_zoom").getAsBoolean();
        var manager=new BiomeManager((x,y,z)->{
            Holder<Biome> value;
            if(!neighborhood.isEmpty())value=neighborhood.get(ChunkPos.pack(x>>2,z>>2)).getNoiseBiome(x,y,z);
            else {int hash=x*31+y*7+z*17;value=biome(names.get(Math.floorMod(hash,names.size())).getAsString());}
            out.println("quart "+x+" "+y+" "+z+" "+biomes.getKey(value.value()));return value;
        },BiomeManager.obfuscateSeed(seed)) {
            @Override public Holder<Biome> getBiome(BlockPos pos) {
                Holder<Biome> value;
                if(zoom)value=super.getBiome(pos);
                else {int hash=pos.getX()*31+pos.getY()*7+pos.getZ()*17;value=biome(names.get(Math.floorMod(hash,names.size())).getAsString());}
                out.println("biome "+pos.getX()+" "+pos.getY()+" "+pos.getZ()+" "+biomes.getKey(value.value()));return value;
            }
        };
        Set<Holder<Biome>> possible=null;if(!f.get("possible").isJsonNull()){possible=new HashSet<>();for(var name:f.getAsJsonArray("possible"))possible.add(biome(name.getAsString()));}
        var contextInput=new JsonObject();var contextArray=new JsonArray();contextArray.add(min);contextArray.add(height);contextArray.add(sea);contextInput.add("context",contextArray);
        var contextMethod=VanillaAnchorOracle.class.getDeclaredMethod("context",JsonObject.class);contextMethod.setAccessible(true);var context=(WorldGenerationContext)contextMethod.invoke(null,contextInput);
        var volume=new DensityVolume(16,f.get("noise_height").getAsInt(),16,cx<<4,f.get("noise_min").getAsInt(),cz<<4);
        var pickerMethod=NoiseBasedChunkGenerator.class.getDeclaredMethod("createFluidPicker",NoiseGeneratorSettings.class);pickerMethod.setAccessible(true);
        var picker=(Aquifer.FluidPicker)pickerMethod.invoke(null,settings);
        try(var noise=new NoiseChunk(random,Beardifier.EMPTY,settings,picker,Blender.empty(),volume)) {
            if(f.has("router")) {
                var generator=new NoiseBasedChunkGenerator(new FixedBiomeSource(biome(names.get(0).getAsString())),Holder.direct(settings));
                var fill=NoiseBasedChunkGenerator.class.getDeclaredMethod("doFill",NoiseChunk.class,ChunkAccess.class);fill.setAccessible(true);fill.invoke(generator,noise,chunk);
                out.println("terrain "+noise.aquifer().shouldScheduleFluidUpdate());dump(chunk,height,out);
            }
            for(int repeat=0;repeat<f.get("repeats").getAsInt();repeat++) {
                try{random.surfaceSystem().buildSurface(random,manager,context,chunk,noise,rule,possible);out.println("surface "+repeat+" ok");}catch(RuntimeException error){out.println("surface "+repeat+" error");error.printStackTrace();}
                dump(chunk,height,out);
            }
            for(var query:f.getAsJsonArray("top_queries")) {
                var q=query.getAsJsonArray();var pos=new BlockPos(q.get(0).getAsInt(),q.get(1).getAsInt(),q.get(2).getAsInt());
                try {
                    var result=random.surfaceSystem().topMaterial(rule,random,context,manager::getBiome,chunk,noise.cachingSamplers(),pos,q.get(3).getAsBoolean());
                    out.println("top "+(result.isPresent()?Integer.toString(Block.getId(result.get())):"none"));
                }catch(RuntimeException error){out.println("top error");error.printStackTrace();}
            }
            var delegate=noise.aquifer();var traced=new Aquifer() {
                public net.minecraft.world.level.block.state.BlockState computeSubstance(int x,int y,int z,double density) {
                    out.println("aquifer "+x+" "+y+" "+z+" "+Long.toHexString(Double.doubleToRawLongBits(density)));
                    var result=delegate.computeSubstance(x,y,z,density);out.println("substance "+(result==null?"none":Integer.toString(Block.getId(result))));return result;
                }
                public boolean shouldScheduleFluidUpdate(){boolean result=delegate.shouldScheduleFluidUpdate();out.println("fluid_schedule "+result);return result;}
            };
            var aquiferField=NoiseChunk.class.getDeclaredField("aquifer");aquiferField.setAccessible(true);aquiferField.set(noise,traced);
            if(source!=null) {
                var delegateSource=source;
                var tracedSource=new BiomeSource() {
                    protected com.mojang.serialization.MapCodec<? extends BiomeSource> codec(){return com.mojang.serialization.MapCodec.unit(delegateSource);}
                    protected java.util.stream.Stream<Holder<Biome>> collectPossibleBiomes(){return delegateSource.possibleBiomes().stream();}
                    public BiomeResolver createResolver(Climate.Sampler sampler) {
                        var resolver=delegateSource.createResolver(sampler);
                        return (x,y,z)->{var value=resolver.getNoiseBiome(x,y,z);out.println("rawquart "+x+" "+y+" "+z+" "+biomes.getKey(value.value()));return value;};
                    }
                };
                var generator=new NoiseBasedChunkGenerator(tracedSource,Holder.direct(settings));
                var generate=NoiseBasedChunkGenerator.class.getDeclaredMethod("generateCarvers",ChunkAccess.class,Blender.class,NoiseChunk.class,RandomState.class,BiomeManager.class,net.minecraft.server.level.WorldGenRegion.class,MaterialRule.class);generate.setAccessible(true);
                try {generate.invoke(generator,chunk,Blender.empty(),noise,random,manager,null,rule);out.println("generatedcarving ok");}
                catch(java.lang.reflect.InvocationTargetException error){out.println("generatedcarving error");error.getCause().printStackTrace();}
                dump(chunk,height,out);
            }
            var carving=f.getAsJsonObject("carving");
            for(var value:carving.getAsJsonArray("initial")) {var q=value.getAsJsonArray();chunk.setBlockState(new BlockPos(q.get(0).getAsInt(),q.get(1).getAsInt(),q.get(2).getAsInt()),Block.stateById(q.get(3).getAsInt()));}
            var mask=new CarvingMask(min+1,min+height-8);
            for(var value:carving.getAsJsonArray("spans")) {var q=value.getAsJsonArray();for(int y=q.get(2).getAsInt();y<=q.get(3).getAsInt();y++)mask.carve(q.get(0).getAsInt(),y,q.get(1).getAsInt());}
            var generator=new NoiseBasedChunkGenerator(new FixedBiomeSource(biome(names.get(0).getAsString())),Holder.direct(settings));
            var apply=NoiseBasedChunkGenerator.class.getDeclaredMethod("applyCarvingMask",ChunkAccess.class,CarvingMask.class,RandomState.class,MaterialRule.class,WorldGenerationContext.class,NoiseChunk.class,java.util.function.Function.class,CarvingMask.Filter.class);apply.setAccessible(true);
            for(var value:carving.getAsJsonArray("filters")) {
                int mode=value.getAsInt();CarvingMask.Filter filter=mode==0?null:(x,y,z)->{boolean keep=mode==1&&Math.floorMod(x*31+y*7+z*17,5)!=0;out.println("filter "+x+" "+y+" "+z+" "+keep);return keep;};
                try {apply.invoke(generator,chunk,mask,random,rule,context,noise,(java.util.function.Function<BlockPos,Holder<Biome>>)manager::getBiome,filter);out.println("carving "+mode+" ok");}
                catch(java.lang.reflect.InvocationTargetException error){out.println("carving "+mode+" error");error.getCause().printStackTrace();}
                dump(chunk,height,out);
            }
        }
    }
    private static void dump(ProtoChunk chunk,int height,PrintWriter out) {
                for(int section=0;section<height/16;section++){out.println("section "+section);var data=chunk.getSection(section);for(int y=0;y<16;y++)for(int z=0;z<16;z++){out.print("blocks");for(int x=0;x<16;x++)out.print(" "+Block.getId(data.getBlockState(x,y,z)));out.println();}}
                for(var type:new Heightmap.Types[]{Heightmap.Types.WORLD_SURFACE_WG,Heightmap.Types.OCEAN_FLOOR_WG}) {out.println("heightmap "+type);for(long word:chunk.getOrCreateHeightmapUnprimed(type).getRawData())out.println(Long.toUnsignedString(word,16));}
                var offsets=chunk.getPostProcessing();for(int section=0;section<offsets.length;section++){var list=offsets[section];out.println("post "+section+" "+(list==null?0:list.size()));if(list!=null)for(short value:list)out.println(Short.toUnsignedInt(value));}
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);if(args[0].equals("export")){export(dir);return;}load(dir);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))){for(var line:Files.readAllLines(dir.resolve("inputs.txt")))fixture(JsonParser.parseString(line).getAsJsonObject(),out);}
    }
}
