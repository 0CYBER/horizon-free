package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.world.level.levelgen.Beardifier;
import net.minecraft.world.level.levelgen.densityfunction.*;
import net.minecraft.world.level.levelgen.structure.*;
import net.minecraft.world.level.levelgen.structure.pools.*;

/** Original transport for vanilla's structure-density sampler. */
public final class VanillaStructureDensityOracle {
    private static BoundingBox box(JsonElement e) {
        if(e.isJsonNull())return null;var a=e.getAsJsonArray();return new BoundingBox(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt(),a.get(3).getAsInt(),a.get(4).getAsInt(),a.get(5).getAsInt());
    }
    private static String bits(float value) {return Integer.toUnsignedString(Float.floatToIntBits(value),16);}
    private static void fixture(JsonObject f,PrintWriter out) {
        var pieces=new ArrayList<Beardifier.Rigid>();var junctions=new ArrayList<JigsawJunction>();
        for(var e:f.getAsJsonArray("pieces")) {var a=e.getAsJsonArray();pieces.add(new Beardifier.Rigid(box(a.get(0)),TerrainAdjustment.valueOf(a.get(1).getAsString()),a.get(2).getAsInt()));}
        for(var e:f.getAsJsonArray("junctions")) {var a=e.getAsJsonArray();junctions.add(new JigsawJunction(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt(),0,StructureTemplatePool.Projection.RIGID));}
        var sampler=new Beardifier(pieces,junctions,box(f.get("affected")));var context=SamplerContext.builder().build();
        out.println("beardifier "+pieces.size()+" "+junctions.size());
        for(var e:f.getAsJsonArray("points")) {var a=e.getAsJsonArray();out.println("value "+bits(sampler.sampleValue(context,a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt())));}
        for(var e:f.getAsJsonArray("volumes")) {
            var a=e.getAsJsonArray();var volume=new DensityVolume(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt(),a.get(3).getAsInt(),a.get(4).getAsInt(),a.get(5).getAsInt(),a.get(6).getAsInt(),a.get(7).getAsInt(),a.get(8).getAsInt());
            var buffer=DensityBuffer.createUnpooled(volume.size()+a.get(9).getAsInt());buffer.fill(-123.5f);
            sampler.sampleVolume(context,buffer,volume);out.println("volume "+buffer.size());
            for(int i=0;i<buffer.size();i++)out.println(bits(buffer.get(i)));
        }
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        try(var out=new PrintWriter(Files.newBufferedWriter(Path.of(args[1]),StandardCharsets.UTF_8))) {
            for(String line:Files.readAllLines(Path.of(args[0])))fixture(JsonParser.parseString(line).getAsJsonObject(),out);
        }
    }
}
