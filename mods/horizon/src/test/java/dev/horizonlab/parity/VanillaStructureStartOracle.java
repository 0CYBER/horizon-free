package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.TagLoader;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.biome.MultiNoiseBiomeSource;
import net.minecraft.world.level.biome.MultiNoiseBiomeSourceParameterLists;
import net.minecraft.world.level.chunk.ChunkGeneratorStructureState;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.structure.*;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplateManager;
import net.minecraft.world.level.storage.LevelStorageSource;

/**
 * Original oracle: runs vanilla structure placement checks, start selection
 * and Structure.generate (as ChunkGenerator.createStructures does for a fresh
 * chunk) for a rectangle of chunks (overworld, or HORIZON_DIMENSION=the_nether /
 * the_end) and prints every valid start.
 */
public final class VanillaStructureStartOracle {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();Bootstrap.bootStrap();
        Path out=Path.of(args[0]);long seed=Long.parseLong(args[1]);
        int x0=Integer.parseInt(args[2]),z0=Integer.parseInt(args[3]),x1=Integer.parseInt(args[4]),z1=Integer.parseInt(args[5]);
        var repo=ServerPacksSource.createVanillaTrustedRepository();repo.reload();repo.setSelected(List.of("vanilla"));
        var resources=new MultiPackResourceManager(PackType.SERVER_DATA,repo.openAllSelected());
        var layers=RegistryLayer.createRegistryAccess();
        var staticTags=TagLoader.loadTagsForExistingRegistries(resources,layers.getLayer(RegistryLayer.STATIC));
        var context=TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD),staticTags);
        RegistryAccess.Frozen world=RegistryDataLoader.load(resources,context,RegistryDataLoader.WORLD_REGISTRIES,Runnable::run).join();
        staticTags.forEach(Registry.PendingTags::apply);
        RegistryAccess access=layers.replaceFrom(RegistryLayer.WORLD,world).compositeAccess();
        String dimension=System.getenv().getOrDefault("HORIZON_DIMENSION","overworld");
        var settings=world.lookupOrThrow(Registries.NOISE_SETTINGS).getOrThrow(switch(dimension){case "the_nether"->NoiseGeneratorSettings.NETHER;case "the_end"->NoiseGeneratorSettings.END;default->NoiseGeneratorSettings.OVERWORLD;});
        var randomState=RandomState.create(world.lookupOrThrow(Registries.NOISE),seed,settings.value());
        var presets=world.lookupOrThrow(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST);
        net.minecraft.world.level.biome.BiomeSource biomeSource=switch(dimension){
            case "the_nether"->MultiNoiseBiomeSource.createFromPreset(presets.getOrThrow(MultiNoiseBiomeSourceParameterLists.NETHER));
            case "the_end"->net.minecraft.world.level.biome.TheEndBiomeSource.create(world.lookupOrThrow(Registries.BIOME));
            default->MultiNoiseBiomeSource.createFromPreset(presets.getOrThrow(MultiNoiseBiomeSourceParameterLists.OVERWORLD));};
        var levelKey=switch(dimension){case "the_nether"->Level.NETHER;case "the_end"->Level.END;default->Level.OVERWORLD;};
        var generator=new NoiseBasedChunkGenerator(biomeSource,settings);
        var state=ChunkGeneratorStructureState.createForNormal(randomState,seed,new ChunkPos(0,0),biomeSource,world.lookupOrThrow(Registries.STRUCTURE_SET));
        Path tmp=Files.createTempDirectory("horizon-structures");
        var storage=LevelStorageSource.createDefault(tmp).createAccess("oracle");
        var templates=new StructureTemplateManager(resources,storage,DataFixers.getDataFixer(),BuiltInRegistries.BLOCK);
        LevelHeightAccessor height=dimension.equals("overworld")?LevelHeightAccessor.create(-64,384):LevelHeightAccessor.create(0,256);
        var climate=randomState.createClimateSampler(net.minecraft.world.level.levelgen.densityfunction.SamplerContext.builder().enableCaches().build());
        var structures=world.lookupOrThrow(Registries.STRUCTURE);
        try(var w=new PrintWriter(Files.newBufferedWriter(out,StandardCharsets.UTF_8))) {
            structures.listElements().forEach(h->w.println("registry "+h.key().identifier()+" "+h.value().step().ordinal()));
            if(System.getenv("HORIZON_RINGS")!=null)for(var set:state.possibleStructureSets())
                if(set.value().placement() instanceof net.minecraft.world.level.levelgen.structure.placement.ConcentricRingsStructurePlacement rings)
                    for(var p:state.getRingPositionsFor(rings))w.println("ring "+set.unwrapKey().get().identifier()+" "+p.x()+" "+p.z());
            if(System.getenv("HORIZON_HEIGHTS")!=null)for(String p:System.getenv("HORIZON_HEIGHTS").split(";")){
                String[] xz=p.split(",");int x=Integer.parseInt(xz[0]),z=Integer.parseInt(xz[1]);
                int floor=generator.getBaseHeight(x,z,Heightmap.Types.OCEAN_FLOOR_WG,height,randomState),surface=generator.getBaseHeight(x,z,Heightmap.Types.WORLD_SURFACE_WG,height,randomState);
                var resolver=biomeSource.createResolver(climate);
                StringBuilder biomes=new StringBuilder();
                for(int qy=(floor-12)>>2;qy<=(floor+8)>>2;qy++)biomes.append(" "+qy+":"+resolver.getNoiseBiome(x>>2,qy,z>>2).unwrapKey().get().identifier().getPath());
                w.println("height "+x+" "+z+" "+floor+" "+surface+biomes);
            }
            // HORIZON_CHUNKS="cx,cz;cx,cz": only these chunks, instead of the rectangle.
            List<int[]> chunks=new ArrayList<>();
            String only=System.getenv("HORIZON_CHUNKS");
            if(only!=null)for(String c:only.split(";")){String[] xz=c.split(",");chunks.add(new int[]{Integer.parseInt(xz[0]),Integer.parseInt(xz[1])});}
            else for(int cx=x0;cx<=x1;cx++)for(int cz=z0;cz<=z1;cz++)chunks.add(new int[]{cx,cz});
            for(int[] chunk:chunks) {
                int cx=chunk[0],cz=chunk[1];
                ChunkPos pos=new ChunkPos(cx,cz);
                for(var set:state.possibleStructureSets()) {
                    var placement=set.value().placement();var entries=set.value().structures();
                    if(!placement.isStructureChunk(state,cx,cz))continue;
                    if(System.getenv("HORIZON_PLACED")!=null)w.println("placed "+cx+" "+cz+" "+set.unwrapKey().get().identifier());
                    var options=new ArrayList<>(entries);
                    WorldgenRandom random=new WorldgenRandom(new LegacyRandomSource(0L));random.setLargeFeatureSeed(seed,cx,cz);
                    int total=0;for(var o:options)total+=o.weight();
                    while(!options.isEmpty()) {
                        StructureSet.StructureSelectionEntry selected;int index=0;
                        if(entries.size()==1){selected=options.get(0);}
                        else{int choice=random.nextInt(total);for(var o:options){choice-=o.weight();if(choice<0)break;index++;}selected=options.get(index);}
                        var structure=selected.structure();
                        StructureStart start=structure.value().generate(structure,levelKey,access,generator,biomeSource,climate,randomState,templates,seed,pos,0,height,structure.value().biomes()::contains);
                        if(System.getenv("HORIZON_PLACED")!=null){
                            w.println("generate "+structure.unwrapKey().get().identifier()+" "+start.isValid());
                            var ctx=new Structure.GenerationContext(access,generator,biomeSource,climate,randomState,templates,seed,pos,height,structure.value().biomes()::contains);
                            try{var m=Structure.class.getDeclaredMethod("findGenerationPoint",Structure.GenerationContext.class);m.setAccessible(true);
                            var stub=(java.util.Optional<?>)m.invoke(structure.value(),ctx);
                            stub.ifPresent(st->{var sp=((Structure.GenerationStub)st).position();w.println("stub "+sp+" "+ctx.biomeResolver().getNoiseBiome(sp.getX()>>2,sp.getY()>>2,sp.getZ()>>2).unwrapKey().get().identifier());});}catch(Exception e){w.println("err "+e);}
                        }
                        if(start.isValid()) {
                            var b=start.getBoundingBox();
                            w.println("start "+cx+" "+cz+" "+structure.unwrapKey().get().identifier()+" "+b.minX()+" "+b.minY()+" "+b.minZ()+" "+b.maxX()+" "+b.maxY()+" "+b.maxZ()+" "+start.getPieces().size());
                            for(var piece:start.getPieces()) {
                                var pb=piece.getBoundingBox();
                                String extra="";
                                if(piece instanceof PoolElementStructurePiece pool)extra=" "+pool.getPosition().getX()+" "+pool.getPosition().getY()+" "+pool.getPosition().getZ()+" "+pool.getRotation()+" "+pool.getGroundLevelDelta()+" "+pool.getJunctions().size()+" "+pool.getElement();
                                w.println("piece "+BuiltInRegistries.STRUCTURE_PIECE.getKey(piece.getType())+" "+pb.minX()+" "+pb.minY()+" "+pb.minZ()+" "+pb.maxX()+" "+pb.maxY()+" "+pb.maxZ()+extra);
                            }
                            if(System.getenv("HORIZON_START_NBT")!=null)
                                w.println("nbt "+cx+" "+cz+" "+canonical(start.createTag(new net.minecraft.world.level.levelgen.structure.pieces.StructurePieceSerializationContext(resources,access,templates),pos)));
                            break;
                        }
                        if(entries.size()==1)break;
                        options.remove(index);total-=selected.weight();
                    }
                }
            }
        }
        storage.close();
        System.exit(0);
    }

    /** A tag as text with sorted keys and every number's type, so equal tags print equally. */
    static String canonical(net.minecraft.nbt.Tag tag) {
        return switch (tag) {
            case net.minecraft.nbt.CompoundTag c -> {
                var keys = new ArrayList<>(c.keySet());
                Collections.sort(keys);
                var b = new StringBuilder("{");
                for (int i = 0; i < keys.size(); i++) b.append(i == 0 ? "" : ",").append(keys.get(i)).append(':').append(canonical(c.get(keys.get(i))));
                yield b.append('}').toString();
            }
            case net.minecraft.nbt.ListTag l -> {
                var b = new StringBuilder("[");
                for (int i = 0; i < l.size(); i++) b.append(i == 0 ? "" : ",").append(canonical(l.get(i)));
                yield b.append(']').toString();
            }
            case net.minecraft.nbt.ByteTag v -> v.value() + "b";
            case net.minecraft.nbt.ShortTag v -> v.value() + "s";
            case net.minecraft.nbt.IntTag v -> String.valueOf(v.value());
            case net.minecraft.nbt.LongTag v -> v.value() + "L";
            case net.minecraft.nbt.FloatTag v -> Float.floatToRawIntBits(v.value()) + "f";
            case net.minecraft.nbt.DoubleTag v -> Double.doubleToRawLongBits(v.value()) + "d";
            case net.minecraft.nbt.StringTag v -> "\"" + v.value().replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
            case net.minecraft.nbt.IntArrayTag v -> "[I;" + String.join(",", Arrays.stream(v.getAsIntArray()).mapToObj(String::valueOf).toList()) + "]";
            case net.minecraft.nbt.LongArrayTag v -> "[L;" + String.join(",", Arrays.stream(v.getAsLongArray()).mapToObj(String::valueOf).toList()) + "]";
            case net.minecraft.nbt.ByteArrayTag v -> { var b = new StringBuilder("[B;"); byte[] a = v.getAsByteArray(); for (int i = 0; i < a.length; i++) b.append(i == 0 ? "" : ",").append(a[i]); yield b.append(']').toString(); }
            default -> tag.toString();
        };
    }
}
