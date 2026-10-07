package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.material.*;
import net.minecraft.world.phys.shapes.*;
/** Native registered fluid shapes over changing world snapshots and shared caches. */
public final class VanillaFluidShapeOracle {
    static final class Stop extends RuntimeException {}
    static FluidState state(JsonElement v){return FluidState.CODEC.parse(JsonOps.INSTANCE,v).getOrThrow();}
    static void emit(PrintWriter out,VoxelShape shape,IdentityHashMap<VoxelShape,Integer> ids){out.println("identity "+ids.computeIfAbsent(shape,k->ids.size()));out.println("shape "+(shape==Shapes.empty())+" "+(shape==Shapes.block())+" "+shape.toAabbs().size());for(var b:shape.toAabbs())out.printf("box %016x %016x %016x %016x %016x %016x%n",Double.doubleToLongBits(b.minX),Double.doubleToLongBits(b.minY),Double.doubleToLongBits(b.minZ),Double.doubleToLongBits(b.maxX),Double.doubleToLongBits(b.maxY),Double.doubleToLongBits(b.maxZ));}
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);var caches=FlowingFluid.class.getDeclaredField("shapes");caches.setAccessible(true);
        if(args[0].equals("export")){var states=new JsonArray();for(var s:Fluid.FLUID_STATE_REGISTRY){if(s.getAmount()>8)throw new AssertionError("new fluid amount");states.add(FluidState.CODEC.encodeStart(JsonOps.INSTANCE,s).getOrThrow());}Files.writeString(dir.resolve("fluid-states.json"),states.toString());return;}
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){
            var f=JsonParser.parseString(line).getAsJsonObject();for(var fluid:List.of(Fluids.WATER,Fluids.FLOWING_WATER,Fluids.LAVA,Fluids.FLOWING_LAVA))((Map<?,?>)caches.get(fluid)).clear();
            var identities=new IdentityHashMap<VoxelShape,Integer>();for(var value:f.getAsJsonArray("ops")){var op=value.getAsJsonObject();int[] calls={0};var s=state(op.get("state"));var above=state(op.get("above"));var p=op.getAsJsonArray("position");var position=new BlockPos(p.get(0).getAsInt(),p.get(1).getAsInt(),p.get(2).getAsInt());var world=(BlockGetter)Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},(proxy,method,values)->{if(method.getName().equals("getFluidState")){var q=(BlockPos)values[0];out.println("fluid "+q.getX()+" "+q.getY()+" "+q.getZ());if(calls[0]++==op.get("fail").getAsInt())throw new Stop();return above;}throw new AssertionError(method);});
                try{emit(out,s.getShape(world,position),identities);}catch(Stop e){out.println("error");}int count=0;for(var fluid:List.of(Fluids.WATER,Fluids.FLOWING_WATER,Fluids.LAVA,Fluids.FLOWING_LAVA))count+=((Map<?,?>)caches.get(fluid)).size();out.println("cached "+count);out.println("queries "+calls[0]);
            }
        }}
    }
}
