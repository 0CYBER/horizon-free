package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.nio.file.*;
import net.minecraft.world.entity.Entity;import net.minecraft.world.phys.Vec3;
/** Native view vectors and native geometry/acos composition used by blocking. */
public final class VanillaViewDirectionOracle {
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static Vec3 vec(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static String bits(double v){return String.format("%016x",Double.doubleToLongBits(v));}
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();switch(f.get("mode").getAsString()){
            case "acos":out.println("angle "+bits(Math.acos(number(f.get("value")))));break;
            case "view":{var v=Entity.calculateViewVector(single(f.get("pitch")),single(f.get("yaw")));out.println("view "+bits(v.x)+" "+bits(v.y)+" "+bits(v.z));break;}
            case "blocking":{var view=Entity.calculateViewVector(0,single(f.get("yaw")));var delta=vec(f.get("source")).subtract(vec(f.get("position")));var horizontal=new Vec3(delta.x,0,delta.z).normalize();out.println("angle "+bits(Math.acos(horizontal.dot(view))));break;}
            case "fallback":out.println("angle "+bits((double)(float)Math.PI));break;
            default:throw new AssertionError();}
        }}
    }
}
