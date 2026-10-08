package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.lang.reflect.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.resources.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.BlockColumn;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.densityfunction.DensityFunctions;
import net.minecraft.world.level.levelgen.material.MaterialSystem;
import net.minecraft.world.level.levelgen.synth.*;

/** Original transport invoking the actual private column extension methods. */
public final class VanillaMaterialExtensionOracle {
    private static final String[] NAMES={"badlands_surface","badlands_pillar","badlands_pillar_roof","iceberg_surface","iceberg_pillar","iceberg_pillar_roof"};
    private static final String[] FIELDS={"badlandsSurfaceNoise","badlandsPillarNoise","badlandsPillarRoofNoise","icebergSurfaceNoise","icebergPillarNoise","icebergPillarRoofNoise"};
    private static MappedRegistry<NormalNoise> noises;
    private static JsonObject biomes;
    private static BlockState[] states;
    private static Method badlands,frozen,climate,make;
    private static int label(BlockState state){for(int i=0;i<states.length;i++)if(states[i]==state)return i;throw new AssertionError("unexpected state "+state);}
    private static void fixture(JsonObject f,PrintWriter out) throws Exception {
        var zero=DensityFunctions.zero();var router=new NoiseRouter(zero,zero,zero,zero,zero,zero,zero,zero);
        var system=RandomState.create(noises,f.get("seed").getAsLong(),f.get("legacy").getAsBoolean(),states[f.get("default").getAsInt()],f.get("sea").getAsInt(),router).surfaceSystem();
        for(int i=0;i<6;i++) {
            var field=MaterialSystem.class.getDeclaredField(FIELDS[i]);field.setAccessible(true);var real=(Noise)field.get(system);final int index=i;
            field.set(system,new Noise(){
                public net.minecraft.util.Interval range(){return net.minecraft.util.Interval.INFINITE;}
                public float get(double x,double z){throw new AssertionError("unexpected 2D query");}
                public float get(double x,double y,double z){out.println("noise "+NAMES[index]+" "+Long.toUnsignedString(Double.doubleToLongBits(x),16)+" "+Long.toUnsignedString(Double.doubleToLongBits(y),16)+" "+Long.toUnsignedString(Double.doubleToLongBits(z),16));return f.has("noise_bits")?Float.intBitsToFloat(Integer.parseUnsignedInt(f.getAsJsonArray("noise_bits").get(index).getAsString(),16)):real.get(x,y,z);}
            });
        }
        var climateInput=new JsonObject();climateInput.add("json",biomes.get(f.get("biome").getAsString()));var biome=(Biome)make.invoke(null,climate.invoke(null,climateInput));
        int min=f.get("min").getAsInt();var values=new BlockState[f.getAsJsonArray("initial").size()];for(int i=0;i<values.length;i++)values[i]=states[f.getAsJsonArray("initial").get(i).getAsInt()];
        int[] remaining={0};
        BlockColumn column=new BlockColumn(){
            public BlockState getBlock(int y){if(remaining[0]==0){out.println("read error "+y);throw new IllegalStateException("fixture read limit");}remaining[0]--;long i=(long)y-min;var state=i>=0 && i<values.length?values[(int)i]:states[2];out.println("read "+y+" "+label(state));return state;}
            public void setBlock(int y,BlockState state){out.println("write "+y+" "+label(state));long i=(long)y-min;if(i>=0 && i<values.length)values[(int)i]=state;}
        };
        for(var value:f.getAsJsonArray("ops")) {
            var op=value.getAsJsonArray();remaining[0]=f.get("limit").getAsInt();int x=op.get(1).getAsInt(),z=op.get(2).getAsInt(),surface=op.get(3).getAsInt(),minimum=op.get(4).getAsInt();
            try{if(op.get(0).getAsString().equals("badlands"))badlands.invoke(system,column,x,z,surface,LevelHeightAccessor.create(minimum,values.length));else frozen.invoke(system,minimum,biome,column,new BlockPos.MutableBlockPos(),x,z,surface);out.println("result ok");}
            catch(InvocationTargetException error){if(!(error.getCause() instanceof RuntimeException))throw error;out.println("result error");}
            out.print("column");for(var state:values)out.print(" "+label(state));out.println();
        }
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
        String[] descriptors={"\"minecraft:air\"","\"minecraft:cave_air\"","\"minecraft:void_air\"","\"minecraft:stone\"","\"minecraft:water\"","{\"id\":\"minecraft:water\",\"properties\":{\"level\":\"5\"}}","\"minecraft:snow_block\"","\"minecraft:packed_ice\"","\"minecraft:oak_log\"","{\"id\":\"minecraft:oak_log\",\"properties\":{\"axis\":\"x\"}}","{\"id\":\"minecraft:oak_stairs\",\"properties\":{\"waterlogged\":\"true\"}}","\"minecraft:dirt\"","\"minecraft:lava\""};
        states=Arrays.stream(descriptors).map(text->BlockState.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(text)).getOrThrow()).toArray(BlockState[]::new);
        noises=new MappedRegistry<>(Registries.NOISE,Lifecycle.stable());var definitions=JsonParser.parseString(Files.readString(dir.resolve("noises.json"))).getAsJsonObject();
        for(var e:definitions.entrySet())noises.register(ResourceKey.create(Registries.NOISE,Identifier.parse(e.getKey())),NormalNoise.DIRECT_CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(e.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);noises.freeze();
        biomes=JsonParser.parseString(Files.readString(dir.resolve("biomes.json"))).getAsJsonObject();
        badlands=MaterialSystem.class.getDeclaredMethod("erodedBadlandsExtension",BlockColumn.class,int.class,int.class,int.class,LevelHeightAccessor.class);badlands.setAccessible(true);
        frozen=MaterialSystem.class.getDeclaredMethod("frozenOceanExtension",int.class,Biome.class,BlockColumn.class,BlockPos.MutableBlockPos.class,int.class,int.class,int.class);frozen.setAccessible(true);
        climate=VanillaBiomeTemperatureOracle.class.getDeclaredMethod("climate",JsonObject.class);climate.setAccessible(true);make=VanillaBiomeTemperatureOracle.class.getDeclaredMethod("make",Object.class);make.setAccessible(true);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))){for(var line:Files.readAllLines(dir.resolve("inputs.txt")))fixture(JsonParser.parseString(line).getAsJsonObject(),out);}
    }
}
