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
                var world=(WorldGenLevel)Proxy.newProxyInstance(WorldGenLevel.class.getClassLoader(),new Class<?>[]{WorldGenLevel.class},(proxy,method,values)->switch(method.getName()) {
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
                        out.println("write "+p.getX()+" "+p.getY()+" "+p.getZ()+" "+Block.getId(state)+" "+flags);
                        int call=calls[0]++;if(call==fail)throw new IllegalArgumentException("injected write failure");
                        boolean result=reject==0||call%reject!=0;
                        if(result)chunk(chunks,p.getX(),p.getZ()).setBlockState(p,state,flags);
                        out.println("written "+result);yield result;
                    }
                    default->throw new AssertionError("unexpected world method "+method);
                });
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
