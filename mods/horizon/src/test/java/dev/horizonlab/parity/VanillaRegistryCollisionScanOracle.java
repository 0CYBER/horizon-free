package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.entity.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;
/** Native registry behavior and BlockCollisions over resident ProtoChunk sections. */
public final class VanillaRegistryCollisionScanOracle {
    static final class Stop extends RuntimeException {}
    static final class Trace {
        PrintWriter out;int calls,fail;
        void query(String text){out.println(text);if(calls++==fail)throw new Stop();}
    }
    static PalettedContainerFactory factory;
    static class Chunk extends ProtoChunk {
        Trace trace;BlockEntity entity;
        Chunk(int x,int z,Trace trace,BlockEntity entity){super(new ChunkPos(x,z),UpgradeData.EMPTY,LevelHeightAccessor.create(-16,48),factory,null);this.trace=trace;this.entity=entity;}
        @Override public BlockState getBlockState(BlockPos p){trace.query("block "+p.getX()+" "+p.getY()+" "+p.getZ());return super.getBlockState(p);}
        @Override public BlockEntity getBlockEntity(BlockPos p){trace.query("entity chunk "+p.getX()+" "+p.getY()+" "+p.getZ());return entity;}
        void write(BlockPos p,int id){getSection(getSectionIndex(p.getY())).setBlockState(p.getX()&15,p.getY()&15,p.getZ()&15,Block.stateById(id),false);}
    }
    static long key(int x,int z){return (long)x<<32|Integer.toUnsignedLong(z);}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    public static void main(String[] args)throws Exception {
        if(args[0].equals("export")){VanillaSuffocationOracle.main(args);return;}
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());
        Holder<Biome> biome=Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},Biomes.PLAINS);var ids=new IdMapper<Holder<Biome>>();ids.add(biome);
        factory=new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),Blocks.AIR.defaultBlockState(),null,Strategy.createForBiomes(ids),biome,null);
        var allocator=Class.forName("sun.misc.Unsafe");var access=allocator.getDeclaredField("theUnsafe");access.setAccessible(true);var unsafe=access.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        var animation=ShulkerBoxBlockEntity.class.getDeclaredField("animationStatus");animation.setAccessible(true);
        var progress=ShulkerBoxBlockEntity.class.getDeclaredField("progress");progress.setAccessible(true);var previous=ShulkerBoxBlockEntity.class.getDeclaredField("progressOld");previous.setAccessible(true);
        BlockEntity[] entities={null,(BlockEntity)allocate.invoke(unsafe,ShulkerBoxBlockEntity.class),(BlockEntity)allocate.invoke(unsafe,ShulkerBoxBlockEntity.class)};
        for(int i=1;i<3;i++){
            for(var status:animation.getType().getEnumConstants())if(((Enum<?>)status).name().equals(i==1?"CLOSED":"OPENED"))animation.set(entities[i],status);
            progress.setFloat(entities[i],i==1?0:0.5f);previous.setFloat(entities[i],i==1?0:0.5f);
        }
        var emit=VanillaFaceSupportOracle.class.getDeclaredMethod("emit",PrintWriter.class,VoxelShape.class);emit.setAccessible(true);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();var trace=new Trace();trace.out=out;trace.fail=f.get("fail").getAsInt();
                var chunks=new HashMap<Long,Chunk>();var modes=f.getAsJsonArray("entities");
                for(var value:f.getAsJsonArray("chunks")){var v=value.getAsJsonArray();int x=v.get(0).getAsInt(),z=v.get(1).getAsInt();chunks.put(key(x,z),new Chunk(x,z,trace,entities[modes.get(1).getAsInt()]));}
                for(var value:f.getAsJsonArray("blocks")){var v=value.getAsJsonArray();var p=new BlockPos(v.get(0).getAsInt(),v.get(1).getAsInt(),v.get(2).getAsInt());chunks.get(key(p.getX()>>4,p.getZ()>>4)).write(p,v.get(3).getAsInt());}
                var world=(CollisionGetter)Proxy.newProxyInstance(CollisionGetter.class.getClassLoader(),new Class<?>[]{CollisionGetter.class},(proxy,method,values)->{
                    if(method.getName().equals("getChunkForCollisions")){int x=(int)values[0],z=(int)values[1];trace.query("chunk "+x+" "+z);return chunks.get(key(x,z));}
                    if(method.getName().equals("getBlockEntity")){var p=(BlockPos)values[0];trace.query("entity global "+p.getX()+" "+p.getY()+" "+p.getZ());return entities[modes.get(0).getAsInt()];}
                    throw new AssertionError(method);
                });
                int mode=f.get("context").getAsInt();var context=mode==0?CollisionContext.empty():mode==1?CollisionContext.emptyWithFluidCollisions():mode==2?CollisionContext.positionContext(number(f.get("y"))):CollisionContext.placementContext(null);
                var v=f.getAsJsonArray("box");var bounds=new AABB(number(v.get(0)),number(v.get(1)),number(v.get(2)),number(v.get(3)),number(v.get(4)),number(v.get(5)));
                var scan=new BlockCollisions<VoxelShape>(world,context,bounds,f.get("suffocating").getAsBoolean(),(p,s)->{out.println("hit "+p.getX()+" "+p.getY()+" "+p.getZ());return s;});int hits=0;
                try{while(hits<f.get("limit").getAsInt()&&scan.hasNext()){emit.invoke(null,out,scan.next());hits++;}}catch(Stop error){out.println("error");}
                out.println("done "+hits+" "+trace.calls);
            }
        }
    }
}
