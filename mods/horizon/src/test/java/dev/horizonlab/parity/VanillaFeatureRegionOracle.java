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
import net.minecraft.tags.*;
import net.minecraft.world.ticks.*;
import net.minecraft.world.level.material.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.feature.Feature;
import net.minecraft.world.level.levelgen.placement.*;

/** Native feature placement into resident ProtoChunks through a world transport. */
public final class VanillaFeatureRegionOracle {
    // Execute the actual native destruction method with resident storage as its
    // world transport; no ServerLevel constructor or copied destruction body.
    private static final class NativeDestruction extends net.minecraft.server.level.WorldGenRegion {
        WorldGenLevel world;
        private NativeDestruction(){super(null,null,null,null);}
        @Override public BlockState getBlockState(BlockPos p){return world.getBlockState(p);}
        @Override public boolean setBlock(BlockPos p,BlockState state,int flags,int recursion){return world.setBlock(p,state,flags,recursion);}
    }
    private static NativeDestruction destruction(WorldGenLevel world)throws ReflectiveOperationException {
        var field=sun.misc.Unsafe.class.getDeclaredField("theUnsafe");field.setAccessible(true);
        var value=(NativeDestruction)((sun.misc.Unsafe)field.get(null)).allocateInstance(NativeDestruction.class);value.world=world;return value;
    }

    private static void setWriteZone(NativeDestruction world,int x,int z,int radius)throws ReflectiveOperationException {
        String[] names={"centerChunkX","centerChunkZ","writeRadius"};int[] values={x,z,radius};
        for(int i=0;i<names.length;i++){var field=net.minecraft.server.level.WorldGenRegion.class.getDeclaredField(names[i]);field.setAccessible(true);field.setInt(world,values[i]);}
    }
    private static void center(NativeDestruction world,ProtoChunk chunk)throws ReflectiveOperationException {
        var field=net.minecraft.server.level.WorldGenRegion.class.getDeclaredField("center");field.setAccessible(true);field.set(world,chunk);
    }
    private static BelowZeroRetrogen retrogen(){return BelowZeroRetrogen.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString("{\"target_status\":\"minecraft:features\"}")).getOrThrow();}
    private static ProtoChunk chunk(Map<ChunkPos,ProtoChunk> chunks,int x,int z) {
        var chunk=chunks.get(new ChunkPos(x>>4,z>>4));if(chunk==null)throw new IllegalArgumentException("missing region chunk");return chunk;
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
    private static void queryState(WorldGenLevel world, BlockPos p, List<String> tags, PrintWriter out) {
        out.println("query "+p.getX()+" "+p.getY()+" "+p.getZ());
        try { var fluid=world.getFluidState(p); boolean falling=fluid.hasProperty(FlowingFluid.FALLING)&&fluid.getValue(FlowingFluid.FALLING); out.println("fluid "+BuiltInRegistries.FLUID.getKey(fluid.getType())+" "+fluid.getAmount()+" "+falling+" "+fluid.isFull()+" "+fluid.isSource()); }
        catch(RuntimeException error){out.println("fluid error");}
        try {out.println("water "+world.isWaterAt(p));}catch(RuntimeException error){out.println("water error");}
        var bits=new StringBuilder();
        for(var tag:tags){try{bits.append(world.getBlockState(p).is(TagKey.create(Registries.BLOCK,Identifier.parse(tag)))?'1':'0');}catch(RuntimeException error){bits.append('E');}}
        out.println("tags "+bits);
    }
    private static void upgradeLifecycle(PalettedContainerFactory factory, PrintWriter out)throws ReflectiveOperationException {
        var stages=new ChunkStatus[]{ChunkStatus.EMPTY,ChunkStatus.STRUCTURE_STARTS,ChunkStatus.STRUCTURE_REFERENCES,ChunkStatus.BIOMES,ChunkStatus.TERRAIN,ChunkStatus.FEATURES};
        var targets=new ChunkStatus[]{ChunkStatus.STRUCTURE_STARTS,ChunkStatus.STRUCTURE_REFERENCES,ChunkStatus.BIOMES,ChunkStatus.TERRAIN,ChunkStatus.FEATURES,ChunkStatus.INITIALIZE_LIGHT,ChunkStatus.LIGHT,ChunkStatus.SPAWN,ChunkStatus.FULL};
        for(var stage:stages)for(var target:targets) {
            out.println("lifecycle "+stage.getName()+" "+target.getName());
            var data=BelowZeroRetrogen.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString("{\"target_status\":\""+target.getName()+"\"}")).getOrThrow();
            var chunks=new ProtoChunk[2];
            for(int x=0;x<2;x++) {
                var chunk=new ProtoChunk(new ChunkPos(x,0),UpgradeData.EMPTY,LevelHeightAccessor.create(-64,128),factory,null);
                chunk.setPersistedStatus(stage);chunk.setBelowZeroRetrogen(data);chunks[x]=chunk;
            }
            var world=destruction(null);center(world,chunks[0]);setWriteZone(world,0,0,1);
            for(int pass=0;pass<2;pass++) {
                for(int x=0;x<2;x++) {
                    boolean readable;try{chunks[x].getNoiseBiome(x*4,Integer.MAX_VALUE,0);readable=true;}catch(IllegalStateException error){readable=false;}
                    out.println("lifecycle_status "+pass+" "+x+" "+chunks[x].getHighestGeneratedStatus().getName()+" "+readable);
                }
                if(pass!=0)break;
                for(int[] step:new int[][]{{16,0,0},{16,-1,1},{16,0,0},{0,-64,3},{16,0,2},{16,0,0},{0,-1,4},{16,0,0},{0,-65,0},{0,-1,3},{16,Integer.MAX_VALUE,0}}) {
                    boolean accepted=world.ensureCanWrite(new BlockPos(step[0],step[1],0));
                    if(accepted)switch(step[2]) {
                        case 1->chunks[0].setPersistedStatus(ChunkStatus.FEATURES);
                        case 2->chunks[0].setBelowZeroRetrogen(data);
                        case 3->chunks[0].setBelowZeroRetrogen(null);
                        case 4->chunks[0].setPersistedStatus(stage);
                    }
                    out.println("lifecycle_write "+step[0]+" "+step[1]+" "+step[2]+" "+accepted);
                }
            }
        }
    }
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        if(args[0].equals("export")){VanillaHeightmapOracle.main(args);return;}
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        Path dir=Path.of(args[1]);
        Holder<Biome> biome=Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},Biomes.PLAINS);
        var ids=new IdMapper<Holder<Biome>>();ids.add(biome);
        var factory=new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),Blocks.AIR.defaultBlockState(),null,Strategy.createForBiomes(ids),biome,null);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());
        bindFluidTags(JsonParser.parseString(Files.readString(dir.resolve("fluid-tags.json"))).getAsJsonObject());
        var queryTags=new ArrayList<>(new TreeSet<>(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject().keySet()));queryTags.add("horizon:absent");
        var contextMethod=VanillaPlacementOracle.class.getDeclaredMethod("context",JsonObject.class);contextMethod.setAccessible(true);
        var bounds=new JsonObject();bounds.add("context",JsonParser.parseString("[-64,64,0]"));
        var generator=((PlacementContext)contextMethod.invoke(null,bounds)).generator();
        var ops=RegistryOps.create(JsonOps.INSTANCE,new RegistryOps.RegistryInfoLookup(){public <T> Optional<HolderGetter<T>> lookup(ResourceKey<? extends Registry<? extends T>> key){
            if(key.equals(Registries.BLOCK))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(BuiltInRegistries.BLOCK);
            if(key.equals(Registries.FLUID))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(BuiltInRegistries.FLUID);
            return Optional.empty();
        }});
        var fluids=new Fluid[]{Fluids.EMPTY,Fluids.FLOWING_WATER,Fluids.WATER,Fluids.FLOWING_LAVA,Fluids.LAVA};
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            var zoneWorld=destruction(null);
            var chunkZone=net.minecraft.server.level.WorldGenRegion.class.getDeclaredMethod("isWithinWriteZone",int.class,int.class);chunkZone.setAccessible(true);
            for(var value:JsonParser.parseString(Files.readString(dir.resolve("zones.json"))).getAsJsonArray()) {
                var row=value.getAsJsonArray();setWriteZone(zoneWorld,row.get(0).getAsInt(),row.get(1).getAsInt(),row.get(2).getAsInt());
                int x=row.get(3).getAsInt(),z=row.get(4).getAsInt();out.println("zone "+chunkZone.invoke(zoneWorld,x,z)+" "+zoneWorld.isWithinWriteZone(new BlockPos(x,Integer.MIN_VALUE,z)));
            }
            var heightChunk=new ProtoChunk(new ChunkPos(0,0),UpgradeData.EMPTY,LevelHeightAccessor.create(-64,384),factory,null);
            center(zoneWorld,heightChunk);setWriteZone(zoneWorld,0,0,0);
            var upgrade=retrogen();
            for(var value:JsonParser.parseString(Files.readString(dir.resolve("heights.json"))).getAsJsonArray()) {
                var row=value.getAsJsonArray();int y=row.get(2).getAsInt();var p=new BlockPos(0,y,0);
                boolean interval=LevelHeightAccessor.create(row.get(0).getAsInt(),row.get(1).getAsInt()).isInsideBuildHeight(y);
                heightChunk.setBelowZeroRetrogen(null);boolean normal=zoneWorld.ensureCanWrite(p);
                heightChunk.setBelowZeroRetrogen(upgrade);boolean upgrading=zoneWorld.ensureCanWrite(p);
                out.println("heightgate "+interval+" "+normal+" "+upgrading);
            }
            upgradeLifecycle(factory,out);
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();long seed=f.get("seed").getAsLong();
                var features=new ArrayList<Feature>();
                for(var value:f.getAsJsonArray("features"))features.add(Feature.DIRECT_CODEC.parse(ops,value).getOrThrow());
                RandomSource random=f.get("legacy").getAsBoolean()?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                if(f.get("worldgen").getAsBoolean())random=new WorldgenRandom(random);
                var chunks=new TreeMap<ChunkPos,ProtoChunk>(Comparator.<ChunkPos>comparingInt(p->p.x()).thenComparingInt(p->p.z()));
                for(int cx=-1;cx<=1;cx++)for(int cz=-1;cz<=1;cz++) {
                    var cp=new ChunkPos(cx,cz);var chunk=new ProtoChunk(cp,UpgradeData.EMPTY,LevelHeightAccessor.create(-64,64),factory,null);
                    chunk.setPersistedStatus(f.get("stage").getAsString().equals("terrain")?ChunkStatus.TERRAIN:ChunkStatus.EMPTY);
                    for(int x=0;x<16;x++)for(int z=0;z<16;z++) {
                        chunk.setBlockState(new BlockPos((cx<<4)+x,-56,(cz<<4)+z),Blocks.STONE.defaultBlockState(),3);
                        if((x+z)%3!=0)chunk.setBlockState(new BlockPos((cx<<4)+x,-48,(cz<<4)+z),Blocks.DIRT.defaultBlockState(),3);
                    }
                    chunks.put(cp,chunk);
                }
                var blockTicks=new WorldGenTickAccess<Block>(p->chunk(chunks,p.getX(),p.getZ()).getBlockTicks());
                var fluidTicks=new WorldGenTickAccess<Fluid>(p->chunk(chunks,p.getX(),p.getZ()).getFluidTicks());
                long[] tickOrder={f.get("tick_order").getAsLong()};
                int[] calls={0};int fail=f.get("fail").getAsInt(),reject=f.get("reject").getAsInt();
                NativeDestruction[] destroyer={null};
                var world=(WorldGenLevel)Proxy.newProxyInstance(WorldGenLevel.class.getClassLoader(),new Class<?>[]{WorldGenLevel.class},(proxy,method,values)->switch(method.getName()) {
                    case "destroyBlock"->{
                        if(method.isDefault())yield InvocationHandler.invokeDefault(proxy,method,values);
                        yield destroyer[0].destroyBlock((BlockPos)values[0],(Boolean)values[1],(net.minecraft.world.entity.Entity)values[2],(Integer)values[3]);
                    }
                    case "getBlockTicks"->blockTicks;
                    case "getFluidTicks"->fluidTicks;
                    case "getGameTime"->f.get("game_time").getAsLong();
                    case "nextSubTickCount"->tickOrder[0]++;
                    case "scheduleTick","createTick","isEmptyBlock","isWaterAt"->InvocationHandler.invokeDefault(proxy,method,values);
                    case "getMinY"->-64;
                    case "getSeaLevel"->0;
                    case "getHeight"->{
                        if(values==null||values.length==0)yield 64;
                        var kind=(Heightmap.Types)values[0];int x=(Integer)values[1],z=(Integer)values[2];yield chunk(chunks,x,z).getHeight(kind,x&15,z&15)+1;
                    }
                    case "getMaxY","isInsideBuildHeight","isOutsideBuildHeight","setBlockAndUpdate"->InvocationHandler.invokeDefault(proxy,method,values);
                    case "getFluidState"->{var p=(BlockPos)values[0];yield chunk(chunks,p.getX(),p.getZ()).getFluidState(p);}
                    case "getBlockState"->{var p=(BlockPos)values[0];yield chunk(chunks,p.getX(),p.getZ()).getBlockState(p);}
                    case "setBlock"->{
                        var p=(BlockPos)values[0];var state=(BlockState)values[1];int flags=(Integer)values[2];
                        if(!f.get("write_zone").isJsonNull()&&!destroyer[0].isWithinWriteZone(p))yield false;
                        if(!destroyer[0].ensureCanWrite(p))yield false;
                        out.println("write "+p.getX()+" "+p.getY()+" "+p.getZ()+" "+Block.getId(state)+" "+flags);
                        int call=calls[0]++;if(call==fail)throw new IllegalArgumentException("injected write failure");
                        boolean result=reject==0||call%reject!=0;
                        if(result)chunk(chunks,p.getX(),p.getZ()).setBlockState(p,state,flags);
                        out.println("written "+result);yield result;
                    }
                    default->throw new AssertionError("unexpected world method "+method);
                });
                destroyer[0]=destruction(world);
                var centerChunk=chunks.get(new ChunkPos(0,0));
                if(f.get("upgrading").getAsBoolean())centerChunk.setBelowZeroRetrogen(upgrade);
                center(destroyer[0],centerChunk);setWriteZone(destroyer[0],0,0,Integer.MAX_VALUE);
                if(!f.get("write_zone").isJsonNull()){var zone=f.getAsJsonArray("write_zone");setWriteZone(destroyer[0],zone.get(0).getAsInt(),zone.get(1).getAsInt(),zone.get(2).getAsInt());}
                var modifiers=new ArrayList<PlacementModifier>();for(var value:f.getAsJsonArray("placement"))modifiers.add(PlacementModifier.CODEC.parse(JsonOps.INSTANCE,value).getOrThrow());
                var placer=new FeaturePlacer(world,generator);
                for(int repeat=0;repeat<2;repeat++) {
                    for(var feature:features) {
                        calls[0]=0;
                        try {out.println("place "+placer.place(new PlacedFeature(Holder.direct(feature),modifiers),random,new BlockPos(15,-40,15)));}catch(RuntimeException error){out.println("place error");}
                        out.println("next "+random.nextLong());
                        for(int[] p:new int[][]{{0,0},{15,15},{16,16},{-1,-1}})for(var kind:Heightmap.Types.values())out.println("height "+kind.name()+" "+p[0]+" "+p[1]+" "+(chunk(chunks,p[0],p[1]).getHeight(kind,p[0]&15,p[1]&15)+1));
                    }
                    out.println("repeat "+repeat);
                }
                for(int y:new int[]{Integer.MIN_VALUE,-65,-64,-63,-1,0,1,319,320,Integer.MAX_VALUE})for(int x:new int[]{0,16}) {
                    calls[0]=0;boolean result=world.setBlock(new BlockPos(x,y,0),Blocks.STONE.defaultBlockState(),2);
                    out.println("heightwrite "+x+" "+y+" "+result);
                }
                for(var raw:f.getAsJsonArray("tick_commands")){
                    var command=raw.getAsJsonArray();String kind=command.get(0).getAsString();int id=command.get(1).getAsInt();var xyz=command.get(2).getAsJsonArray();var p=new BlockPos(xyz.get(0).getAsInt(),xyz.get(1).getAsInt(),xyz.get(2).getAsInt());int delay=command.get(3).getAsInt();
                    out.println("tickrequest "+kind+" "+id);
                    try{if(kind.equals("block"))world.scheduleTick(p,Block.stateById(id).getBlock(),delay);else world.scheduleTick(p,fluids[id],delay);out.println("tickresult ok");}catch(RuntimeException error){out.println("tickresult error");}
                    out.println("tickorder "+tickOrder[0]);
                }
                for(var value:f.getAsJsonArray("probe_states")) {
                    int id=value.getAsInt();int[] xyz=new int[][]{{0,-64,0},{-1,-49,-1},{16,-1,16},{31,-32,31}}[id%4];var p=new BlockPos(xyz[0],xyz[1],xyz[2]);
                    out.println("probe "+id);
                    for(var state:new BlockState[]{Block.stateById(id),Blocks.AIR.defaultBlockState()}) {
                        chunk(chunks,p.getX(),p.getZ()).setBlockState(p,state,3);
                        queryState(world,p,queryTags,out);
                    }
                }
                for(int[] xyz:new int[][]{{0,-65,0},{31,0,31},{-1,Integer.MIN_VALUE,-1},{16,Integer.MAX_VALUE,16},{-17,-40,0},{32,-40,0},{Integer.MAX_VALUE,Integer.MIN_VALUE,0}})queryState(world,new BlockPos(xyz[0],xyz[1],xyz[2]),queryTags,out);
                for(var entry:chunks.entrySet()) {
                    var cp=entry.getKey();var chunk=entry.getValue();out.println("chunk "+cp.x()+" "+cp.z());
                    out.println("blockticks "+chunk.getBlockTicks().count());
                    for(var tick:((SerializableTickContainer<Block>)chunk.getBlockTicks()).pack(f.get("game_time").getAsLong()))out.println("bt "+Block.getId(tick.type().defaultBlockState())+" "+tick.pos().getX()+" "+tick.pos().getY()+" "+tick.pos().getZ()+" "+tick.delay()+" "+tick.priority().getValue());
                    out.println("fluidticks "+chunk.getFluidTicks().count());
                    for(var tick:((SerializableTickContainer<Fluid>)chunk.getFluidTicks()).pack(f.get("game_time").getAsLong()))out.println("ft "+BuiltInRegistries.FLUID.getKey(tick.type())+" "+tick.pos().getX()+" "+tick.pos().getY()+" "+tick.pos().getZ()+" "+tick.delay()+" "+tick.priority().getValue());
                    for(var kind:Heightmap.Types.values()) {
                        if(chunk.hasPrimedHeightmap(kind)){out.println("map "+kind.name());for(long word:chunk.getOrCreateHeightmapUnprimed(kind).getRawData())out.println(Long.toUnsignedString(word,16));}
                        else out.println("missing "+kind.name());
                    }
                    for(int i=0;i<4;i++){out.println("section "+i);var section=chunk.getSection(i);for(int y=0;y<16;y++)for(int z=0;z<16;z++)for(int x=0;x<16;x++)out.println(Block.getId(section.getBlockState(x,y,z)));}
                }
                out.println("tail "+random.nextLong());
            }
        }
    }
}
