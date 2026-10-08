package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.level.border.WorldBorder;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;
/** Native border extents, interpolation ticks, collision shape and proximity. */
public final class VanillaWorldBorderOracle {
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static void values(PrintWriter out,String label,double... values){out.print(label);for(double v:values)out.printf(" %016x",Double.doubleToLongBits(v));out.println();}
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var allocator=Class.forName("sun.misc.Unsafe");var access=allocator.getDeclaredField("theUnsafe");access.setAccessible(true);var unsafe=access.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        var entity=(Marker)allocate.invoke(unsafe,Marker.class);var position=Entity.class.getDeclaredField("position");position.setAccessible(true);
        var emit=VanillaFaceSupportOracle.class.getDeclaredMethod("emit",PrintWriter.class,VoxelShape.class);emit.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();var border=new WorldBorder();
                for(var value:f.getAsJsonArray("ops")){
                    var op=value.getAsJsonArray();switch(op.get(0).getAsString()){
                        case "center"->border.setCenter(number(op.get(1)),number(op.get(2)));
                        case "size"->border.setSize(number(op.get(1)));
                        case "limit"->border.setAbsoluteMaxSize(op.get(1).getAsInt());
                        case "lerp"->border.lerpSizeBetween(number(op.get(1)),number(op.get(2)),op.get(3).getAsLong(),op.get(4).getAsLong());
                        case "tick"->{for(int i=0;i<op.get(1).getAsInt();i++)border.tick();}
                        default->throw new AssertionError(op);
                    }
                    out.println("status "+border.getStatus().name()+" "+border.getLerpTime()+" "+border.getAbsoluteMaxSize());values(out,"state",border.getCenterX(),border.getCenterZ(),border.getSize(),border.getLerpTarget(),border.getLerpSpeed());
                    for(var partial:f.getAsJsonArray("partials")){float t=Float.intBitsToFloat(Integer.parseUnsignedInt(partial.getAsString(),16));values(out,"bounds",border.getMinX(t),border.getMinZ(t),border.getMaxX(t),border.getMaxZ(t));}
                    emit.invoke(null,out,border.getCollisionShape());
                    for(var item:f.getAsJsonArray("queries")){
                        var q=item.getAsJsonArray();double x=number(q.get(0)),z=number(q.get(1)),margin=number(q.get(2));position.set(entity,new Vec3(x,0,z));
                        var b=new AABB(number(q.get(3)),number(q.get(4)),number(q.get(5)),number(q.get(6)),number(q.get(7)),number(q.get(8)));
                        values(out,"distance",border.getDistanceToBorder(x,z));out.println("inside "+border.isWithinBounds(x,z,margin)+" "+border.isInsideCloseToBorder(entity,b));
                    }
                }
            }
        }
    }
}
