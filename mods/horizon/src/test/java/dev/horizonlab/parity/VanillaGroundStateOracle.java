package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.BlockPos;
import net.minecraft.world.entity.*;
import net.minecraft.world.phys.*;
/** Native ground/support state transitions with controlled support queries. */
public final class VanillaGroundStateOracle {
    static final class Stop extends RuntimeException {}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static Vec3 vector(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static Optional<BlockPos> position(JsonElement v){if(v.isJsonNull())return Optional.empty();var a=v.getAsJsonArray();return Optional.of(new BlockPos(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt()));}
    static final class Level extends net.minecraft.server.level.ServerLevel {
        JsonObject op;PrintWriter out;int calls;
        private Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public Optional<BlockPos> findSupportingBlock(Entity entity,AABB b){out.printf("find %016x %016x %016x %016x %016x %016x%n",Double.doubleToLongBits(b.minX),Double.doubleToLongBits(b.minY),Double.doubleToLongBits(b.minZ),Double.doubleToLongBits(b.maxX),Double.doubleToLongBits(b.maxY),Double.doubleToLongBits(b.maxZ));int call=calls++;if(call==op.get("fail").getAsInt())throw new Stop();return position(op.getAsJsonArray("results").get(call));}
    }
    static final class Actor extends Marker {Actor(){super(null,null);}void configure(Level level){setLevel(level);}}
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);var unsafe=field.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        var ground=Entity.class.getDeclaredField("onGround");ground.setAccessible(true);var empty=Entity.class.getDeclaredField("onGroundNoBlocks");empty.setAccessible(true);var support=Entity.class.getDeclaredField("mainSupportingBlockPos");support.setAccessible(true);
        var check=Entity.class.getDeclaredMethod("checkSupportingBlock",boolean.class,Vec3.class);check.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();var initial=f.getAsJsonObject("initial");var level=(Level)allocate.invoke(unsafe,Level.class);level.out=out;var entity=(Actor)allocate.invoke(unsafe,Actor.class);entity.configure(level);ground.setBoolean(entity,initial.get("grounded").getAsBoolean());entity.horizontalCollision=initial.get("horizontal").getAsBoolean();empty.setBoolean(entity,initial.get("empty").getAsBoolean());support.set(entity,position(initial.get("support")));
                for(var value:f.getAsJsonArray("ops")){
                    var op=value.getAsJsonObject();level.op=op;level.calls=0;var b=op.getAsJsonArray("box");entity.setBoundingBox(new AABB(number(b.get(0)),number(b.get(1)),number(b.get(2)),number(b.get(3)),number(b.get(4)),number(b.get(5))));Vec3 movement=op.get("movement").isJsonNull()?null:vector(op.get("movement"));boolean onGround=op.get("grounded").getAsBoolean();
                    try{switch(op.get("mode").getAsInt()){case 0->check.invoke(entity,onGround,movement);case 1->entity.setOnGroundWithMovement(onGround,movement);case 2->entity.setOnGroundWithMovement(onGround,op.get("horizontal").getAsBoolean(),movement);default->throw new AssertionError();}out.println("result ok");}
                    catch(InvocationTargetException error){if(error.getCause() instanceof Stop)out.println("result error");else throw error;}catch(Stop error){out.println("result error");}
                    out.println("flags "+entity.onGround()+" "+entity.horizontalCollision+" "+empty.getBoolean(entity));var selected=(Optional<?>)support.get(entity);if(selected.isEmpty())out.println("support none");else{var p=(BlockPos)selected.get();out.println("support "+p.getX()+" "+p.getY()+" "+p.getZ());}out.println("queries "+level.calls);
                }
            }
        }
    }
}
