package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.resources.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.densityfunction.*;
import net.minecraft.world.level.levelgen.material.MaterialSystem;
import net.minecraft.world.level.levelgen.synth.*;

/** Original transport for the material system's bands and surface queries. */
public final class VanillaMaterialSurfaceOracle {
    private static final String[] COLORS={"terracotta","orange_terracotta","yellow_terracotta","brown_terracotta","red_terracotta","white_terracotta","light_gray_terracotta"};
    private static MappedRegistry<NormalNoise> noises;
    private static int label(BlockState state) {
        String name=BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString();
        for(int i=0;i<COLORS.length;i++)if(name.equals("minecraft:"+COLORS[i]))return i;
        throw new AssertionError("unknown band state "+name);
    }
    private static String bits(double value) {return Long.toUnsignedString(Double.doubleToLongBits(value),16);}
    private static void fixture(JsonObject f,PrintWriter out) throws Exception {
        long seed=f.get("seed").getAsLong();boolean legacy=f.get("legacy").getAsBoolean();
        RandomSource direct=legacy?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
        if(f.get("child").getAsBoolean())direct=direct.forkPositional().fromHashOf(Identifier.parse("minecraft:clay_bands"));
        direct.consumeCount(f.get("consume").getAsInt());
        var generate=MaterialSystem.class.getDeclaredMethod("generateBands",RandomSource.class);generate.setAccessible(true);
        var bands=(BlockState[])generate.invoke(null,direct);out.println("bands "+bands.length);
        for(var band:bands)out.println(label(band));
        out.println("next "+direct.nextLong()+" "+direct.nextInt());
        var zero=DensityFunctions.zero();var router=new NoiseRouter(zero,zero,zero,zero,zero,zero,zero,zero);
        var random=RandomState.create(noises,seed,legacy,Blocks.STONE.defaultBlockState(),63,router);
        var system=random.surfaceSystem();
        var bandsField=MaterialSystem.class.getDeclaredField("clayBands");bandsField.setAccessible(true);
        out.println("system bands");for(var band:(BlockState[])bandsField.get(system))out.println(label(band));
        String[] names={"clayBandsOffsetNoise","surfaceNoise","surfaceSecondaryNoise"};
        for(int field=0;field<names.length;field++) {
            if(f.getAsJsonArray("overrides").get(field).isJsonNull())continue;
            float value=Float.intBitsToFloat(Integer.parseUnsignedInt(f.getAsJsonArray("overrides").get(field).getAsString(),16));
            Noise noise=new Noise() {
                public net.minecraft.util.Interval range(){return net.minecraft.util.Interval.INFINITE;}
                public float get(double x,double z){throw new AssertionError("unexpected 2d noise lookup");}
                public float get(double x,double y,double z){if(y!=0)throw new AssertionError("unexpected noise Y");return value;}
            };
            var target=MaterialSystem.class.getDeclaredField(names[field]);target.setAccessible(true);target.set(system,noise);
        }
        var bandMethod=MaterialSystem.class.getDeclaredMethod("getBand",int.class,int.class,int.class);bandMethod.setAccessible(true);
        var depthMethod=MaterialSystem.class.getDeclaredMethod("getSurfaceDepth",int.class,int.class);depthMethod.setAccessible(true);
        var secondaryMethod=MaterialSystem.class.getDeclaredMethod("getSurfaceSecondary",int.class,int.class);secondaryMethod.setAccessible(true);
        for(var e:f.getAsJsonArray("points")) {
            var p=e.getAsJsonArray();int x=p.get(0).getAsInt(),y=p.get(1).getAsInt(),z=p.get(2).getAsInt();
            try {out.println("band "+label((BlockState)bandMethod.invoke(system,x,y,z)));}
            catch(java.lang.reflect.InvocationTargetException ex) {if(!(ex.getCause() instanceof ArrayIndexOutOfBoundsException))throw ex;out.println("band error");}
            out.println("depth "+depthMethod.invoke(system,x,z));
            out.println("secondary "+bits((Double)secondaryMethod.invoke(system,x,z)));
        }
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        Path dir=Path.of(args[1]);noises=new MappedRegistry<>(Registries.NOISE,Lifecycle.stable());
        var definitions=JsonParser.parseString(Files.readString(dir.resolve("noises.json"))).getAsJsonObject();
        for(var e:definitions.entrySet())noises.register(ResourceKey.create(Registries.NOISE,Identifier.parse(e.getKey())),NormalNoise.DIRECT_CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(e.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);
        noises.freeze();
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(String line:Files.readAllLines(dir.resolve("inputs.txt")))fixture(JsonParser.parseString(line).getAsJsonObject(),out);
        }
    }
}
