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
/** Original snapshot transport executing the actual EntityGetter default method. */
public final class VanillaEntityObstructionOracle {
    private static final class Stop extends RuntimeException {}
    public static final class Snapshot extends Marker {
        boolean spectator;
        // Instances are allocated without constructors: the world-owned entity lifecycle is outside this fixture.
        public Snapshot(){super(null,null);}
        @Override public boolean isSpectator(){return spectator;}
    }
    private static AABB box(JsonElement value) {
        var a=value.getAsJsonArray();double[] v=new double[6];
        for(int i=0;i<6;i++)v[i]=Double.longBitsToDouble(Long.parseUnsignedLong(a.get(i).getAsString(),16));
        return new AABB(v[0],v[1],v[2],v[3],v[4],v[5]);
    }
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        Class<?> allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);var unsafe=field.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        var removed=Entity.class.getDeclaredField("removalReason");removed.setAccessible(true);var vehicle=Entity.class.getDeclaredField("vehicle");vehicle.setAccessible(true);
        Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var inputs=Files.lines(dir.resolve("inputs.txt"))) {
            for(var line:(Iterable<String>)inputs::iterator) {
                var f=JsonParser.parseString(line).getAsJsonObject();var entities=new ArrayList<Snapshot>();
                for(var value:f.getAsJsonArray("entities")) {
                    var e=value.getAsJsonObject();var snapshot=(Snapshot)allocate.invoke(unsafe,Snapshot.class);
                    snapshot.setId(entities.size()+1);snapshot.setBoundingBox(box(e.get("box")));snapshot.blocksBuilding=e.get("building").getAsBoolean();snapshot.spectator=e.get("spectator").getAsBoolean();
                    if(e.get("removed").getAsBoolean())removed.set(snapshot,Entity.RemovalReason.DISCARDED);
                    int parent=e.get("parent").getAsInt();if(parent>=0)vehicle.set(snapshot,entities.get(parent));entities.add(snapshot);
                }
                VoxelShape shape=Shapes.empty();for(var value:f.getAsJsonArray("shape"))shape=Shapes.or(shape,Shapes.create(box(value)));
                int excluded=f.get("exclude").getAsInt();Entity except=excluded<0?null:entities.get(excluded);int[] calls={0};
                var world=(EntityGetter)Proxy.newProxyInstance(EntityGetter.class.getClassLoader(),new Class<?>[]{EntityGetter.class},(proxy,method,values)->{
                    if(method.isDefault())return InvocationHandler.invokeDefault(proxy,method,values);
                    if(!method.getName().equals("getEntities")||values.length!=3)throw new AssertionError(method);
                    var exceptEntity=(Entity)values[0];var bounds=(AABB)values[1];var predicate=(Predicate<Entity>)values[2];
                    out.printf("query %d %016x %016x %016x %016x %016x %016x%n",exceptEntity==null?-1:exceptEntity.getId(),Double.doubleToRawLongBits(bounds.minX),Double.doubleToRawLongBits(bounds.minY),Double.doubleToRawLongBits(bounds.minZ),Double.doubleToRawLongBits(bounds.maxX),Double.doubleToRawLongBits(bounds.maxY),Double.doubleToRawLongBits(bounds.maxZ));
                    calls[0]++;if(f.get("fail").getAsBoolean())throw new Stop();
                    var selected=new ArrayList<Entity>();
                    for(var entity:entities)if(entity!=exceptEntity&&entity.getBoundingBox().intersects(bounds)&&predicate.test(entity)){selected.add(entity);out.println("picked "+entity.getId());}
                    return selected;
                });
                try {out.println("clear "+world.isUnobstructed(except,shape));}catch(Stop error){out.println("clear error");}
                out.println("queries "+calls[0]);
            }
        }
    }
}
