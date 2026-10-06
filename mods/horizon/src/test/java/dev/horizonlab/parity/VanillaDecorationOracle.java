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

/** Actual chunk decoration, with structures disabled and traced feature sinks. */
public final class VanillaDecorationOracle {
    private static final class Stop extends RuntimeException {}
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        Path dir=Path.of(args[1]);
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
                var bounds=new JsonObject();bounds.add("context",JsonParser.parseString("[-64,384,63]"));
                var generator=((PlacementContext)contextMethod.invoke(null,bounds)).generator();
                var placed=new MappedRegistry<PlacedFeature>(Registries.PLACED_FEATURE,Lifecycle.stable());
                var structures=new MappedRegistry<Structure>(Registries.STRUCTURE,Lifecycle.stable());structures.freeze();
                int[] calls={0},queries={0},notifications={0},chunksRead={0};
                var features=new ArrayList<PlacedFeature>();
                for(int i=0;i<featureCount;i++) {
                    final int id=i;
                    var leaf=new Feature(){
                        public MapCodec<? extends Feature> codec(){return MapCodec.unit(this);}
                        public boolean place(WorldGenLevel w,ChunkGenerator g,RandomSource r,BlockPos p){
                            out.println("feature "+id+" "+p.getX()+" "+p.getY()+" "+p.getZ());
                            out.println("draw "+r.nextLong()+" "+r.nextInt(17)+" "+Integer.toHexString(Float.floatToRawIntBits(r.nextFloat()))+" "+Long.toHexString(Double.doubleToRawLongBits(r.nextDouble())));
                            if(calls[0]++==f.get("feature_fail").getAsInt())throw new Stop();
                            return f.get("return_mode").getAsInt()==0||(f.get("return_mode").getAsInt()==2&&id%2==0);
                        }
                    };
                    var modifiers=new ArrayList<PlacementModifier>();for(var value:f.getAsJsonArray("placements").get(i).getAsJsonArray())modifiers.add(PlacementModifier.CODEC.parse(JsonOps.INSTANCE,value).getOrThrow());
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
                var cp=new ChunkPos(f.get("cx").getAsInt(),f.get("cz").getAsInt());var chunks=new HashMap<ChunkPos,ProtoChunk>();
                var palettes=f.getAsJsonArray("palettes");int ordinal=0;
                for(int dz=-1;dz<=1;dz++)for(int dx=-1;dx<=1;dx++) {
                    var pos=new ChunkPos(cp.x()+dx,cp.z()+dz);var chunk=new ProtoChunk(pos,UpgradeData.EMPTY,LevelHeightAccessor.create(min,height),factory,null);
                    final var values=palettes.get(ordinal++).getAsJsonArray();
                    chunk.fillBiomesFromNoise((x,y,z)->holders.get(values.get(Math.floorMod((y-(min>>2))*16+(z&3)*4+(x&3),values.size())).getAsInt()));chunks.put(pos,chunk);
                }
                var access=new RegistryAccess.ImmutableRegistryAccess(List.of(structures,placed)).freeze();
                var world=(WorldGenLevel)Proxy.newProxyInstance(WorldGenLevel.class.getClassLoader(),new Class<?>[]{WorldGenLevel.class},(proxy,method,values)->switch(method.getName()) {
                    case "getMinY"->min;case "getHeight"->height;case "getSeaLevel"->63;
                    case "getMinSectionY"->InvocationHandler.invokeDefault(proxy,method,values);
                    case "getSeed"->f.get("seed").getAsLong();case "registryAccess"->access;
                    case "getChunk"->{int x=(Integer)values[0],z=(Integer)values[1];out.println("chunk "+x+" "+z);if(chunksRead[0]++==f.get("chunk_fail").getAsInt())throw new Stop();var chunk=chunks.get(new ChunkPos(x,z));if(chunk==null)throw new AssertionError("missing chunk");yield chunk;}
                    case "setCurrentlyGenerating"->{String label=values[0]==null?"none":((Supplier<?>)values[0]).get().toString();out.println("generating "+label);if(notifications[0]++==f.get("notify_fail").getAsInt())throw new Stop();yield null;}
                    case "getBiome"->{var p=(BlockPos)values[0];int id=Math.floorMod(p.getX()*31+p.getY()*7+p.getZ()*17+f.get("biome_offset").getAsInt(),biomeCount+1);out.println("biome "+p.getX()+" "+p.getY()+" "+p.getZ()+" "+id);if(queries[0]++==f.get("query_fail").getAsInt())throw new Stop();yield holders.get(id);}
                    default->throw new AssertionError(method);
                });
                var manager=new StructureManager(world,new WorldOptions(f.get("seed").getAsLong(),false,false),null);
                for(int repeat=0;repeat<2;repeat++) {
                    calls[0]=queries[0]=notifications[0]=chunksRead[0]=0;
                    try {generator.applyBiomeDecoration(world,chunks.get(cp),manager);out.println("decorate ok");}
                    catch(Stop|net.minecraft.ReportedException error){out.println("decorate error");}
                }
            }
        }
    }
}
