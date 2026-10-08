package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;
import net.minecraft.world.entity.*;import net.minecraft.world.entity.vehicle.boat.*;import net.minecraft.world.phys.AABB;
/** Actual default and boat passenger fluid-interaction bounds. */
public final class VanillaFluidBoxOracle {
    static final class Stop extends RuntimeException {}
    static Field status;
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static AABB box(JsonElement v){if(v.isJsonNull())return null;var a=v.getAsJsonArray();return new AABB(number(a.get(0)),number(a.get(1)),number(a.get(2)),number(a.get(3)),number(a.get(4)),number(a.get(5)));}
    static String text(AABB b){return b==null?"none":String.format("%016x %016x %016x %016x %016x %016x",Double.doubleToLongBits(b.minX),Double.doubleToLongBits(b.minY),Double.doubleToLongBits(b.minZ),Double.doubleToLongBits(b.maxX),Double.doubleToLongBits(b.maxY),Double.doubleToLongBits(b.maxZ));}
    static void setStatus(Entity boat,int value){try{status.set(boat,value<0?null:AbstractBoat.Status.values()[value]);}catch(Exception e){throw new AssertionError(e);}}
    static final class Data {JsonObject f;PrintWriter out;int calls;Actor actor;ProbeBoat boat;Entity vehicle;
        void query(String s){out.println(s);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt()){actor.setBoundingBox(box(f.get("after_actor")));boat.setBoundingBox(box(f.get("after_boat")));setStatus(boat,f.get("after_status").getAsInt());}}
    }
    static final class Actor extends Marker {Data d;Actor(){super(null,null);}@Override public Entity getVehicle(){d.query("vehicle");return d.vehicle;}}
    static final class Modifier extends Marker {Data d;Modifier(){super(null,null);}@Override protected AABB modifyPassengerFluidInteractionBox(AABB b){d.query("modifier "+text(b));return box(d.f.get("modified"));}}
    static final class ProbeBoat extends Boat {Data d;ProbeBoat(){super(null,null,null);}@Override public boolean isUnderWater(){d.query("boat-under");return super.isUnderWater();}}
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);var unsafe=uf.get(null);var allocate=u.getMethod("allocateInstance",Class.class);status=AbstractBoat.class.getDeclaredField("status");status.setAccessible(true);var helper=AbstractBoat.class.getDeclaredMethod("modifyPassengerFluidInteractionBox",AABB.class);helper.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.actor=(Actor)allocate.invoke(unsafe,Actor.class);d.actor.d=d;d.actor.setBoundingBox(box(f.get("actor")));d.boat=(ProbeBoat)allocate.invoke(unsafe,ProbeBoat.class);d.boat.d=d;d.boat.setBoundingBox(box(f.get("boat")));setStatus(d.boat,f.get("status").getAsInt());int vehicle=f.get("vehicle").getAsInt();if(vehicle==1)d.vehicle=(Marker)allocate.invoke(unsafe,Marker.class);else if(vehicle==2)d.vehicle=d.boat;else if(vehicle==3){var m=(Modifier)allocate.invoke(unsafe,Modifier.class);m.d=d;d.vehicle=m;}
            try{var result=f.get("direct").getAsBoolean()?(AABB)helper.invoke(d.boat,box(f.get("input"))):d.actor.getFluidInteractionBox();out.println("result "+text(result));}catch(Stop e){out.println("error");}catch(InvocationTargetException e){if(e.getCause() instanceof Stop)out.println("error");else throw e;}
            out.println("actor "+text(d.actor.getBoundingBox()));out.println("boat "+text(d.boat.getBoundingBox()));out.println("queries "+d.calls);
        }}
    }
}
