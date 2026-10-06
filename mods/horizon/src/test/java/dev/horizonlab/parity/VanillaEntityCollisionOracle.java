package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.util.function.Predicate;
import net.minecraft.world.entity.*;
import net.minecraft.world.level.EntityGetter;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.*;
/** Actual EntityGetter collision selection with traced entity policy overrides. */
public final class VanillaEntityCollisionOracle {
    static final class Stop extends RuntimeException {}
    static final class Trace {
        PrintWriter out;int calls,fail;
        void query(String text){out.println(text);if(calls++==fail)throw new Stop();}
    }
    public static class Snapshot extends Marker {
        boolean spectator,collidable;int policy;Trace trace;
        public Snapshot(){super(null,null);}
        @Override public boolean isSpectator(){return spectator;}
        @Override public boolean canBeCollidedWith(Entity source){trace.query("collidable "+getId()+" "+(source==null?-1:source.getId()));return collidable;}
        @Override public boolean canCollideWith(Entity candidate){trace.query("collide "+getId()+" "+candidate.getId());return policy==0?super.canCollideWith(candidate):policy==1;}
    }
    static AABB box(JsonElement value){var v=value.getAsJsonArray();double[] d=new double[6];for(int i=0;i<6;i++)d[i]=Double.longBitsToDouble(Long.parseUnsignedLong(v.get(i).getAsString(),16));return new AABB(d[0],d[1],d[2],d[3],d[4],d[5]);}
    @SuppressWarnings("unchecked")
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);var unsafe=field.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        var vehicle=Entity.class.getDeclaredField("vehicle");vehicle.setAccessible(true);
        var emit=VanillaFaceSupportOracle.class.getDeclaredMethod("emit",PrintWriter.class,VoxelShape.class);emit.setAccessible(true);
        Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();var trace=new Trace();trace.out=out;trace.fail=f.get("fail").getAsInt();var entities=new ArrayList<Snapshot>();
                for(var item:f.getAsJsonArray("entities")){
                    var v=item.getAsJsonObject();var entity=(Snapshot)allocate.invoke(unsafe,Snapshot.class);entity.setId(entities.size()+1);entity.setBoundingBox(box(v.get("box")));entity.spectator=v.get("spectator").getAsBoolean();entity.collidable=v.get("collidable").getAsBoolean();entity.policy=v.get("policy").getAsInt();entity.trace=trace;
                    int parent=v.get("parent").getAsInt();if(parent>=0)vehicle.set(entity,entities.get(parent));entities.add(entity);
                }
                var world=(EntityGetter)Proxy.newProxyInstance(EntityGetter.class.getClassLoader(),new Class<?>[]{EntityGetter.class},(proxy,method,values)->{
                    if(method.isDefault())return InvocationHandler.invokeDefault(proxy,method,values);
                    if(!method.getName().equals("getEntities")||values.length!=3)throw new AssertionError(method);
                    var source=(Entity)values[0];var bounds=(AABB)values[1];var predicate=(Predicate<Entity>)values[2];
                    trace.query(String.format("query %d %016x %016x %016x %016x %016x %016x",source==null?-1:source.getId(),Double.doubleToLongBits(bounds.minX),Double.doubleToLongBits(bounds.minY),Double.doubleToLongBits(bounds.minZ),Double.doubleToLongBits(bounds.maxX),Double.doubleToLongBits(bounds.maxY),Double.doubleToLongBits(bounds.maxZ)));
                    var selected=new ArrayList<Entity>();for(var entity:entities)if(entity!=source&&entity.getBoundingBox().intersects(bounds)&&predicate.test(entity))selected.add(entity);return selected;
                });
                int source=f.get("source").getAsInt();
                try{var shapes=world.getEntityCollisions(source<0?null:entities.get(source),box(f.get("box")));out.println("selected "+shapes.size());for(var shape:shapes)emit.invoke(null,out,shape);}
                catch(Stop error){out.println("error");}
                out.println("queries "+trace.calls);
            }
        }
    }
}
