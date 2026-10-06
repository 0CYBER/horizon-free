package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.resources.Identifier;
import net.minecraft.tags.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.levelgen.Heightmap;

/** Original transport calling actual vanilla heightmap and chunk APIs. */
public final class VanillaHeightmapOracle {
    private static final Heightmap.Types[] TYPES=Heightmap.Types.values();
    private static PalettedContainerFactory factory;

    private static JsonObject facts() {
        var result=new JsonObject();var states=new JsonArray();var defaults=new JsonObject();
        for(var state:Block.BLOCK_STATE_REGISTRY) {
            var entry=new JsonArray();
            entry.add(BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString());
            entry.add(state.isAir());entry.add(!state.getFluidState().isEmpty());
            entry.add(state.toString());entry.add(BlockState.FULL_CODEC.encodeStart(JsonOps.INSTANCE,state).getOrThrow());states.add(entry);
        }
        for(var id:new TreeSet<>(BuiltInRegistries.BLOCK.keySet()))
            defaults.addProperty(id.toString(),Block.getId(BuiltInRegistries.BLOCK.getValue(id).defaultBlockState()));
        result.add("states",states);result.add("defaults",defaults);return result;
    }
    static void bindTags(JsonObject resources) {
        var registry=BuiltInRegistries.BLOCK;
        var loader=new TagLoader<Holder<Block>>((id,required)->registry.get(id).map(value->(Holder<Block>)value),"tags/block");
        var definitions=new HashMap<Identifier,List<TagLoader.EntryWithSource>>();
        for(var item:resources.entrySet()) {
            var file=TagFile.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(item.getValue().getAsString())).getOrThrow();
            var entries=new ArrayList<TagLoader.EntryWithSource>();
            for(var entry:file.entries())entries.add(new TagLoader.EntryWithSource(entry,"vanilla"));
            definitions.put(Identifier.parse(item.getKey()),entries);
        }
        var tags=new HashMap<TagKey<Block>,List<Holder<Block>>>();
        loader.build(definitions).forEach((id,values)->tags.put(TagKey.create(Registries.BLOCK,id),List.copyOf(values)));
        if(tags.size()!=resources.size())throw new AssertionError("incomplete block tags");
        registry.prepareTagReload(new TagLoader.LoadResult<>(Registries.BLOCK,tags)).apply();
    }
    private static final class Chunk extends ProtoChunk {
        final List<String> reads=new ArrayList<>();
        final boolean trace;
        Chunk(int min,int height,boolean trace) {
            super(new ChunkPos(0,0),UpgradeData.EMPTY,LevelHeightAccessor.create(min,height),factory,null);
            this.trace=trace;
        }
        @Override public BlockState getBlockState(BlockPos pos) {
            var state=super.getBlockState(pos);
            if(trace)reads.add("read "+pos.getX()+" "+pos.getY()+" "+pos.getZ()+" "+Block.getId(state));
            return state;
        }
        void write(int x,int y,int z,int id) {
            getSection(getSectionIndex(y)).setBlockState(x&15,y&15,z&15,Block.stateById(id),false);
        }
    }
    private static void fixture(JsonObject input,PrintWriter out) {
        int min=input.get("min").getAsInt(),height=input.get("height").getAsInt();
        var chunk=new Chunk(min,height,input.get("trace").getAsBoolean());
        out.println("chunk "+min+" "+height);
        for(var item:input.getAsJsonArray("initial")) {
            var a=item.getAsJsonArray();chunk.write(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt(),a.get(3).getAsInt());
        }
        for(var type:TYPES)chunk.getOrCreateHeightmapUnprimed(type);
        for(var item:input.getAsJsonArray("ops")) {
            var a=item.getAsJsonArray();String op=a.get(0).getAsString();
            switch(op) {
                case "set" -> {
                    int x=a.get(1).getAsInt(),y=a.get(2).getAsInt(),z=a.get(3).getAsInt(),id=a.get(4).getAsInt(),mask=a.get(5).getAsInt();
                    chunk.write(x,y,z,id);
                    for(int t=0;t<TYPES.length;t++)if((mask&(1<<t))!=0) {
                        try {out.println("update "+t+" "+chunk.getOrCreateHeightmapUnprimed(TYPES[t]).update(x,y,z,Block.stateById(id)));}
                        catch(RuntimeException e){out.println("update "+t+" error");}
                    }
                }
                case "update" -> {
                    int x=a.get(1).getAsInt(),y=a.get(2).getAsInt(),z=a.get(3).getAsInt(),id=a.get(4).getAsInt(),t=a.get(5).getAsInt();
                    try {out.println("update "+t+" "+chunk.getOrCreateHeightmapUnprimed(TYPES[t]).update(x,y,z,Block.stateById(id)));}
                    catch(RuntimeException e){out.println("update "+t+" error");}
                }
                case "prime" -> {
                    var selected=EnumSet.noneOf(Heightmap.Types.class);int mask=a.get(1).getAsInt();
                    for(int t=0;t<TYPES.length;t++)if((mask&(1<<t))!=0)selected.add(TYPES[t]);
                    Heightmap.primeHeightmaps(chunk,selected);out.println("prime "+mask);
                }
                case "raw" -> {
                    int t=a.get(1).getAsInt();var words=a.get(2).getAsJsonArray();long[] data=new long[words.size()];
                    for(int i=0;i<data.length;i++)data[i]=Long.parseUnsignedLong(words.get(i).getAsString(),16);
                    chunk.getOrCreateHeightmapUnprimed(TYPES[t]).setRawData(chunk,TYPES[t],data);out.println("raw "+t);
                }
                case "query" -> {
                    int x=a.get(1).getAsInt(),z=a.get(2).getAsInt();
                    for(int t=0;t<TYPES.length;t++) {
                        var map=chunk.getOrCreateHeightmapUnprimed(TYPES[t]);
                        try {out.println("height "+t+" "+x+" "+z+" "+map.getFirstAvailable(x,z)+" "+map.getHighestTaken(x,z));}
                        catch(RuntimeException e){out.println("height "+t+" "+x+" "+z+" error");}
                    }
                }
                case "dump" -> {
                    out.println("highest_section "+chunk.getHighestSectionPosition());
                    for(int t=0;t<TYPES.length;t++) {
                        long[] data=chunk.getOrCreateHeightmapUnprimed(TYPES[t]).getRawData();
                        out.println("words "+t+" "+data.length);
                        for(long word:data)out.println(Long.toUnsignedString(word,16));
                    }
                }
                default -> throw new IllegalArgumentException(op);
            }
            for(String read:chunk.reads)out.println(read);chunk.reads.clear();
        }
    }
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();Bootstrap.bootStrap();
        Path directory=Path.of(args[1]);
        if(args[0].equals("export")) {
            Files.writeString(directory,new Gson().toJson(facts()),StandardCharsets.UTF_8);return;
        }
        Holder<Biome> biome=Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},Biomes.PLAINS);
        var ids=new IdMapper<Holder<Biome>>();ids.add(biome);
        // Only in-memory section storage is exercised; biome and section codecs are not invoked.
        factory=new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),Blocks.AIR.defaultBlockState(),null,Strategy.createForBiomes(ids),biome,null);
        bindTags(JsonParser.parseString(Files.readString(directory.resolve("tags.json"))).getAsJsonObject());
        try(var out=new PrintWriter(Files.newBufferedWriter(directory.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(int t=0;t<TYPES.length;t++)out.println("type "+t+" "+TYPES[t].getSerializationKey()+" "+TYPES[t].sendToClient()+" "+TYPES[t].keepAfterWorldgen());
            for(var state:Block.BLOCK_STATE_REGISTRY) {
                int mask=0;for(int t=0;t<TYPES.length;t++)if(TYPES[t].isOpaque().test(state))mask|=1<<t;
                out.println("predicate "+Block.getId(state)+" "+mask);
            }
            for(String line:Files.readAllLines(directory.resolve("inputs.txt")))fixture(JsonParser.parseString(line).getAsJsonObject(),out);
        }
    }
}
