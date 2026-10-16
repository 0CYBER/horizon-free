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
        Trace trace;BlockEntity entity;boolean liquidTrace;
        Chunk(int x,int z,Trace trace,BlockEntity entity){super(new ChunkPos(x,z),UpgradeData.EMPTY,LevelHeightAccessor.create(-16,48),factory,null);this.trace=trace;this.entity=entity;}
        @Override public BlockState getBlockState(BlockPos p){trace.query("block "+p.getX()+" "+p.getY()+" "+p.getZ());return super.getBlockState(p);}
        @Override public BlockEntity getBlockEntity(BlockPos p){trace.query("entity chunk "+p.getX()+" "+p.getY()+" "+p.getZ());return entity;}
        @Override public LevelChunkSection getSection(int index){if(liquidTrace)trace.query("liquid-section "+(index-1));return super.getSection(index);}
        void write(BlockPos p,int id){getSection(getSectionIndex(p.getY())).setBlockState(p.getX()&15,p.getY()&15,p.getZ()&15,Block.stateById(id),false);}
    }
    static String boxText(String label,AABB b){return label+String.format(" %016x %016x %016x %016x %016x %016x",Double.doubleToLongBits(b.minX),Double.doubleToLongBits(b.minY),Double.doubleToLongBits(b.minZ),Double.doubleToLongBits(b.maxX),Double.doubleToLongBits(b.maxY),Double.doubleToLongBits(b.maxZ));}
    static VoxelShape boxShape(JsonElement input){var v=input.getAsJsonArray();return Shapes.box(number(v.get(0)),number(v.get(1)),number(v.get(2)),number(v.get(3)),number(v.get(4)),number(v.get(5)));}
    static final class StepBorder extends net.minecraft.world.level.border.WorldBorder {
        Trace trace;VoxelShape shape;
        @Override public boolean isInsideCloseToBorder(net.minecraft.world.entity.Entity entity,AABB bounds){trace.query(boxText("border-test",bounds));return shape!=null;}
        @Override public VoxelShape getCollisionShape(){trace.query("border-shape");return shape;}
    }
    static final class StepLevel extends net.minecraft.server.level.ServerLevel {
        Trace trace;StepBorder border;List<VoxelShape> entities;CollisionGetter world;CollisionContext context;LevelReader liquidWorld;Collection<Chunk> chunks;
        boolean filterEntities;
        private StepLevel(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public boolean containsAnyLiquid(AABB bounds){for(var c:chunks)c.liquidTrace=true;try{return liquidWorld.containsAnyLiquid(bounds);}finally{for(var c:chunks)c.liquidTrace=false;}}
        @Override public net.minecraft.world.level.border.WorldBorder getWorldBorder(){trace.query("border");return border;}
        @Override public List<VoxelShape> getEntityCollisions(net.minecraft.world.entity.Entity entity,AABB bounds){trace.query(boxText("entities",bounds));return filterEntities ? entities.stream().filter(s -> s.bounds().intersects(bounds.inflate(1.0e-7))).toList() : entities;}
        @Override public Iterable<VoxelShape> getBlockCollisions(net.minecraft.world.entity.Entity entity,AABB bounds){return ()->new BlockCollisions<VoxelShape>(world,context,bounds,false,(p,shape)->shape);}
    }
    static final class StepEntity extends net.minecraft.world.entity.decoration.ArmorStand {
        float step;boolean grounded;
        private StepEntity(){super(null,null);}
        @Override public float maxUpStep(){return step;}
        @Override public boolean onGround(){return grounded;}
        void configure(StepLevel level,AABB bounds,float step,boolean grounded){setLevel(level);setBoundingBox(bounds);this.step=step;this.grounded=grounded;}
    }
    static final class EdgePlayer extends net.minecraft.world.entity.player.Player {
        float step;boolean grounded,staying;
        private EdgePlayer(){super(null,null);}
        @Override public GameType gameMode(){return GameType.SURVIVAL;}
        @Override public float maxUpStep(){return step;}
        @Override public boolean onGround(){return grounded;}
        @Override protected boolean isStayingOnGroundSurface(){return staying;}
        void configure(StepLevel level,AABB bounds,float step,boolean grounded,boolean staying){setLevel(level);setBoundingBox(bounds);this.step=step;this.grounded=grounded;this.staying=staying;}
        Vec3 backoff(Vec3 delta,net.minecraft.world.entity.MoverType mover){return maybeBackOffFromEdge(delta,mover);}
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
        var abilities=net.minecraft.world.entity.player.Player.class.getDeclaredField("abilities");abilities.setAccessible(true);
        var collide=net.minecraft.world.entity.Entity.class.getDeclaredMethod("collide",Vec3.class);collide.setAccessible(true);
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
                if(f.has("movement")){
                    var m=f.getAsJsonObject("movement");var level=(StepLevel)allocate.invoke(unsafe,StepLevel.class);level.trace=trace;level.world=world;level.context=context;
                    level.chunks=chunks.values();level.liquidWorld=(LevelReader)Proxy.newProxyInstance(LevelReader.class.getClassLoader(),new Class<?>[]{LevelReader.class},(proxy,method,values)->{switch(method.getName()){case "getMinSectionY":trace.query("liquid-min-section");return -1;case "getMaxSectionY":trace.query("liquid-max-section");return 1;case "getMinY":trace.query("liquid-min-y");return -16;case "getMaxY":trace.query("liquid-max-y");return 31;case "getChunk":if(values.length!=2)throw new AssertionError(method);int x=(int)values[0],z=(int)values[1];trace.query("liquid-chunk "+x+" "+z);var c=chunks.get(key(x,z));if(c==null)throw new Stop();return c;default:if(method.isDefault())return InvocationHandler.invokeDefault(proxy,method,values);throw new AssertionError(method);}});

                    level.entities=new ArrayList<>();for(var shape:m.getAsJsonArray("entities"))level.entities.add(boxShape(shape));
                    level.border=new StepBorder();level.border.trace=trace;level.border.shape=m.get("border").isJsonNull()?null:boxShape(m.get("border"));
                    var entity=(StepEntity)allocate.invoke(unsafe,StepEntity.class);entity.configure(level,bounds,(float)number(m.get("step")),m.get("grounded").getAsBoolean());var delta=m.getAsJsonArray("delta");
                    if(m.has("edge")){
                        var e=m.getAsJsonObject("edge");var player=(EdgePlayer)allocate.invoke(unsafe,EdgePlayer.class);player.configure(level,bounds,(float)number(m.get("step")),m.get("grounded").getAsBoolean(),e.get("staying").getAsBoolean());player.fallDistance=number(e.get("fall"));var a=new net.minecraft.world.entity.player.Abilities();a.flying=e.get("flying").getAsBoolean();abilities.set(player,a);
                        try{var result=player.backoff(new Vec3(number(delta.get(0)),number(delta.get(1)),number(delta.get(2))),net.minecraft.world.entity.MoverType.valueOf(e.get("mover").getAsString()));out.printf("edge %016x %016x %016x%n",Double.doubleToLongBits(result.x),Double.doubleToLongBits(result.y),Double.doubleToLongBits(result.z));}catch(Stop error){out.println("error");}out.println("done edge "+trace.calls);continue;
                    }
                    if(m.has("free")&&m.get("free").getAsBoolean()){try{out.println("free "+entity.isFree(number(delta.get(0)),number(delta.get(1)),number(delta.get(2))));}catch(Stop error){out.println("error");}out.println("done free "+trace.calls);continue;}
                    if(m.has("check")&&m.get("check").getAsBoolean()){try{out.println("clear "+level.noCollision(entity,bounds));}catch(Stop error){out.println("error");}out.println("done check "+trace.calls);continue;}
                    try{var result=(Vec3)collide.invoke(entity,new Vec3(number(delta.get(0)),number(delta.get(1)),number(delta.get(2))));out.printf("movement %016x %016x %016x%n",Double.doubleToLongBits(result.x),Double.doubleToLongBits(result.y),Double.doubleToLongBits(result.z));}
                    catch(InvocationTargetException error){if(error.getCause() instanceof Stop)out.println("error");else throw error;}
                    out.println("done movement "+trace.calls);continue;
                }
                var scan=new BlockCollisions<VoxelShape>(world,context,bounds,f.get("suffocating").getAsBoolean(),(p,s)->{out.println("hit "+p.getX()+" "+p.getY()+" "+p.getZ());return s;});int hits=0;
                try{while(hits<f.get("limit").getAsInt()&&scan.hasNext()){emit.invoke(null,out,scan.next());hits++;}}catch(Stop error){out.println("error");}
                out.println("done "+hits+" "+trace.calls);
            }
        }
    }
}
