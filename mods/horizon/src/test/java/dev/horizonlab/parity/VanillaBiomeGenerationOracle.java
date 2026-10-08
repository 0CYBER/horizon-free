package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.resources.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.blending.Blender;
import net.minecraft.world.level.levelgen.densityfunction.*;
import net.minecraft.world.level.levelgen.material.rule.MaterialRule;
import net.minecraft.world.level.levelgen.synth.NormalNoise;

/** Original transport invoking actual ChunkGenerator.doCreateBiomes. */
public final class VanillaBiomeGenerationOracle {
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
        var bundle=JsonParser.parseString(Files.readString(dir.resolve("registries.json"))).getAsJsonObject();
        var noises=new MappedRegistry<NormalNoise>(Registries.NOISE,Lifecycle.stable());var densities=new MappedRegistry<DensityFunction>(Registries.DENSITY_FUNCTION,Lifecycle.stable());
        var noiseLookup=noises.createRegistrationLookup();var densityLookup=densities.createRegistrationLookup();
        var ops=RegistryOps.create(JsonOps.INSTANCE,new RegistryOps.RegistryInfoLookup(){public <T> Optional<HolderGetter<T>> lookup(ResourceKey<? extends Registry<? extends T>> key){
            if(key.equals(Registries.NOISE))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(noiseLookup);
            if(key.equals(Registries.DENSITY_FUNCTION))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(densityLookup);return Optional.empty();}});
        for(var e:bundle.getAsJsonObject("noises").entrySet())noises.register(ResourceKey.create(Registries.NOISE,Identifier.parse(e.getKey())),NormalNoise.DIRECT_CODEC.parse(ops,JsonParser.parseString(e.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);noises.freeze();
        for(var e:bundle.getAsJsonObject("densities").entrySet())densities.register(ResourceKey.create(Registries.DENSITY_FUNCTION,Identifier.parse(e.getKey())),DensityFunctions.DIRECT_CODEC.parse(ops,JsonParser.parseString(e.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);densities.freeze();
        var presets=new TreeMap<String,Climate.ParameterList<ResourceKey<Biome>>>();MultiNoiseBiomeSourceParameterList.knownPresets().forEach((preset,list)->presets.put(preset.id().toString(),list));
        var ids=new IdMapper<Holder<Biome>>();var holders=new TreeMap<String,Holder<Biome>>();
        var keys=new TreeSet<String>();keys.add("minecraft:plains");keys.addAll(List.of("horizon:old_0","horizon:old_1","horizon:old_2"));keys.addAll(List.of("minecraft:the_end","minecraft:end_highlands","minecraft:end_midlands","minecraft:small_end_islands","minecraft:end_barrens"));for(var list:presets.values())for(var entry:list.values())keys.add(entry.getSecond().identifier().toString());
        for(var key:keys){var holder=Holder.Reference.<Biome>createStandAlone(new HolderOwner<Biome>(){},ResourceKey.create(Registries.BIOME,Identifier.parse(key)));holders.put(key,holder);ids.add(holder);}
        var factory=new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),Blocks.AIR.defaultBlockState(),null,Strategy.createForBiomes(ids),holders.get("minecraft:plains"),null);
        var fill=ChunkGenerator.class.getDeclaredMethod("doCreateBiomes",Blender.class,RandomState.class,ChunkAccess.class);fill.setAccessible(true);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();int min=f.get("min").getAsInt(),height=f.get("height").getAsInt(),sea=63;long seed=f.get("seed").getAsLong();boolean legacy=f.get("legacy").getAsBoolean();
                var setting=JsonParser.parseString(bundle.getAsJsonObject("settings").get(f.get("setting").getAsString()).getAsString()).getAsJsonObject();var router=NoiseRouter.CODEC.parse(ops,setting.get("noise_router")).getOrThrow();
                var state=RandomState.create(noises,seed,legacy,Blocks.STONE.defaultBlockState(),sea,router);
                var entries=new ArrayList<com.mojang.datafixers.util.Pair<Climate.ParameterPoint,Holder<Biome>>>();
                for(var entry:presets.get(f.get("preset").getAsString()).values())entries.add(com.mojang.datafixers.util.Pair.of(entry.getFirst(),holders.get(entry.getSecond().identifier().toString())));
                BiomeSource source=MultiNoiseBiomeSource.createFromList(new Climate.ParameterList<>(entries));
                if(f.has("source"))switch(f.get("source").getAsString()) {
                    case "fixed" -> source=new FixedBiomeSource(holders.get(f.has("fixed_biome")?f.get("fixed_biome").getAsString():"minecraft:plains"));
                    case "checkerboard" -> source=new CheckerboardColumnBiomeSource(HolderSet.direct(List.of(holders.get("minecraft:plains"),holders.get("minecraft:the_end"),holders.get("minecraft:nether_wastes"))),f.get("scale").getAsInt());
                    case "end" -> {var constructor=TheEndBiomeSource.class.getDeclaredConstructor(Holder.class,Holder.class,Holder.class,Holder.class,Holder.class);constructor.setAccessible(true);source=constructor.newInstance(holders.get("minecraft:the_end"),holders.get("minecraft:end_highlands"),holders.get("minecraft:end_midlands"),holders.get("minecraft:small_end_islands"),holders.get("minecraft:end_barrens"));}
                    default -> throw new IllegalArgumentException("unknown source");
                }
                var material=Holder.Reference.<MaterialRule>createStandAlone(new HolderOwner<MaterialRule>(){},ResourceKey.create(Registries.MATERIAL_RULE,Identifier.parse("horizon:unused")));
                var settings=new NoiseGeneratorSettings(NoiseSettings.create(min,height),Blocks.STONE.defaultBlockState(),Blocks.WATER.defaultBlockState(),router,material,List.of(),sea,false,Optional.empty(),legacy,NoiseGeneratorSettings.DebugFunctions.EMPTY);
                var generator=new NoiseBasedChunkGenerator(source,Holder.direct(settings));
                var bounds=LevelHeightAccessor.create(f.get("write_min").getAsInt(),f.get("write_max").getAsInt()-f.get("write_min").getAsInt()+1);
                var chunk=new ProtoChunk(new ChunkPos(f.get("cx").getAsInt(),f.get("cz").getAsInt()),UpgradeData.EMPTY,LevelHeightAccessor.create(min,height),factory,null){@Override public LevelHeightAccessor getHeightAccessorForGeneration(){return f.has("retrogen")&&f.get("retrogen").getAsBoolean()?super.getHeightAccessorForGeneration():bounds;}};
                if(f.has("retrogen")&&f.get("retrogen").getAsBoolean()) {
                    chunk.fillBiomesFromNoise((x,y,z)->holders.get("horizon:old_"+Math.floorMod(x*31+y*7+z*17,3)));
                    chunk.setBelowZeroRetrogen(BelowZeroRetrogen.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString("{\"target_status\":\"minecraft:full\"}")).getOrThrow());
                } else chunk.setPersistedStatus(net.minecraft.world.level.chunk.status.ChunkStatus.BIOMES);
                for(int repeat=0;repeat<f.get("repeats").getAsInt();repeat++) {
                    fill.invoke(generator,Blender.empty(),state,chunk);out.println("fill "+repeat);
                    for(int section=0;section<height/16;section++){out.println("section "+section);var data=chunk.getSection(section);for(int y=0;y<4;y++)for(int z=0;z<4;z++)for(int x=0;x<4;x++)out.println("biome "+data.getNoiseBiome(x,y,z).unwrapKey().orElseThrow().identifier());}
                    var manager=new BiomeManager(chunk::getNoiseBiome,BiomeManager.obfuscateSeed(seed));
                    for(var q:f.getAsJsonArray("queries")){var a=q.getAsJsonArray();out.println("query "+manager.getBiome(new BlockPos(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt())).unwrapKey().orElseThrow().identifier());}
                }
            }
        }
    }
}
