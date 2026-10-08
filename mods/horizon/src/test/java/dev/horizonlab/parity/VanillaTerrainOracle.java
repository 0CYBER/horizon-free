package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.serialization.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
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

/** Original transport invoking NoiseChunk and the actual generator fill routine. */
public final class VanillaTerrainOracle {
    private static MappedRegistry<NormalNoise> noises;
    private static RegistryOps<JsonElement> ops;
    private static Holder<Biome> biome;
    private static PalettedContainerFactory factory;
    @SuppressWarnings("unchecked")
    private static void load(JsonObject bundle) {
        noises=new MappedRegistry<>(Registries.NOISE,Lifecycle.stable());
        var densities=new MappedRegistry<DensityFunction>(Registries.DENSITY_FUNCTION,Lifecycle.stable());
        var noiseLookup=noises.createRegistrationLookup();var densityLookup=densities.createRegistrationLookup();
        ops=RegistryOps.create(JsonOps.INSTANCE,new RegistryOps.RegistryInfoLookup() {
            public <T> Optional<HolderGetter<T>> lookup(ResourceKey<? extends Registry<? extends T>> key) {
                if(key.equals(Registries.NOISE))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(noiseLookup);
                if(key.equals(Registries.DENSITY_FUNCTION))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(densityLookup);
                return Optional.empty();
            }
        });
        for(var e:bundle.getAsJsonObject("noises").entrySet())noises.register(ResourceKey.create(Registries.NOISE,Identifier.parse(e.getKey())),NormalNoise.DIRECT_CODEC.parse(ops,JsonParser.parseString(e.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);
        noises.freeze();
        for(var e:bundle.getAsJsonObject("densities").entrySet())densities.register(ResourceKey.create(Registries.DENSITY_FUNCTION,Identifier.parse(e.getKey())),DensityFunctions.DIRECT_CODEC.parse(ops,JsonParser.parseString(e.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);
        densities.freeze();
        VanillaHeightmapOracle.bindTags(bundle.getAsJsonObject("tags"));
        biome=Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},Biomes.PLAINS);
        var ids=new IdMapper<Holder<Biome>>();ids.add(biome);
        factory=new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),Blocks.AIR.defaultBlockState(),null,Strategy.createForBiomes(ids),biome,null);
    }
    private static Beardifier structure(JsonObject f) {
        if(!f.has("structure"))return Beardifier.EMPTY;
        var source=f.getAsJsonObject("structure");
        var pieces=new ArrayList<Beardifier.Rigid>();
        var junctions=new ArrayList<net.minecraft.world.level.levelgen.structure.pools.JigsawJunction>();
        java.util.function.Function<JsonElement,net.minecraft.world.level.levelgen.structure.BoundingBox> box=e->{var a=e.getAsJsonArray();return new net.minecraft.world.level.levelgen.structure.BoundingBox(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt(),a.get(3).getAsInt(),a.get(4).getAsInt(),a.get(5).getAsInt());};
        for(var e:source.getAsJsonArray("pieces")) {var a=e.getAsJsonArray();pieces.add(new Beardifier.Rigid(box.apply(a.get(0)),net.minecraft.world.level.levelgen.structure.TerrainAdjustment.valueOf(a.get(1).getAsString()),a.get(2).getAsInt()));}
        for(var e:source.getAsJsonArray("junctions")) {var a=e.getAsJsonArray();junctions.add(new net.minecraft.world.level.levelgen.structure.pools.JigsawJunction(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt(),0,net.minecraft.world.level.levelgen.structure.pools.StructureTemplatePool.Projection.RIGID));}
        return new Beardifier(pieces,junctions,box.apply(source.get("affected")));
    }
    private static void fixture(JsonObject f,PrintWriter out) throws Exception {
        int min=f.get("min").getAsInt(),height=f.get("height").getAsInt(),noiseMin=f.get("noise_min").getAsInt(),noiseHeight=f.get("noise_height").getAsInt();
        int cx=f.get("cx").getAsInt(),cz=f.get("cz").getAsInt(),sea=f.get("sea").getAsInt();long seed=f.get("seed").getAsLong();boolean legacy=f.get("legacy").getAsBoolean();
        var block=Block.stateById(f.get("block").getAsInt());var fluid=Block.stateById(f.get("fluid").getAsInt());
        var router=NoiseRouter.CODEC.parse(ops,f.get("router")).getOrThrow();
        Optional<Aquifer.Config> config=f.get("aquifers").isJsonNull()?Optional.empty():Optional.of(Aquifer.Config.CODEC.parse(ops,f.get("aquifers")).getOrThrow());
        // Neither material rules nor biome properties are evaluated by doFill.
        var rule=Holder.Reference.createStandAlone(new HolderOwner<MaterialRule>(){},ResourceKey.create(Registries.MATERIAL_RULE,Identifier.parse("horizon:unused")));
        var settings=new NoiseGeneratorSettings(NoiseSettings.create(noiseMin,noiseHeight),block,fluid,router,rule,List.of(),sea,false,config,legacy,NoiseGeneratorSettings.DebugFunctions.EMPTY);
        var state=RandomState.create(noises,seed,legacy,block,sea,router);
        var generator=new NoiseBasedChunkGenerator(new FixedBiomeSource(biome),Holder.direct(settings));
        var pickerMethod=NoiseBasedChunkGenerator.class.getDeclaredMethod("createFluidPicker",NoiseGeneratorSettings.class);pickerMethod.setAccessible(true);
        var picker=(Aquifer.FluidPicker)pickerMethod.invoke(null,settings);
        var volume=new DensityVolume(16,noiseHeight,16,cx<<4,noiseMin,cz<<4,1,1,1);
        var chunk=new ProtoChunk(new ChunkPos(cx,cz),UpgradeData.EMPTY,LevelHeightAccessor.create(min,height),factory,null);
        for(var item:f.getAsJsonArray("initial")) {
            var a=item.getAsJsonArray();int x=a.get(0).getAsInt(),y=a.get(1).getAsInt(),z=a.get(2).getAsInt();
            chunk.getSection(chunk.getSectionIndex(y)).setBlockState(x&15,y&15,z&15,Block.stateById(a.get(3).getAsInt()),false);
        }
        var fill=NoiseBasedChunkGenerator.class.getDeclaredMethod("doFill",NoiseChunk.class,ChunkAccess.class);fill.setAccessible(true);
        out.println("chunk "+cx+" "+cz+" "+min+" "+height);
        try(var noise=new NoiseChunk(state,structure(f),settings,picker,Blender.empty(),volume)) {
            for(int repeat=0;repeat<f.get("repeats").getAsInt();repeat++) {
                fill.invoke(generator,noise,chunk);
                out.println("fill "+repeat+" "+noise.aquifer().shouldScheduleFluidUpdate());
                for(int section=0;section<height/16;section++) {
                    var data=chunk.getSection(section);out.println("section "+section);
                    for(int y=0;y<16;y++)for(int z=0;z<16;z++)for(int x=0;x<16;x++)out.println(Block.getId(data.getBlockState(x,y,z)));
                }
                for(var type:new Heightmap.Types[]{Heightmap.Types.OCEAN_FLOOR_WG,Heightmap.Types.WORLD_SURFACE_WG}) {
                    var map=chunk.getOrCreateHeightmapUnprimed(type);out.println("heightmap "+type);
                    for(long word:map.getRawData())out.println(Long.toUnsignedString(word,16));
                }
                var offsets=chunk.getPostProcessing();
                for(int section=0;section<offsets.length;section++) {
                    var list=offsets[section];out.println("post "+section+" "+(list==null?0:list.size()));
                    if(list!=null)for(short offset:list)out.println(Short.toUnsignedInt(offset));
                }
            }
        }
    }
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();Bootstrap.bootStrap();
        if(SharedConstants.DEBUG_AQUIFERS || SharedConstants.DEBUG_DISABLE_FLUID_GENERATION)throw new AssertionError("regular gameplay flags required");
        Path dir=Path.of(args[1]);load(JsonParser.parseString(Files.readString(dir.resolve("registries.json"))).getAsJsonObject());
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(String line:Files.readAllLines(dir.resolve("inputs.txt")))fixture(JsonParser.parseString(line).getAsJsonObject(),out);
        }
    }
}
