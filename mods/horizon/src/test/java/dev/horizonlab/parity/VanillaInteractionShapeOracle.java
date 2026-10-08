package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.nbt.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;
/** Audits static registered interaction shapes and invokes actual ray-face overrides. */
public final class VanillaInteractionShapeOracle {
    static BlockGetter world=(BlockGetter)Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},(proxy,method,values)->{if(method.isDefault())return InvocationHandler.invokeDefault(proxy,method,values);throw new AssertionError("interaction shape unexpectedly queried world: "+method);});
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static Vec3 vector(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static VoxelShape shape(JsonElement v){var result=Shapes.empty();for(var value:v.getAsJsonArray()){var a=value.getAsJsonArray();result=Shapes.or(result,Shapes.box(number(a.get(0)),number(a.get(1)),number(a.get(2)),number(a.get(3)),number(a.get(4)),number(a.get(5))));}return result;}
    static Class<?> owner(Class<?> type)throws Exception{for(;type!=null;type=type.getSuperclass())try{return type.getDeclaredMethod("getInteractionShape",BlockState.class,BlockGetter.class,BlockPos.class).getDeclaringClass();}catch(NoSuchMethodException ignored){}throw new AssertionError();}
    static void export(Path dir)throws Exception{
        var allowed=Set.of(BlockBehaviour.class,AbstractCauldronBlock.class,ComposterBlock.class,HopperBlock.class,ScaffoldingBlock.class);for(var block:BuiltInRegistries.BLOCK)if(!allowed.contains(owner(block.getClass())))throw new AssertionError("new interaction implementation");
        var grid=VanillaCollisionOracle.class.getDeclaredMethod("grid",VoxelShape.class);grid.setAccessible(true);var states=new ListTag();var defaults=new CompoundTag();var shapes=new ListTag();var identities=new IdentityHashMap<VoxelShape,Integer>();var mapping=new int[Block.BLOCK_STATE_REGISTRY.size()];
        for(var state:Block.BLOCK_STATE_REGISTRY){int id=Block.getId(state);states.add(BlockState.FULL_CODEC.encodeStart(NbtOps.INSTANCE,state).getOrThrow());if(state==state.getBlock().defaultBlockState())defaults.putInt(BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString(),id);var shape=state.getInteractionShape(world,BlockPos.ZERO);Integer index=identities.get(shape);if(index==null){index=shapes.size();identities.put(shape,index);shapes.add((CompoundTag)grid.invoke(null,shape));}mapping[id]=index;}
        var registry=new CompoundTag();registry.putInt("DataVersion",net.minecraft.SharedConstants.getCurrentVersion().dataVersion().version());registry.put("states",states);registry.put("defaults",defaults);var data=new CompoundTag();data.putInt("DataVersion",net.minecraft.SharedConstants.getCurrentVersion().dataVersion().version());data.putString("kind","interaction");data.put("states",states);data.put("shapes",shapes);data.putIntArray("cached_shapes",mapping);
        try(var out=new DataOutputStream(Files.newOutputStream(dir.resolve("block-registry.nbt")))){NbtIo.write(registry,out);}try(var out=new DataOutputStream(Files.newOutputStream(dir.resolve("interaction-shapes.nbt")))){NbtIo.write(data,out);}var info=new JsonObject();info.addProperty("states",states.size());info.addProperty("shapes",shapes.size());Files.writeString(dir.resolve("info.json"),info.toString());
    }
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);if(args[0].equals("export")){export(dir);return;}
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){var identities=new IdentityHashMap<VoxelShape,Integer>();for(var state:Block.BLOCK_STATE_REGISTRY){var shape=state.getInteractionShape(world,BlockPos.ZERO);out.println("state "+Block.getId(state)+" "+identities.computeIfAbsent(shape,k->identities.size())+" "+(shape==Shapes.empty())+" "+(shape==Shapes.block())+" "+shape.toAabbs().size());for(var b:shape.toAabbs())out.printf("box %016x %016x %016x %016x %016x %016x%n",Double.doubleToLongBits(b.minX),Double.doubleToLongBits(b.minY),Double.doubleToLongBits(b.minZ),Double.doubleToLongBits(b.maxX),Double.doubleToLongBits(b.maxY),Double.doubleToLongBits(b.maxZ));}
            for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var p=f.getAsJsonArray("position");var position=new BlockPos(p.get(0).getAsInt(),p.get(1).getAsInt(),p.get(2).getAsInt());var state=Block.stateById(f.get("state").getAsInt());var hit=world.clipWithInteractionOverride(vector(f.get("from")),vector(f.get("to")),position,shape(f.get("shape")),state);if(hit==null)out.println("none");else{var v=hit.getLocation();out.printf("hit %016x %016x %016x %s %d %d %d %s%n",Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z),hit.getDirection().getSerializedName(),hit.getBlockPos().getX(),hit.getBlockPos().getY(),hit.getBlockPos().getZ(),hit.isInside());}}
        }
    }
}
