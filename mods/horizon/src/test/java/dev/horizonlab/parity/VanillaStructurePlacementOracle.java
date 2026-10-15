package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.TagLoader;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.world.RandomizableContainer;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.EntityBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.blending.Blender;
import net.minecraft.world.level.levelgen.structure.*;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplateManager;
import net.minecraft.world.level.storage.LevelStorageSource;

/**
 * Original oracle: vanilla terrain (biomes and buildTerrain with live
 * structure starts and references) for the 3x3 chunks around each target,
 * then the structure part of ChunkGenerator.applyBiomeDecoration (no placed
 * features) through a WorldGenRegion-like level, and a dump of the target
 * chunk's blocks and block entities.
 *
 * usage: OUT SEED DIMENSION CX,CZ...
 */
public final class VanillaStructurePlacementOracle {
    private static final class Ignored extends RuntimeException {}

    /**
     * Allocated without a constructor: no feature flags are enabled, so
     * EntityType.create returns null and structure entities are skipped as
     * Horizon skips them, while block placement continues.
     */
    private static final class NoEntityLevel extends net.minecraft.server.level.ServerLevel {
        private NoEntityLevel() {
            super(null, null, null, null, null, null, false, 0L, null, false);
        }
        static long seed;
        @Override
        public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() {
            return net.minecraft.world.flag.FeatureFlagSet.of();
        }
        @Override
        public long getSeed() {
            return seed;
        }
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();Bootstrap.bootStrap();
        Path out=Path.of(args[0]);long seed=Long.parseLong(args[1]);String dimension=args[2];
        var repo=ServerPacksSource.createVanillaTrustedRepository();repo.reload();repo.setSelected(List.of("vanilla"));
        var resources=new MultiPackResourceManager(PackType.SERVER_DATA,repo.openAllSelected());
        var layers=RegistryLayer.createRegistryAccess();
        var staticTags=TagLoader.loadTagsForExistingRegistries(resources,layers.getLayer(RegistryLayer.STATIC));
        var context=TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD),staticTags);
        RegistryAccess.Frozen world=RegistryDataLoader.load(resources,context,RegistryDataLoader.WORLD_REGISTRIES,Runnable::run).join();
        var loaded=net.minecraft.server.ReloadableServerRegistries.reload(layers.replaceFrom(RegistryLayer.WORLD,world),staticTags,resources,Runnable::run).join();
        var components=BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(loaded.lookupWithUpdatedTags());
        staticTags.forEach(Registry.PendingTags::apply);
        components.forEach(net.minecraft.core.component.DataComponentInitializers.PendingComponents::apply);
        RegistryAccess access=loaded.layers().compositeAccess();
        var settings=world.lookupOrThrow(Registries.NOISE_SETTINGS).getOrThrow(switch(dimension){case "the_nether"->NoiseGeneratorSettings.NETHER;case "the_end"->NoiseGeneratorSettings.END;default->NoiseGeneratorSettings.OVERWORLD;});
        var presets=world.lookupOrThrow(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST);
        BiomeSource biomeSource=switch(dimension){
            case "the_nether"->MultiNoiseBiomeSource.createFromPreset(presets.getOrThrow(MultiNoiseBiomeSourceParameterLists.NETHER));
            case "the_end"->TheEndBiomeSource.create(world.lookupOrThrow(Registries.BIOME));
            default->MultiNoiseBiomeSource.createFromPreset(presets.getOrThrow(MultiNoiseBiomeSourceParameterLists.OVERWORLD));};
        var levelKey=switch(dimension){case "the_nether"->Level.NETHER;case "the_end"->Level.END;default->Level.OVERWORLD;};
        var randomState=RandomState.create(world.lookupOrThrow(Registries.NOISE),seed,settings.value());
        var generator=new NoiseBasedChunkGenerator(biomeSource,settings);
        var state=ChunkGeneratorStructureState.createForNormal(randomState,seed,new ChunkPos(0,0),biomeSource,world.lookupOrThrow(Registries.STRUCTURE_SET));
        Path tmp=Files.createTempDirectory("horizon-structures");
        var storage=LevelStorageSource.createDefault(tmp).createAccess("oracle");
        var templates=new StructureTemplateManager(resources,storage,DataFixers.getDataFixer(),BuiltInRegistries.BLOCK);
        LevelHeightAccessor height=dimension.equals("overworld")?LevelHeightAccessor.create(-64,384):LevelHeightAccessor.create(0,256);
        var climate=randomState.createClimateSampler(net.minecraft.world.level.levelgen.densityfunction.SamplerContext.builder().enableCaches().build());
        var structures=world.lookupOrThrow(Registries.STRUCTURE);
        var factory=PalettedContainerFactory.create(access);
        var unsafeField=Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");unsafeField.setAccessible(true);Object unsafe=unsafeField.get(null);
        NoEntityLevel.seed=seed;
        var noEntities=(net.minecraft.server.level.ServerLevel)unsafe.getClass().getMethod("allocateInstance",Class.class).invoke(unsafe,NoEntityLevel.class);
        try(var w=new PrintWriter(Files.newBufferedWriter(out,StandardCharsets.UTF_8))) {
            for(int t=3;t<args.length;t++) {
                String[] xz=args[t].split(",");int cx=Integer.parseInt(xz[0]),cz=Integer.parseInt(xz[1]);
                Map<Long,ProtoChunk> chunks=new HashMap<>();
                java.util.function.BiFunction<Integer,Integer,ProtoChunk> chunkAt=(x,z)->chunks.computeIfAbsent(ChunkPos.pack(x,z),k->new ProtoChunk(new ChunkPos(x,z),UpgradeData.EMPTY,height,factory,null));
                // STRUCTURE_STARTS within reference range of the 3x3 region.
                for(int x=cx-9;x<=cx+9;x++)for(int z=cz-9;z<=cz+9;z++) {
                    ChunkPos pos=new ChunkPos(x,z);ProtoChunk chunk=chunkAt.apply(x,z);
                    for(var set:state.possibleStructureSets()) {
                        var placement=set.value().placement();var entries=set.value().structures();
                        if(!placement.isStructureChunk(state,x,z))continue;
                        var options=new ArrayList<>(entries);
                        WorldgenRandom random=new WorldgenRandom(new LegacyRandomSource(0L));random.setLargeFeatureSeed(seed,x,z);
                        int total=0;for(var o:options)total+=o.weight();
                        while(!options.isEmpty()) {
                            StructureSet.StructureSelectionEntry selected;int index=0;
                            if(entries.size()==1){selected=options.get(0);}
                            else{int choice=random.nextInt(total);for(var o:options){choice-=o.weight();if(choice<0)break;index++;}selected=options.get(index);}
                            var structure=selected.structure();
                            StructureStart start=structure.value().generate(structure,levelKey,access,generator,biomeSource,climate,randomState,templates,seed,pos,0,height,structure.value().biomes()::contains);
                            if(start.isValid()){chunk.setStartForStructure(structure.value(),start);break;}
                            if(entries.size()==1)break;
                            options.remove(index);total-=selected.weight();
                        }
                    }
                }
                // STRUCTURE_REFERENCES for the 3x3 region.
                for(int x=cx-1;x<=cx+1;x++)for(int z=cz-1;z<=cz+1;z++) {
                    ProtoChunk chunk=chunkAt.apply(x,z);int bx=x<<4,bz=z<<4;
                    for(int sx=x-8;sx<=x+8;sx++)for(int sz=z-8;sz<=z+8;sz++)
                        for(StructureStart start:chunkAt.apply(sx,sz).getAllStarts().values())
                            if(start.isValid()&&start.getBoundingBox().intersects(bx,bz,bx+15,bz+15))chunk.addReferenceForStructure(start.getStructure(),ChunkPos.pack(sx,sz));
                }
                int[] center={cx,cz};
                var heightRef=new Object(){LevelHeightAccessor value=height;};
                BiomeManager[] zoom=new BiomeManager[1];
                // WorldGenRegion's static attribute layers (lava's tick delay reads FAST_LAVA).
                net.minecraft.world.attribute.EnvironmentAttributeSystem[] attributes=new net.minecraft.world.attribute.EnvironmentAttributeSystem[1];
                var uncached=biomeSource;
                Object[] self=new Object[1];
                var level=(WorldGenLevel)Proxy.newProxyInstance(WorldGenLevel.class.getClassLoader(),new Class<?>[]{WorldGenLevel.class},(proxy,method,values)->{
                    switch(method.getName()) {
                        case "getMinY":return heightRef.value.getMinY();
                        case "getHeight":{
                            if(values==null||values.length==0)return heightRef.value.getHeight();
                            if(values.length==3){var kind=(Heightmap.Types)values[0];int x=(Integer)values[1],z=(Integer)values[2];return chunkAt.apply(x>>4,z>>4).getHeight(kind,x&15,z&15)+1;}
                            break;
                        }
                        case "getSeaLevel":return generator.getSeaLevel();
                        case "getSeed":return seed;
                        case "registryAccess":return access;
                        case "getRandom":return self[0];
                        case "getBlockState":{var p=(BlockPos)values[0];return chunkAt.apply(p.getX()>>4,p.getZ()>>4).getBlockState(p);}
                        case "getFluidState":{var p=(BlockPos)values[0];return chunkAt.apply(p.getX()>>4,p.getZ()>>4).getFluidState(p);}
                        case "isStateAtPosition":{var p=(BlockPos)values[0];@SuppressWarnings("unchecked")var test=(java.util.function.Predicate<BlockState>)values[1];return test.test(chunkAt.apply(p.getX()>>4,p.getZ()>>4).getBlockState(p));}
                        case "isFluidAtPosition":{var p=(BlockPos)values[0];@SuppressWarnings("unchecked")var test=(java.util.function.Predicate<net.minecraft.world.level.material.FluidState>)values[1];return test.test(chunkAt.apply(p.getX()>>4,p.getZ()>>4).getFluidState(p));}
                        case "ensureCanWrite":{var p=(BlockPos)values[0];return Math.abs((p.getX()>>4)-center[0])<=1&&Math.abs((p.getZ()>>4)-center[1])<=1;}
                        case "setBlock":{
                            if(values.length!=4)break;
                            var p=(BlockPos)values[0];var blockState=(BlockState)values[1];int flags=(Integer)values[2];
                            if(!(Math.abs((p.getX()>>4)-center[0])<=1&&Math.abs((p.getZ()>>4)-center[1])<=1))return false;
                            ProtoChunk chunk=chunkAt.apply(p.getX()>>4,p.getZ()>>4);
                            BlockState old=chunk.setBlockState(p,blockState,flags);
                            if(blockState.hasBlockEntity()){var tag=new net.minecraft.nbt.CompoundTag();tag.putInt("x",p.getX());tag.putInt("y",p.getY());tag.putInt("z",p.getZ());tag.putString("id","DUMMY");chunk.setBlockEntityNbt(tag);}
                            else if(old!=null&&old.hasBlockEntity())chunk.removeBlockEntity(p);
                            if((flags&16)==0){BlockPos post=blockState.getPostProcessPos((BlockGetter)proxy,p);if(post!=null)chunkAt.apply(post.getX()>>4,post.getZ()>>4).markPosForPostProcessing(post);}
                            return true;
                        }
                        case "getBlockEntity":{
                            if(values.length!=1)break;
                            var p=(BlockPos)values[0];ProtoChunk chunk=chunkAt.apply(p.getX()>>4,p.getZ()>>4);
                            BlockEntity be=chunk.getBlockEntity(p);if(be!=null)return be;
                            var tag=chunk.getBlockEntityNbt(p);BlockState st=chunk.getBlockState(p);
                            if(tag!=null){
                                if("DUMMY".equals(tag.getStringOr("id",""))){if(!st.hasBlockEntity())return null;be=((EntityBlock)st.getBlock()).newBlockEntity(p,st);}
                                else be=BlockEntity.loadStatic(p,st,tag,access);
                                if(be!=null){chunk.setBlockEntity(be);return be;}
                            }
                            return null;
                        }
                        case "getChunk":{
                            int x,z;
                            if(values[0] instanceof BlockPos p){x=p.getX()>>4;z=p.getZ()>>4;}else{x=(Integer)values[0];z=(Integer)values[1];}
                            if(values.length==4&&!(Boolean)values[3]&&!chunks.containsKey(ChunkPos.pack(x,z)))return null;
                            return chunkAt.apply(x,z);
                        }
                        case "hasChunk":return true;
                        case "getBiomeManager":return zoom[0];
                        case "environmentAttributes":return attributes[0];
                        case "getBiome":return zoom[0].getBiome((BlockPos)values[0]);
                        case "getNoiseBiome":{int qx=(Integer)values[0],qy=(Integer)values[1],qz=(Integer)values[2];return chunkAt.apply(qx>>2,qz>>2).getNoiseBiome(qx,qy,qz);}
                        case "getUncachedNoiseBiome":return uncached.createResolver(climate).getNoiseBiome((Integer)values[0],(Integer)values[1],(Integer)values[2]);
                        case "setCurrentlyGenerating","scheduleTick","blockUpdated","updateNeighborsAt","gameEvent","playSound","levelEvent","addParticle":return null;
                        case "addFreshEntity":return true;
                        case "addFreshEntityWithPassengers":return null;
                        case "getLevel":return noEntities;
                        case "isClientSide":return false;
                        case "enabledFeatures":return net.minecraft.world.flag.FeatureFlags.DEFAULT_FLAGS;
                        case "dimensionType":return world.lookupOrThrow(Registries.DIMENSION_TYPE).getOrThrow(switch(dimension){case "the_nether"->net.minecraft.world.level.dimension.BuiltinDimensionTypes.NETHER;case "the_end"->net.minecraft.world.level.dimension.BuiltinDimensionTypes.END;default->net.minecraft.world.level.dimension.BuiltinDimensionTypes.OVERWORLD;}).value();
                        case "hashCode":return System.identityHashCode(proxy);
                        case "equals":return proxy==values[0];
                        case "toString":return "oracle region";
                    }
                    if(method.isDefault())return InvocationHandler.invokeDefault(proxy,method,values);
                    throw new AssertionError(method);
                });
                self[0]=randomState.getOrCreateRandomFactory(Identifier.withDefaultNamespace("worldgen_region_random")).at(new ChunkPos(cx,cz).getWorldPosition());
                zoom[0]=new BiomeManager(level,BiomeManager.obfuscateSeed(seed));
                attributes[0]=net.minecraft.world.attribute.EnvironmentAttributeSystem.builder().addStaticLayers(level).build();
                var manager=new StructureManager(level,new WorldOptions(seed,true,false),null);
                // BIOMES for the 5x5 area, then TERRAIN for the 3x3 region.
                for(int x=cx-2;x<=cx+2;x++)for(int z=cz-2;z<=cz+2;z++){generator.createBiomes(randomState,Blender.empty(),manager,chunkAt.apply(x,z)).join();chunkAt.apply(x,z).setPersistedStatus(ChunkStatus.BIOMES);}
                for(int x=cx-1;x<=cx+1;x++)for(int z=cz-1;z<=cz+1;z++) {
                    ProtoChunk chunk=chunkAt.apply(x,z);
                    Set<Holder<Biome>> possible=new HashSet<>();
                    for(int dx=-1;dx<=1;dx++)for(int dz=-1;dz<=1;dz++)chunkAt.apply(x+dx,z+dz).collectBiomesInPalette(possible);
                    var region=new BiomeManager(level,BiomeManager.obfuscateSeed(seed));
                    generator.buildTerrain(chunk,Blender.empty(),randomState,manager,region,null,possible).join();
                    Heightmap.primeHeightmaps(chunk,EnumSet.of(Heightmap.Types.MOTION_BLOCKING,Heightmap.Types.MOTION_BLOCKING_NO_LEAVES,Heightmap.Types.OCEAN_FLOOR,Heightmap.Types.WORLD_SURFACE));
                    chunk.setPersistedStatus(ChunkStatus.TERRAIN);
                }
                // FEATURES: structure pieces only.
                ProtoChunk target=chunkAt.apply(cx,cz);
                SectionPos sectionPos=SectionPos.of(new ChunkPos(cx,cz),level.getMinSectionY());
                var structuresByStep=new TreeMap<Integer,List<Structure>>();
                structures.listElements().forEach(h->structuresByStep.computeIfAbsent(h.value().step().ordinal(),k->new ArrayList<>()).add(h.value()));
                WorldgenRandom random=new WorldgenRandom(new XoroshiroRandomSource(0L));
                long decorationSeed=random.setDecorationSeed(seed,sectionPos.origin().getX(),sectionPos.origin().getZ());
                var chunkBox=new BoundingBox(cx<<4,height.getMinY()+1,cz<<4,(cx<<4)+15,height.getMaxY(),(cz<<4)+15);
                for(int step=0;step<GenerationStep.Decoration.values().length;step++) {
                    int index=0;
                    for(Structure structure:structuresByStep.getOrDefault(step,List.of())) {
                        random.setFeatureSeed(decorationSeed,index,step);
                        for(StructureStart start:manager.startsForStructure(sectionPos.x(),sectionPos.z(),structure)) {
                            try{start.placeInChunk(level,manager,generator,random,chunkBox,new ChunkPos(cx,cz));}
                            catch(Ignored e){w.println("note entity skipped");}
                        }
                        index++;
                    }
                }
                w.println("chunk "+cx+" "+cz);
                for(int y=height.getMinY();y<=height.getMaxY();y++) {
                    StringBuilder line=new StringBuilder("y "+y);
                    for(int z=0;z<16;z++)for(int x=0;x<16;x++)line.append(' ').append(Block.getId(target.getBlockState(new BlockPos((cx<<4)+x,y,(cz<<4)+z))));
                    w.println(line);
                }
                var entities=new TreeMap<BlockPos,BlockEntity>(Comparator.<BlockPos>comparingInt(BlockPos::getX).thenComparingInt(BlockPos::getY).thenComparingInt(BlockPos::getZ));
                entities.putAll(target.getBlockEntities());
                for(var e:entities.values()) {
                    var p=e.getBlockPos();String table="-",lootSeed="-";
                    if(e instanceof RandomizableContainer rc&&rc.getLootTable()!=null){table=rc.getLootTable().identifier().toString();lootSeed=Long.toString(rc.getLootTableSeed());}
                    if(e instanceof net.minecraft.world.level.block.entity.BrushableBlockEntity){try{var f=e.getClass().getDeclaredField("lootTable");f.setAccessible(true);var key=(net.minecraft.resources.ResourceKey<?>)f.get(e);var s=e.getClass().getDeclaredField("lootTableSeed");s.setAccessible(true);if(key!=null){table=key.identifier().toString();lootSeed=Long.toString(s.getLong(e));}}catch(ReflectiveOperationException ex){throw new RuntimeException(ex);}}
                    w.println("be "+p.getX()+" "+p.getY()+" "+p.getZ()+" "+BuiltInRegistries.BLOCK_ENTITY_TYPE.getKey(e.getType())+" "+table+" "+lootSeed);
                }
            }
        }
        storage.close();
        System.exit(0);
    }
}
