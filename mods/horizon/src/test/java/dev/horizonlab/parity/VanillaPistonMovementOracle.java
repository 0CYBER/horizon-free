package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.phys.Vec3;
/** Actual Entity piston limiter with sequential tick/movement inputs. */
public final class VanillaPistonMovementOracle {
    static final class Stop extends RuntimeException {}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static Vec3 vector(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static final class Level extends net.minecraft.server.level.ServerLevel {
        JsonObject op;PrintWriter out;int calls;
        private Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public long getGameTime(){out.println("time");calls++;if(op.get("fail").getAsBoolean())throw new Stop();return op.get("time").getAsLong();}
    }
    static final class Actor extends Marker {Actor(){super(null,null);}void configure(Level level){setLevel(level);}Vec3 limit(Vec3 v){return limitPistonMovement(v);}}
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);var unsafe=field.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        var deltas=Entity.class.getDeclaredField("pistonDeltas");deltas.setAccessible(true);var time=Entity.class.getDeclaredField("pistonDeltasGameTime");time.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();var level=(Level)allocate.invoke(unsafe,Level.class);level.out=out;var entity=(Actor)allocate.invoke(unsafe,Actor.class);entity.configure(level);var d=vector(f.get("deltas"));deltas.set(entity,new double[]{d.x,d.y,d.z});time.setLong(entity,f.get("time").getAsLong());
                for(var value:f.getAsJsonArray("ops")){var op=value.getAsJsonObject();level.op=op;level.calls=0;try{var result=entity.limit(vector(op.get("movement")));out.printf("movement %016x %016x %016x%n",Double.doubleToLongBits(result.x),Double.doubleToLongBits(result.y),Double.doubleToLongBits(result.z));}catch(Stop error){out.println("error");}var accumulated=(double[])deltas.get(entity);out.printf("deltas %016x %016x %016x%n",Double.doubleToLongBits(accumulated[0]),Double.doubleToLongBits(accumulated[1]),Double.doubleToLongBits(accumulated[2]));out.println("tick "+time.getLong(entity));out.println("queries "+level.calls);}
            }
        }
    }
}
