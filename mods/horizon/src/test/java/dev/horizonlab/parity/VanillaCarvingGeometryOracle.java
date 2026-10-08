package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.levelgen.carver.WorldCarver;

/** Original transport invoking native shared cave/canyon geometry. */
public final class VanillaCarvingGeometryOracle {
    private static double d(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    private static float f(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    private static String bits(double v){return Long.toHexString(Double.doubleToLongBits(v));}
    private static final class OutputFailure extends RuntimeException {}
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(String line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var input=JsonParser.parseString(line).getAsJsonObject();var cp=input.getAsJsonArray("chunk");var chunk=new ChunkPos(cp.get(0).getAsInt(),cp.get(1).getAsInt());
                var mask=new CarvingMask(input.get("min").getAsInt(),input.get("max").getAsInt());
                for(var value:input.getAsJsonArray("ellipsoids")) {
                    var e=value.getAsJsonObject();var center=e.getAsJsonArray("center");int[] calls={0};
                    var output=new CarverOutput(){public int minY(){return mask.minY();}public int maxY(){return mask.maxY();}public void carve(int x,int y,int z){
                        out.println("cell "+x+" "+y+" "+z);int previous=calls[0]++;if(previous==e.get("fail").getAsInt())throw new OutputFailure();mask.carve(x,y,z);
                    }};
                    int mode=e.get("skip").getAsInt();boolean ok=true;
                    try {WorldCarver.carveEllipsoid(chunk,d(center.get(0)),d(center.get(1)),d(center.get(2)),d(e.get("horizontal")),d(e.get("vertical")),output,(x,y,z,by)->{
                        boolean skip=switch(mode){case 0->x*x+y*y+z*z>=1;case 1->y<=-0.7||x*x+y*y+z*z>=1;case 2->Math.floorMod(by,3)==0;default->false;};
                        out.println("skip "+bits(x)+" "+bits(y)+" "+bits(z)+" "+by+" "+skip);return skip;
                    });}catch(OutputFailure error){ok=false;}
                    out.println("ellipsoid "+(ok?"ok":"error")+" "+calls[0]);
                    mask.visit((x,z,low,high)->out.println("column "+x+" "+z+" "+low+" "+high));
                }
            }
            for(String line:Files.readAllLines(dir.resolve("reach.txt"))) {
                var q=JsonParser.parseString(line).getAsJsonArray();
                out.println("reach "+WorldCarver.canReach(new ChunkPos(q.get(0).getAsInt(),q.get(1).getAsInt()),d(q.get(2)),d(q.get(3)),q.get(4).getAsInt(),q.get(5).getAsInt(),f(q.get(6))));
            }
        }
    }
}
