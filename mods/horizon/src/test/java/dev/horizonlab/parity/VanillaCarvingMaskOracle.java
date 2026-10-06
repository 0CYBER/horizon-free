package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import net.minecraft.world.level.chunk.CarvingMask;

/** Original transport invoking the pinned vanilla carving mask directly. */
public final class VanillaCarvingMaskOracle {
    private static boolean carve(CarvingMask mask,JsonArray q) {
        try {mask.carve(q.get(0).getAsInt(),q.get(1).getAsInt(),q.get(2).getAsInt());return true;}
        catch(IndexOutOfBoundsException error){return false;}
    }
    private static final class VisitorFailure extends RuntimeException {}
    public static void main(String[] args) throws Exception {
        Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(String line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();var mask=new CarvingMask(f.get("min").getAsInt(),f.get("max").getAsInt());
                out.println("mask "+mask.minY()+" "+mask.maxY());
                for(var value:f.getAsJsonArray("ops")) {
                    var op=value.getAsJsonObject();
                    switch(op.get("kind").getAsString()) {
                        case "carve" -> out.println("carve "+(carve(mask,op.getAsJsonArray("pos"))?"ok":"error"));
                        case "fill" -> {
                            var b=op.getAsJsonArray("box");int calls=0;
                            for(int x=b.get(0).getAsInt();x<=b.get(1).getAsInt();x++)for(int z=b.get(4).getAsInt();z<=b.get(5).getAsInt();z++) {
                                int low=b.get(2).getAsInt(),high=b.get(3).getAsInt();
                                for(int dy=0;dy<=(long)high-low;dy++){mask.carve(x,low+dy,z);calls++;}
                            }
                            out.println("fill "+calls);
                        }
                        case "visit" -> {
                            int[] calls={0};boolean ok=true;
                            try {mask.visit((x,z,low,high)->{
                                out.println("column "+x+" "+z+" "+low+" "+high);int previous=calls[0]++;
                                var points=op.getAsJsonObject("add").get(Integer.toString(previous));
                                if(points!=null)for(var q:points.getAsJsonArray())out.println("add "+(carve(mask,q.getAsJsonArray())?"ok":"error"));
                                if(previous==op.get("fail").getAsInt())throw new VisitorFailure();
                            });}catch(VisitorFailure error){ok=false;}
                            out.println("visit "+(ok?"ok":"error")+" "+calls[0]);
                        }
                        case "read" -> {mask.visit((x,z,low,high)->out.println("column "+x+" "+z+" "+low+" "+high));out.println("read");}
                        default -> throw new IllegalArgumentException("unknown operation");
                    }
                    out.println("empty "+mask.isEmpty());
                }
            }
        }
    }
}
