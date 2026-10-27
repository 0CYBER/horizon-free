package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.monster.zombie.Zombie;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.piston.*;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;
/**
 * The real moving-piston push maths: PistonMath.getMovementArea,
 * moveByPositionAndProgress, getMovement, the moveCollidedEntities loop over
 * the block's own boxes, and moveStuckEntities' box and matchesStickyCritera.
 * fixEntityWithinPistonBase is left out: it moves the entity
 * (`Entity.move(MoverType.PISTON, ...)`), which needs a live level, so the
 * Rust side is covered by its own unit tests.
 */
public final class VanillaPistonPushOracle {
 private static float number(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
 private static double dbl(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
 private static AABB box(JsonElement v){var a=v.getAsJsonArray();return new AABB(dbl(a.get(0)),dbl(a.get(1)),dbl(a.get(2)),dbl(a.get(3)),dbl(a.get(4)),dbl(a.get(5)));}
 private static void line(PrintWriter out,String name,AABB b){out.printf(name+" %016x %016x %016x %016x %016x %016x%n",Double.doubleToLongBits(b.minX),Double.doubleToLongBits(b.minY),Double.doubleToLongBits(b.minZ),Double.doubleToLongBits(b.maxX),Double.doubleToLongBits(b.maxY),Double.doubleToLongBits(b.maxZ));}
 private static Field field(Class<?> c,String name)throws Exception{var f=c.getDeclaredField(name);f.setAccessible(true);return f;}
 private static Method method(Class<?> c,String name,Class<?>...types)throws Exception{var m=c.getDeclaredMethod(name,types);m.setAccessible(true);return m;}
 @SuppressWarnings("unchecked") public static void main(String[] args)throws Exception {
  net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
  var allocator=Class.forName("sun.misc.Unsafe");var uf=allocator.getDeclaredField("theUnsafe");uf.setAccessible(true);var unsafe=uf.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
  var progress=field(PistonMovingBlockEntity.class,"progress");
  var movedBy=method(PistonMovingBlockEntity.class,"moveByPositionAndProgress",BlockPos.class,AABB.class,PistonMovingBlockEntity.class);
  var extended=method(PistonMovingBlockEntity.class,"getExtendedProgress",float.class);
  var getMovement=method(PistonMovingBlockEntity.class,"getMovement",AABB.class,Direction.class,AABB.class);
  var matches=method(PistonMovingBlockEntity.class,"matchesStickyCritera",AABB.class,Entity.class,BlockPos.class);
  var movementDir=method(PistonMovingBlockEntity.class,"getMovementDirection");
  var position=field(Entity.class,"position");var onGroundField=field(Entity.class,"onGround");var supporting=field(Entity.class,"mainSupportingBlockPos");var bbField=field(Entity.class,"bb");
  var faces=new Direction[]{Direction.EAST,Direction.WEST,Direction.UP,Direction.DOWN,Direction.SOUTH,Direction.NORTH};
  try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var inputs=Files.lines(dir.resolve("inputs.txt"))) {
   for(var line:(Iterable<String>)inputs::iterator) {
    var f=JsonParser.parseString(line).getAsJsonObject();var v=f.getAsJsonArray("pos");var p=new BlockPos(v.get(0).getAsInt(),v.get(1).getAsInt(),v.get(2).getAsInt());
    var direction=Direction.values()[f.get("direction").getAsInt()];
    var piston=new PistonMovingBlockEntity(p,Blocks.MOVING_PISTON.defaultBlockState(),Blocks.STONE.defaultBlockState(),direction,f.get("extending").getAsBoolean(),f.get("source").getAsBoolean());
    float current=number(f.get("progress"));
    progress.setFloat(piston,current);
    float newProgress=number(f.get("new_progress"));
    double deltaProgress=newProgress-current;
    Direction movement=(Direction)movementDir.invoke(piston);
    AABB blockBox=box(f.get("block_box"));
    AABB entityBox=box(f.get("entity"));
    AABB movementBox=box(f.get("movement_box"));
    out.printf("extended %08x%n",Float.floatToIntBits((float)extended.invoke(piston,current)));
    AABB origin=(AABB)movedBy.invoke(null,p,blockBox,piston);
    line(out,"origin",origin);
    line(out,"area",PistonMath.getMovementArea(blockBox,movement,deltaProgress));
    for(var face:faces)out.printf("movement %s %016x%n",face.getSerializedName(),Double.doubleToLongBits((double)getMovement.invoke(null,movementBox,face,entityBox)));
    double delta=0.0;boolean capped=false;
    for(var value:f.getAsJsonArray("boxes")){
     AABB movingAABB=PistonMath.getMovementArea((AABB)movedBy.invoke(null,p,box(value),piston),movement,deltaProgress);
     if(movingAABB.intersects(entityBox)){delta=Math.max(delta,(double)getMovement.invoke(null,movingAABB,movement,entityBox));if(delta>=deltaProgress){capped=true;break;}}
    }
    double collided=delta<=0.0?0.0:Math.min(delta,deltaProgress)+0.01;
    out.printf("collided %016x %b%n",Double.doubleToLongBits(collided),capped);
    line(out,"search",PistonMath.getMovementArea(origin,movement,deltaProgress).minmax(origin));
    AABB sticky=(AABB)movedBy.invoke(null,p,new AABB(0.0,dbl(f.get("sticky_top")),0.0,1.0,1.5000010000000001,1.0),piston);
    line(out,"sticky",sticky);
    boolean pushable=f.get("reaction").getAsInt()==0;
    var entity=(Entity)allocate.invoke(unsafe,pushable?Zombie.class:Marker.class);
    position.set(entity,new Vec3(dbl(f.get("entity_x")),0.0,dbl(f.get("entity_z"))));
    bbField.set(entity,entityBox);
    onGroundField.setBoolean(entity,f.get("on_ground").getAsBoolean());
    supporting.set(entity,f.get("supported").getAsBoolean()?Optional.of(p):Optional.empty());
    out.printf("matches %b%n",(boolean)matches.invoke(null,sticky,entity,p));
   }
  }
 }
}