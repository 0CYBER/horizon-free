package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.function.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.feature.Feature;
import net.minecraft.world.level.levelgen.placement.*;
import net.minecraft.world.level.levelgen.structure.Structure;

/** Actual chunk decoration, with structures disabled and resident block-writing features. */
public final class VanillaDecorationRegionOracle {
    private static final class Stop extends RuntimeException {}
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        if(args[0].equals("export")){VanillaHeightmapOracle.main(args);return;}
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        Path dir=Path.of(args[1]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());
        var contextMethod=VanillaPlacementOracle.class.getDeclaredMethod("context",JsonObject.class);contextMethod.setAccessible(true);
        var settingsConstructor=BiomeGenerationSettings.class.getDeclaredConstructor(HolderSet.class,List.class);settingsConstructor.setAccessible(true);
        var getter=ChunkGenerator.class.getDeclaredField("generationSettingsGetter");getter.setAccessible(true);
        var stepsField=ChunkGenerator.class.getDeclaredField("featuresPerStep");stepsField.setAccessible(true);
        var possibleField=BiomeSource.class.getDeclaredField("possibleBiomes");possibleField.setAccessible(true);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();int min=f.get("min").getAsInt(),height=f.get("height").getAsInt();
                int biomeCount=f.getAsJsonArray("biomes").size(),featureCount=f.getAsJsonArray("placements").size();
                var holders=new ArrayList<Holder<Biome>>();var ids=new IdMapper<Holder<Biome>>();
                for(int i=0;i<=biomeCount;i++){var holder=Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},ResourceKey.create(Registries.BIOME,Identifier.parse("horizon:b"+i)));holders.add(holder);ids.add(holder);}
                var biomeLookup=new HolderGetter<Biome>() {
                    public Optional<Holder.Reference<Biome>> get(ResourceKey<Biome> key) {
                        return holders.stream().filter(h->h.unwrapKey().orElseThrow().equals(key)).map(h->(Holder.Reference<Biome>)h).findFirst();
                    }
                    public Optional<HolderSet.Named<Biome>> get(net.minecraft.tags.TagKey<Biome> key) {return Optional.empty();}
                };
                var ops=RegistryOps.create(JsonOps.INSTANCE,new RegistryOps.RegistryInfoLookup(){public <T> Optional<HolderGetter<T>> lookup(ResourceKey<? extends Registry<? extends T>> key){
                    return key.equals(Registries.BIOME)?(Optional<HolderGetter<T>>)(Optional<?>)Optional.of(biomeLookup):Optional.empty();
                }});
                var bounds=new JsonObject();bounds.add("context",JsonParser.parseString("[-64,384,63]"));
                var generator=((PlacementContext)contextMethod.invoke(null,bounds)).generator();
                var placed=new MappedRegistry<PlacedFeature>(Registries.PLACED_FEATURE,Lifecycle.stable());
                var structures=new MappedRegistry<Structure>(Registries.STRUCTURE,Lifecycle.stable());structures.freeze();
                int[] calls={0},queries={0},notifications={0},chunksRead={0};
                var features=new ArrayList<PlacedFeature>();
                for(int i=0;i<featureCount;i++) {
                    final int id=i;
                    var leaf=Feature.DIRECT_CODEC.parse(JsonOps.INSTANCE,f.getAsJsonArray("features").get(i)).getOrThrow();
                    var modifiers=new ArrayList<PlacementModifier>();for(var value:f.getAsJsonArray("placements").get(i).getAsJsonArray())modifiers.add(PlacementModifier.CODEC.parse(ops,value).getOrThrow());
                    var root=new PlacedFeature(Holder.direct(leaf),modifiers);features.add(root);
                    placed.register(ResourceKey.create(Registries.PLACED_FEATURE,Identifier.parse("horizon:f"+i)),root,RegistrationInfo.BUILT_IN);
                }
                placed.freeze();
                var settings=new ArrayList<BiomeGenerationSettings>();
                for(var value:f.getAsJsonArray("biomes")) {
                    var steps=new ArrayList<HolderSet<PlacedFeature>>();
                    for(var step:value.getAsJsonArray()){var roots=new ArrayList<Holder<PlacedFeature>>();for(var root:step.getAsJsonArray())roots.add(Holder.direct(features.get(root.getAsInt())));steps.add(HolderSet.direct(roots));}
                    settings.add(settingsConstructor.newInstance(HolderSet.direct(List.of()),steps));
                }
                settings.add(settingsConstructor.newInstance(HolderSet.direct(List.of()),List.of()));
                Function<Holder<Biome>,BiomeGenerationSettings> settingsGetter=biome->settings.get(ids.getId(biome));getter.set(generator,settingsGetter);
                var possible=new LinkedHashSet<Holder<Biome>>(holders.subList(0,biomeCount));possibleField.set(generator.getBiomeSource(),(Supplier<Set<Holder<Biome>>>)(()->possible));
                List<FeatureSorter.StepFeatureData> sorted;
                try {sorted=FeatureSorter.buildFeaturesPerStep(new ArrayList<>(possible),b->settingsGetter.apply(b).features(),true);}
                catch(IllegalStateException error){out.println("plan error");continue;}
                stepsField.set(generator,(Supplier<List<FeatureSorter.StepFeatureData>>)(()->sorted));out.println("plan ok");
                var factory=new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),Blocks.AIR.defaultBlockState(),null,Strategy.createForBiomes(ids),holders.get(biomeCount),null);
                var cp=new ChunkPos(f.get("cx").getAsInt(),f.get("cz").getAsInt());var chunks=new TreeMap<ChunkPos,ProtoChunk>(Comparator.<ChunkPos>comparingInt(p->p.x()).thenComparingInt(p->p.z()));
                var palettes=f.getAsJsonArray("palettes");int ordinal=0;
                for(int dz=-1;dz<=1;dz++)for(int dx=-1;dx<=1;dx++) {
                    var pos=new ChunkPos(cp.x()+dx,cp.z()+dz);var chunk=new ProtoChunk(pos,UpgradeData.EMPTY,LevelHeightAccessor.create(min,height),factory,null);
                    chunk.setPersistedStatus(switch(f.get("stage").getAsString()){case "terrain"->net.minecraft.world.level.chunk.status.ChunkStatus.TERRAIN;case "biomes"->net.minecraft.world.level.chunk.status.ChunkStatus.BIOMES;default->net.minecraft.world.level.chunk.status.ChunkStatus.EMPTY;});
                    for(int x=0;x<16;x++)for(int z=0;z<16;z++) {
                        chunk.setBlockState(new BlockPos((pos.x()<<4)+x,-56,(pos.z()<<4)+z),Blocks.STONE.defaultBlockState(),3);
                        if((x+z)%3!=0)chunk.setBlockState(new BlockPos((pos.x()<<4)+x,-48,(pos.z()<<4)+z),Blocks.DIRT.defaultBlockState(),3);
                    }
                    final var values=palettes.get(ordinal++).getAsJsonArray();
                    chunk.fillBiomesFromNoise((x,y,z)->holders.get(values.get(Math.floorMod((y-(min>>2))*16+(z&3)*4+(x&3),values.size())).getAsInt()));chunks.put(pos,chunk);
                }
                var zoom=new BiomeManager((x,y,z)->{var chunk=chunks.get(new ChunkPos(x>>2,z>>2));if(chunk==null)throw new Stop();return chunk.getNoiseBiome(x,y,z);},BiomeManager.obfuscateSeed(f.get("seed").getAsLong()));
                var access=new RegistryAccess.ImmutableRegistryAccess(List.of(structures,placed)).freeze();
                var world=(WorldGenLevel)Proxy.newProxyInstance(WorldGenLevel.class.getClassLoader(),new Class<?>[]{WorldGenLevel.class},(proxy,method,values)->switch(method.getName()) {
                    case "getMinY"->min;case "getSeaLevel"->63;
                    case "getHeight"->{if(values==null||values.length==0)yield height;var kind=(Heightmap.Types)values[0];int x=(Integer)values[1],z=(Integer)values[2];var chunk=chunks.get(new ChunkPos(x>>4,z>>4));if(chunk==null)throw new Stop();yield chunk.getHeight(kind,x&15,z&15)+1;}
                    case "getMaxY","isInsideBuildHeight","isOutsideBuildHeight","setBlockAndUpdate"->InvocationHandler.invokeDefault(proxy,method,values);
                    case "getBlockState"->{var p=(BlockPos)values[0];var chunk=chunks.get(new ChunkPos(p.getX()>>4,p.getZ()>>4));if(chunk==null)throw new Stop();yield chunk.getBlockState(p);}
                    case "setBlock"->{var p=(BlockPos)values[0];var state=(net.minecraft.world.level.block.state.BlockState)values[1];int flags=(Integer)values[2];out.println("write "+p.getX()+" "+p.getY()+" "+p.getZ()+" "+Block.getId(state)+" "+flags);int call=calls[0]++;if(call==f.get("write_fail").getAsInt())throw new Stop();int reject=f.get("reject").getAsInt();boolean result=reject==0||call%reject!=0;if(result){var chunk=chunks.get(new ChunkPos(p.getX()>>4,p.getZ()>>4));if(chunk==null)throw new Stop();chunk.setBlockState(p,state,flags);}out.println("written "+result);yield result;}
                    case "getMinSectionY"->InvocationHandler.invokeDefault(proxy,method,values);
                    case "getSeed"->f.get("seed").getAsLong();case "registryAccess"->access;
                    case "getChunk"->{int x=(Integer)values[0],z=(Integer)values[1];out.println("chunk "+x+" "+z);if(chunksRead[0]++==f.get("chunk_fail").getAsInt())throw new Stop();var chunk=chunks.get(new ChunkPos(x,z));if(chunk==null)throw new AssertionError("missing chunk");yield chunk;}
                    case "setCurrentlyGenerating"->{String label=values[0]==null?"none":((Supplier<?>)values[0]).get().toString();out.println("generating "+label);if(notifications[0]++==f.get("notify_fail").getAsInt())throw new Stop();yield null;}
                    case "getBiome"->{var p=(BlockPos)values[0];var biome=zoom.getBiome(p);out.println("biome "+p.getX()+" "+p.getY()+" "+p.getZ()+" "+ids.getId(biome));yield biome;}
                    default->throw new AssertionError(method);
                });
                var manager=new StructureManager(world,new WorldOptions(f.get("seed").getAsLong(),false,false),null);
                for(int repeat=0;repeat<2;repeat++) {
                    calls[0]=queries[0]=notifications[0]=chunksRead[0]=0;
                    try {generator.applyBiomeDecoration(world,chunks.get(cp),manager);out.println("decorate ok");}
                    catch(Stop|net.minecraft.ReportedException error){out.println("decorate error");}
                    for(int[] p:new int[][]{{0,0},{15,15},{16,16},{-1,-1}})for(var kind:Heightmap.Types.values())out.println("height "+kind.name()+" "+p[0]+" "+p[1]+" "+(chunks.get(new ChunkPos(p[0]>>4,p[1]>>4)).getHeight(kind,p[0]&15,p[1]&15)+1));
                }
                for(var entry:chunks.entrySet()) {
                    var pos=entry.getKey();var chunk=entry.getValue();out.println("stored "+pos.x()+" "+pos.z());
                    for(var kind:Heightmap.Types.values()) {
                        if(chunk.hasPrimedHeightmap(kind)){out.println("map "+kind.name());for(long word:chunk.getOrCreateHeightmapUnprimed(kind).getRawData())out.println(Long.toUnsignedString(word,16));}
                        else out.println("missing "+kind.name());
                    }
                    for(int i=0;i<height/16;i++){out.println("section "+i);var section=chunk.getSection(i);for(int y=0;y<16;y++)for(int z=0;z<16;z++)for(int x=0;x<16;x++)out.println(Block.getId(section.getBlockState(x,y,z)));}
                }
            }
        }
    }
}
