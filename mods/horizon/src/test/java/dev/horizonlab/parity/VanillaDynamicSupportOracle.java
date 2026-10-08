package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import net.minecraft.core.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.entity.*;
import net.minecraft.world.level.block.piston.PistonMovingBlockEntity;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.phys.shapes.*;
/** Native uncached support selection; block-entity lifecycle is outside these snapshots. */
public final class VanillaDynamicSupportOracle {
 private static final class Stop extends RuntimeException {}
 @SuppressWarnings("unchecked") public static void main(String[] args)throws Exception {
  net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
  if(args[0].equals("export")) {
   VanillaCollisionOracle.main(new String[]{"generate",args[1]});
   VanillaBlockStateCodecOracle.main(new String[]{"export",dir.resolve("facts.json").toString()});
   var cache=BlockBehaviour.BlockStateBase.class.getDeclaredField("cache");cache.setAccessible(true);var ids=new JsonArray();for(var state:Block.BLOCK_STATE_REGISTRY)if(cache.get(state)==null)ids.add(Block.getId(state));Files.writeString(dir.resolve("uncached-states.json"),ids.toString());return;
  }
  var allocator=Class.forName("sun.misc.Unsafe");var access=allocator.getDeclaredField("theUnsafe");access.setAccessible(true);var unsafe=access.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
  var progress=PistonMovingBlockEntity.class.getDeclaredField("progress");progress.setAccessible(true);var clipField=PistonMovingBlockEntity.class.getDeclaredField("NOCLIP");clipField.setAccessible(true);var noclip=(ThreadLocal<Direction>)clipField.get(null);
  var shulkerOld=ShulkerBoxBlockEntity.class.getDeclaredField("progressOld");shulkerOld.setAccessible(true);var shulkerProgress=ShulkerBoxBlockEntity.class.getDeclaredField("progress");shulkerProgress.setAccessible(true);
  var animation=ShulkerBoxBlockEntity.class.getDeclaredField("animationStatus");animation.setAccessible(true);
  var emit=VanillaFaceSupportOracle.class.getDeclaredMethod("emit",PrintWriter.class,VoxelShape.class);emit.setAccessible(true);
  try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var inputs=Files.lines(dir.resolve("inputs.txt"))) {
   for(var line:(Iterable<String>)inputs::iterator) {
    var f=JsonParser.parseString(line).getAsJsonObject();var state=Block.stateById(f.get("state").getAsInt());var xyz=f.getAsJsonArray("position");var p=new BlockPos(xyz.get(0).getAsInt(),xyz.get(1).getAsInt(),xyz.get(2).getAsInt());String mode=f.get("entity").getAsString();
    Object entity=null;
    if(!mode.equals("none")) {
     if(mode.equals("other"))entity=allocate.invoke(unsafe,state.getBlock() instanceof ShulkerBoxBlock?PistonMovingBlockEntity.class:ShulkerBoxBlockEntity.class);
     else if(mode.equals("piston")) {
      var data=f.getAsJsonObject("piston");var moving=new PistonMovingBlockEntity(p,state,Block.stateById(data.get("state").getAsInt()),Direction.values()[data.get("direction").getAsInt()],data.get("extending").getAsBoolean(),data.get("source").getAsBoolean());
      progress.setFloat(moving,Float.intBitsToFloat(Integer.parseUnsignedInt(data.get("progress").getAsString(),16)));entity=moving;int clip=data.get("noclip").getAsInt();noclip.set(clip<0?null:Direction.values()[clip]);
     }
     else {entity=allocate.invoke(unsafe,ShulkerBoxBlockEntity.class);Object status=null;for(var value:animation.getType().getEnumConstants())if(((Enum<?>)value).name().equals(mode.toUpperCase(java.util.Locale.ROOT)))status=value;if(status==null)throw new AssertionError(mode);animation.set(entity,status);}
    }
    if(entity instanceof ShulkerBoxBlockEntity && f.has("shulker_progress")){var data=f.getAsJsonArray("shulker_progress");shulkerOld.setFloat(entity,Float.intBitsToFloat(Integer.parseUnsignedInt(data.get(0).getAsString(),16)));shulkerProgress.setFloat(entity,Float.intBitsToFloat(Integer.parseUnsignedInt(data.get(1).getAsString(),16)));}
    Object selected=entity;int[] calls={0};
    var world=(BlockGetter)Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},(proxy,method,values)->{
     if(!method.getName().equals("getBlockEntity"))throw new AssertionError(method);
     var at=(BlockPos)values[0];out.println("entity "+at.getX()+" "+at.getY()+" "+at.getZ());if(calls[0]++==f.get("fail").getAsInt())throw new Stop();return selected;
    });
    var offset=state.getOffset(p);out.printf("offset %016x %016x %016x%n",Double.doubleToRawLongBits(offset.x),Double.doubleToRawLongBits(offset.y),Double.doubleToRawLongBits(offset.z));
    try {emit.invoke(null,out,f.has("collision")&&f.get("collision").getAsBoolean()?state.getCollisionShape(world,p):state.getBlockSupportShape(world,p));}catch(Stop error){out.println("shape error");}
    for(var direction:Direction.values())for(var type:SupportType.values()) {
     try {out.println("support "+direction.ordinal()+" "+type.ordinal()+" "+state.isFaceSturdy(world,p,direction,type));}catch(Stop error){out.println("support "+direction.ordinal()+" "+type.ordinal()+" error");}
    }
    out.println("queries "+calls[0]);noclip.remove();
   }
  }
 }
}
