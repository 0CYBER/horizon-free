package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.registries.*;import net.minecraft.nbt.*;import net.minecraft.world.level.*;import net.minecraft.world.level.biome.*;import net.minecraft.world.level.block.*;import net.minecraft.world.level.block.state.*;import net.minecraft.world.level.chunk.*;import net.minecraft.world.phys.AABB;
/** Native containsAnyLiquid through actual chunk sections and palette filtering. */
public final class VanillaFluidOccupancyOracle {
    static final class Stop extends RuntimeException {}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static AABB bounds(JsonArray a){return new AABB(number(a.get(0)),number(a.get(1)),number(a.get(2)),number(a.get(3)),number(a.get(4)),number(a.get(5)));}
    static final class Data {PrintWriter out;int min,height,calls,fail;boolean trace;
        void query(String s){if(!trace)return;out.println(s);if(calls++==fail)throw new Stop();}}
    static final class Chunk extends ProtoChunk {Data d;
        Chunk(Data d,PalettedContainerFactory factory){super(new ChunkPos(0,0),UpgradeData.EMPTY,LevelHeightAccessor.create(d.min,d.height),factory,null);this.d=d;}
        @Override public LevelChunkSection getSection(int index){if(d!=null)d.query("section "+((d.min>>4)+index));return super.getSection(index);}}
    static void export(Path dir)throws Exception{var states=new ListTag();var defaults=new CompoundTag();var names=new JsonObject();for(var state:Block.BLOCK_STATE_REGISTRY){states.add(BlockState.FULL_CODEC.encodeStart(NbtOps.INSTANCE,state).getOrThrow());if(state==state.getBlock().defaultBlockState()){String n=BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString();defaults.putInt(n,Block.getId(state));names.addProperty(n,Block.getId(state));}}var data=new CompoundTag();data.putInt("DataVersion",net.minecraft.SharedConstants.getCurrentVersion().dataVersion().version());data.put("states",states);data.put("defaults",defaults);try(var out=new DataOutputStream(Files.newOutputStream(dir.resolve("block-registry.nbt")))){NbtIo.write(data,out);}var info=new JsonObject();info.addProperty("states",states.size());info.add("defaults",names);Files.writeString(dir.resolve("info.json"),info.toString());}
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);if(args[0].equals("export")){export(dir);return;}
        Holder<Biome> biome=Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},Biomes.PLAINS);var ids=new IdMapper<Holder<Biome>>();ids.add(biome);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var state:Block.BLOCK_STATE_REGISTRY)out.println("state "+Block.getId(state)+" "+!state.getFluidState().isEmpty());for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.out=out;d.min=f.get("min").getAsInt();d.height=f.get("height").getAsInt();var factory=new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),Block.stateById(f.get("base").getAsInt()),null,Strategy.createForBiomes(ids),biome,null);var chunk=new Chunk(d,factory);
            var world=(LevelReader)Proxy.newProxyInstance(LevelReader.class.getClassLoader(),new Class<?>[]{LevelReader.class},(proxy,method,values)->{switch(method.getName()){case "getMinSectionY":d.query("min-section");return d.min>>4;case "getMaxSectionY":d.query("max-section");return(d.min+d.height-1)>>4;case "getMinY":d.query("min-y");return d.min;case "getMaxY":d.query("max-y");return d.min+d.height-1;case "getChunk":if(values.length!=2)throw new AssertionError(method);d.query("chunk "+values[0]+" "+values[1]);return chunk;default:if(method.isDefault())return InvocationHandler.invokeDefault(proxy,method,values);throw new AssertionError(method);}});
            for(var v:f.getAsJsonArray("ops")){var op=v.getAsJsonObject();d.trace=false;for(var write:op.getAsJsonArray("writes")){var a=write.getAsJsonArray();int x=a.get(0).getAsInt(),y=a.get(1).getAsInt(),z=a.get(2).getAsInt();chunk.getSection((y-d.min)>>4).setBlockState(x&15,y&15,z&15,Block.stateById(a.get(3).getAsInt()),false);}d.calls=0;d.fail=op.get("fail").getAsInt();d.trace=true;try{out.println("result "+world.containsAnyLiquid(bounds(op.getAsJsonArray("bounds"))));}catch(Stop e){out.println("error");}out.println("queries "+d.calls);}
        }}
    }
}
