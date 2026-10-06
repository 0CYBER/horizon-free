package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.BlockPos;
import net.minecraft.world.entity.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;
/** Actual Block.pushEntitiesUp with controlled shapes and selected entity lists. */
public final class VanillaBlockPushOracle {
    static final class Stop extends RuntimeException {}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static AABB box(JsonElement v){var a=v.getAsJsonArray();return new AABB(number(a.get(0)),number(a.get(1)),number(a.get(2)),number(a.get(3)),number(a.get(4)),number(a.get(5)));}
    static String bounds(String label,AABB b){return String.format("%s %016x %016x %016x %016x %016x %016x",label,Double.doubleToLongBits(b.minX),Double.doubleToLongBits(b.minY),Double.doubleToLongBits(b.minZ),Double.doubleToLongBits(b.maxX),Double.doubleToLongBits(b.maxY),Double.doubleToLongBits(b.maxZ));}
    static VoxelShape shape(JsonElement v){VoxelShape s=Shapes.empty();for(var b:v.getAsJsonArray())s=Shapes.or(s,Shapes.create(box(b)));return s;}
    static final class Data{JsonObject f;PrintWriter out;int calls;List<Actor> actors;void query(String text){out.println(text);if(calls++==f.get("fail").getAsInt())throw new Stop();}}
    static final class State extends BlockState {
        Data d;int id;VoxelShape shape;State(){super(null,null,null);}
        @Override public VoxelShape getCollisionShape(BlockGetter world,BlockPos p){d.query("shape "+id+" "+p.getX()+" "+p.getY()+" "+p.getZ());return shape;}
    }
    static final class Actor extends Marker {
        Data d;int id;Actor(){super(null,null);}
        @Override public void teleportRelative(double x,double y,double z){d.query(String.format("teleport %d %016x %016x %016x",id,Double.doubleToLongBits(x),Double.doubleToLongBits(y),Double.doubleToLongBits(z)));setBoundingBox(getBoundingBox().move(x,y,z));if(d.f.get("mutate").getAsBoolean()&&id+1<d.actors.size()){var next=d.actors.get(id+1);next.setBoundingBox(next.getBoundingBox().move(0,.125,0));}}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);var unsafe=field.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.actors=new ArrayList<>();for(var b:f.getAsJsonArray("entities")){var actor=(Actor)allocate.invoke(unsafe,Actor.class);actor.d=d;actor.id=d.actors.size();actor.setBoundingBox(box(b));d.actors.add(actor);}
                var old=(State)allocate.invoke(unsafe,State.class);old.d=d;old.id=0;old.shape=shape(f.get("old"));var next=(State)allocate.invoke(unsafe,State.class);next.d=d;next.id=1;next.shape=shape(f.get("new"));
                var world=(LevelAccessor)Proxy.newProxyInstance(LevelAccessor.class.getClassLoader(),new Class<?>[]{LevelAccessor.class},(proxy,method,values)->{if(method.getName().equals("getEntities")&&values.length==2){if(values[0]!=null)throw new AssertionError();d.query(bounds("entities",(AABB)values[1]));return d.actors;}throw new AssertionError(method);});var p=f.getAsJsonArray("position");var pos=new BlockPos(p.get(0).getAsInt(),p.get(1).getAsInt(),p.get(2).getAsInt());
                try{out.println("result "+(Block.pushEntitiesUp(old,next,world,pos)==next));}catch(Stop error){out.println("error");}for(var actor:d.actors)out.println(bounds("final "+actor.id,actor.getBoundingBox()));out.println("queries "+d.calls);
            }
        }
    }
}
