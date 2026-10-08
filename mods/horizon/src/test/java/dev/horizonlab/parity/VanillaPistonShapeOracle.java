package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import net.minecraft.core.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.piston.PistonMovingBlockEntity;
import net.minecraft.world.phys.shapes.*;
/** Executes native moving-piston collision and interpolation with actual registry states. */
public final class VanillaPistonShapeOracle {
 private static final class Stop extends RuntimeException {}
 private static Field field(String name)throws Exception {var f=PistonMovingBlockEntity.class.getDeclaredField(name);f.setAccessible(true);return f;}
 private static float number(JsonElement value){return Float.intBitsToFloat(Integer.parseUnsignedInt(value.getAsString(),16));}
 @SuppressWarnings("unchecked") public static void main(String[] args)throws Exception {
  net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
  if(args[0].equals("export")){VanillaCollisionOracle.main(new String[]{"generate",args[1]});VanillaBlockStateCodecOracle.main(new String[]{"export",dir.resolve("facts.json").toString()});return;}
  var progress=field("progress");var previous=field("progressO");var noclip=(ThreadLocal<Direction>)field("NOCLIP").get(null);
  var related=PistonMovingBlockEntity.class.getDeclaredMethod("getCollisionRelatedBlockState");related.setAccessible(true);
  var movement=PistonMovingBlockEntity.class.getDeclaredMethod("getMovementDirection");movement.setAccessible(true);
  var emit=VanillaFaceSupportOracle.class.getDeclaredMethod("emit",PrintWriter.class,VoxelShape.class);emit.setAccessible(true);
  try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var inputs=Files.lines(dir.resolve("inputs.txt"))) {
   for(var line:(Iterable<String>)inputs::iterator) {
    var f=JsonParser.parseString(line).getAsJsonObject();var v=f.getAsJsonArray("position");var p=new BlockPos(v.get(0).getAsInt(),v.get(1).getAsInt(),v.get(2).getAsInt());
    var direction=Direction.values()[f.get("direction").getAsInt()];var moved=Block.stateById(f.get("state").getAsInt());
    var piston=new PistonMovingBlockEntity(p,Blocks.MOVING_PISTON.defaultBlockState(),moved,direction,f.get("extending").getAsBoolean(),f.get("source").getAsBoolean());progress.setFloat(piston,number(f.get("progress")));previous.setFloat(piston,number(f.get("previous")));float partial=number(f.get("partial"));
    int clip=f.get("noclip").getAsInt();noclip.set(clip<0?null:Direction.values()[clip]);int[] calls={0};
    var world=(BlockGetter)Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},(proxy,method,values)->{
     if(!method.getName().equals("getBlockEntity"))throw new AssertionError(method);var at=(BlockPos)values[0];out.println("entity "+at.getX()+" "+at.getY()+" "+at.getZ());if(calls[0]++==f.get("fail").getAsInt())throw new Stop();return null;
    });
    try {
     out.printf("motion %d %08x %08x %08x %08x%n",((Direction)movement.invoke(piston)).ordinal(),Float.floatToIntBits(piston.getProgress(partial)),Float.floatToIntBits(piston.getXOff(partial)),Float.floatToIntBits(piston.getYOff(partial)),Float.floatToIntBits(piston.getZOff(partial)));
     out.println("related "+Block.getId((net.minecraft.world.level.block.state.BlockState)related.invoke(piston)));
     if(!f.get("kinematic").getAsBoolean())try {emit.invoke(null,out,piston.getCollisionShape(world,p));}catch(Stop error){out.println("shape error");}
     out.println("queries "+calls[0]);
    } finally {noclip.remove();}
   }
  }
 }
}
