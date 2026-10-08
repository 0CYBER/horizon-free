package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.player.*;
import net.minecraft.world.level.GameType;
import net.minecraft.world.phys.*;
/** Actual Player edge-backoff decisions and probe bounds with controlled collisions. */
public final class VanillaEdgeBackoffOracle {
    static final class Stop extends RuntimeException {}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static Vec3 vector(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static final class Data {
        JsonObject input;PrintWriter out;int calls,probes;
        void query(String text){out.println(text);if(calls++==input.get("fail").getAsInt())throw new Stop();}
    }
    static final class Level extends net.minecraft.server.level.ServerLevel {
        Data d;
        private Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public boolean noCollision(Entity entity,AABB b){d.query(String.format("clear %016x %016x %016x %016x %016x %016x",Double.doubleToLongBits(b.minX),Double.doubleToLongBits(b.minY),Double.doubleToLongBits(b.minZ),Double.doubleToLongBits(b.maxX),Double.doubleToLongBits(b.maxY),Double.doubleToLongBits(b.maxZ)));var values=d.input.getAsJsonArray("clear");int index=d.probes++;return index<values.size()?values.get(index).getAsBoolean():false;}
    }
    static final class Actor extends Player {
        Data d;Actor(){super(null,null);}void configure(Level level){setLevel(level);}
        @Override public GameType gameMode(){return GameType.SURVIVAL;}
        @Override public float maxUpStep(){d.query("step");return Float.intBitsToFloat(Integer.parseUnsignedInt(d.input.get("step").getAsString(),16));}
        @Override protected boolean isStayingOnGroundSurface(){d.query("staying");return d.input.get("staying").getAsBoolean();}
        @Override public boolean onGround(){d.query("ground");return d.input.get("ground").getAsBoolean();}
        Vec3 backoff(Vec3 v,MoverType type){return maybeBackOffFromEdge(v,type);}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);var unsafe=field.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);var abilities=Player.class.getDeclaredField("abilities");abilities.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.input=f;d.out=out;var level=(Level)allocate.invoke(unsafe,Level.class);level.d=d;var entity=(Actor)allocate.invoke(unsafe,Actor.class);entity.d=d;entity.configure(level);var a=new Abilities();a.flying=f.get("flying").getAsBoolean();abilities.set(entity,a);entity.fallDistance=number(f.get("fall"));var b=f.getAsJsonArray("box");entity.setBoundingBox(new AABB(number(b.get(0)),number(b.get(1)),number(b.get(2)),number(b.get(3)),number(b.get(4)),number(b.get(5))));
                try{var result=entity.backoff(vector(f.get("movement")),MoverType.valueOf(f.get("mover").getAsString()));out.printf("movement %016x %016x %016x%n",Double.doubleToLongBits(result.x),Double.doubleToLongBits(result.y),Double.doubleToLongBits(result.z));}catch(Stop error){out.println("error");}out.println("queries "+d.calls);
            }
        }
    }
}
