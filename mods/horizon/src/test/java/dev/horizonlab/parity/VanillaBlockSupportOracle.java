package dev.horizonlab.parity;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.nbt.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.phys.shapes.*;
/** Private support-shape data export; native cache answers are the independent reference. */
public final class VanillaBlockSupportOracle {
 public static void main(String[] args)throws Exception {
  net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
  var cache=BlockBehaviour.BlockStateBase.class.getDeclaredField("cache");cache.setAccessible(true);Path dir=Path.of(args[1]);
  if(args[0].equals("export")) {
   VanillaCollisionOracle.main(new String[]{"generate",args[1]});
   var grid=VanillaCollisionOracle.class.getDeclaredMethod("grid",VoxelShape.class);grid.setAccessible(true);
   var states=new ListTag();var shapes=new ListTag();var ids=new IdentityHashMap<VoxelShape,Integer>();int[] mapping=new int[Block.BLOCK_STATE_REGISTRY.size()];
   for(var state:Block.BLOCK_STATE_REGISTRY){int id=Block.getId(state);states.add(BlockState.FULL_CODEC.encodeStart(NbtOps.INSTANCE,state).getOrThrow());if(cache.get(state)==null){mapping[id]=-1;continue;}
    var shape=state.getBlockSupportShape(EmptyBlockGetter.INSTANCE,BlockPos.ZERO);Integer shapeId=ids.get(shape);if(shapeId==null){shapeId=shapes.size();ids.put(shape,shapeId);shapes.add((CompoundTag)grid.invoke(null,shape));}mapping[id]=shapeId;
   }
   var cachedIds=new com.google.gson.JsonArray();for(int i=0;i<mapping.length;i++)if(mapping[i]>=0)cachedIds.add(i);Files.writeString(dir.resolve("support-cached-states.json"),cachedIds.toString());
   var root=new CompoundTag();root.putInt("DataVersion",net.minecraft.SharedConstants.getCurrentVersion().dataVersion().version());root.putString("Kind","block_support");root.put("states",states);root.put("shapes",shapes);root.putIntArray("cached_shapes",mapping);
   try(var out=new DataOutputStream(Files.newOutputStream(dir.resolve("cached-support.nbt")))){NbtIo.write(root,out);}return;
  }
  try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")))) {
   for(var state:Block.BLOCK_STATE_REGISTRY){int id=Block.getId(state);if(cache.get(state)==null){out.println("state "+id+" uncached");continue;}
    out.println("state "+id+" cached");
    for(var direction:Direction.values()) {
     out.print("support "+direction.ordinal());for(var type:SupportType.values())out.print(" "+state.isFaceSturdy(EmptyBlockGetter.INSTANCE,BlockPos.ZERO,direction,type));out.println(" "+state.isFaceSturdy(EmptyBlockGetter.INSTANCE,BlockPos.ZERO,direction));
    }
   }
  }
 }
}
