package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.StringTokenizer;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.levelgen.XoroshiroRandomSource;
import net.minecraft.world.level.levelgen.synth.GradientNoise;
import net.minecraft.world.level.levelgen.synth.Noise;
import net.minecraft.world.level.levelgen.synth.NoiseStack;
import net.minecraft.world.level.levelgen.synth.NormalNoise;
import net.minecraft.world.level.levelgen.synth.PerlinNoise;
import net.minecraft.world.level.levelgen.synth.SimplexNoise;
import net.minecraft.world.level.levelgen.synth.SmearedPerlinNoise;
import net.minecraft.world.level.levelgen.densityfunction.DensityBuffer;
import net.minecraft.world.level.levelgen.densityfunction.DensityVolume;

/** Original transport adapter; samples the reference classes directly. */
public final class VanillaNoiseOracle {
    private static int integer(StringTokenizer args) { return Integer.parseInt(args.nextToken()); }
    private static double number(StringTokenizer args) { return Double.longBitsToDouble(Long.parseUnsignedLong(args.nextToken(),16)); }
    private static float small(StringTokenizer args) { return Float.intBitsToFloat(Integer.parseUnsignedInt(args.nextToken(),16)); }
    private static String bits(float value) { return Integer.toUnsignedString(Float.floatToIntBits(value)); }
    private static String bits(double value) { return Long.toUnsignedString(Double.doubleToLongBits(value)); }
    private static Object field(Object object, String name) throws Exception {
        Field field=GradientNoise.class.getDeclaredField(name);field.setAccessible(true);return field.get(object);
    }
    private static Object declared(Object object,String name) throws Exception {
        Field field=object.getClass().getDeclaredField(name);field.setAccessible(true);return field.get(object);
    }
    private static void normal(StringTokenizer args,PrintWriter out) throws Exception {
        String kind=args.nextToken();long seed=Long.parseLong(args.nextToken());int parity=integer(args),base=integer(args),count=integer(args),normalization=integer(args);
        double amplitude=number(args);double[] modifiers=new double[integer(args)];for(int i=0;i<modifiers.length;i++)modifiers[i]=number(args);
        double[][] points=new double[integer(args)][3];for(double[] point:points)for(int i=0;i<3;i++)point[i]=number(args);
        RandomSource random=kind.equals("legacy")?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
        NormalNoise definition;
        try {
            if((parity&1)!=0)definition=NormalNoise.createParity(base,modifiers);
            else {
                var builder=NormalNoise.builder().setBaseOctave(base).setBaseAmplitude(amplitude).setOctaveCount(count);
                if(normalization==2)builder.setLegacyNormalization();else builder.setNormalize(normalization==1);
                for(int i=0;i<modifiers.length;i++)builder.setAmplitudeModifier(i,modifiers[i]);
                definition=builder.build();
            }
        } catch(IllegalArgumentException e) { out.println("normal error "+random.nextLong());return; }
        var octaves=(java.util.List<?>)declared(definition,"octaves");
        out.printf("normal %d %s %s %s%n",octaves.size(),bits((double)declared(definition,"normalizationFactor")),bits(definition.range().min()),bits(definition.range().max()));
        for(Object octave:octaves) {
            Method name=octave.getClass().getDeclaredMethod("seed");name.setAccessible(true);
            out.printf("octave %d %s %s %s%n",(int)declared(octave,"octaveIndex"),bits((double)declared(octave,"frequency")),bits((double)declared(octave,"amplitude")),name.invoke(octave));
        }
        NoiseStack noise;
        try { noise=(NoiseStack)((parity&2)!=0?definition.createForLegacyNetherBiome(random):definition.create(random)); }
        catch(IllegalArgumentException | IllegalStateException e) { out.println("normal-create error");out.println("normal-parent "+random.nextLong());return; }
        Field layerField=NoiseStack.class.getDeclaredField("layers");layerField.setAccessible(true);Object[] layers=(Object[])layerField.get(noise);
        out.printf("normal-create %d %s %s%n",layers.length,bits(noise.range().min()),bits(noise.range().max()));
        for(int i=0;i<layers.length;i++)out.printf("normal-layer %s %s %s %s%n",bits((double)declared(layers[i],"frequency")),bits((float)declared(layers[i],"amplitude")),bits(noise.getLayer(i).get(.25,.75)),bits(noise.getLayer(i).get(-17.25,64.125,32.5)));
        for(double[] point:points)out.printf("normal-query %s %s%n",bits(noise.get(point[0],point[2])),bits(noise.get(point[0],point[1],point[2])));
        var volume=new DensityVolume(2,5,2,-17,-64,32,4,1,4);var buffer=DensityBuffer.createUnpooled(volume.size());
        for(int i=0;i<volume.size();i++)buffer.set(i,(i-10.0f)*.125f);
        float[] amplitudes={.5f,-.25f};
        for(int pass=0;pass<2;pass++) {
            noise.addToVolume(buffer,volume,.125,.25,amplitudes[pass]);
            for(int i=0;i<volume.size();i++)out.printf("normal-volume %d %d %s%n",pass,i,bits(buffer.get(i)));
        }
        out.println("normal-parent "+random.nextLong());
    }
    public static void main(String[] paths) throws Exception {
        var samplers=new ArrayList<Noise>();
        Method wrap=GradientNoise.class.getDeclaredMethod("wrap",double.class);wrap.setAccessible(true);
        try(var reader=Files.newBufferedReader(Path.of(paths[0]),StandardCharsets.UTF_8);
            var out=new PrintWriter(Files.newBufferedWriter(Path.of(paths[1]),StandardCharsets.UTF_8))) {
            String line;
            while((line=reader.readLine())!=null) {
                var args=new StringTokenizer(line);
                String operation=args.nextToken();
                switch(operation) {
                    case "normal" -> normal(args,out);
                    case "create" -> {
                        String kind=args.nextToken(),stream=args.nextToken();long seed=Long.parseLong(args.nextToken());
                        boolean skip=integer(args)!=0;double scale=number(args);
                        RandomSource random=switch(stream) {case "legacy"->new LegacyRandomSource(seed);case "xoroshiro"->new XoroshiroRandomSource(seed);default->throw new AssertionError();};
                        GradientNoise noise=switch(kind) {case "perlin"->new PerlinNoise(random);case "simplex"->new SimplexNoise(random,skip);case "smeared"->new SmearedPerlinNoise(random,scale);default->throw new AssertionError();};
                        samplers.add(noise);
                        String lower,upper;
                        try { lower=bits(noise.range().min());upper=bits(noise.range().max()); }
                        catch(IllegalArgumentException e) { lower="error";upper="error"; }
                        out.printf("create %s %s %s %s %s %d%n",bits((double)field(noise,"offsetX")),bits((double)field(noise,"offsetY")),bits((double)field(noise,"offsetZ")),lower,upper,random.nextLong());
                        out.print("permutations");for(byte v:(byte[])field(noise,"perms"))out.print(" "+(v&255));out.println();
                    }
                    case "query", "local" -> {
                        var noise=samplers.get(integer(args));double x=number(args),y=number(args),z=number(args);
                        if(operation.equals("local")) { x-=(double)field(noise,"offsetX");y-=(double)field(noise,"offsetY");z-=(double)field(noise,"offsetZ"); }
                        float[] derivative={small(args),small(args),small(args)};
                        out.print("query "+bits(noise.get(x,z))+" "+bits(noise.get(x,y,z)));
                        if(noise instanceof PerlinNoise perlin)out.print(" "+bits(perlin.noiseWithDerivative(x,y,z,derivative))+" "+bits(derivative[0])+" "+bits(derivative[1])+" "+bits(derivative[2]));
                        out.println();
                    }
                    case "volume" -> {
                        var noise=samplers.get(integer(args));
                        int sx=integer(args),sy=integer(args),sz=integer(args),mx=integer(args),my=integer(args),mz=integer(args),dx=integer(args),dy=integer(args),dz=integer(args);
                        double xzScale=number(args),yScale=number(args);float amplitude=small(args);long seed=Long.parseLong(args.nextToken());int repeats=integer(args);
                        DensityVolume volume;
                        try { volume=new DensityVolume(sx,sy,sz,mx,my,mz,dx,dy,dz); }
                        catch(IllegalArgumentException e) { out.println("volume error");break; }
                        var buffer=DensityBuffer.createUnpooled(volume.size()+4);var random=new LegacyRandomSource(seed);
                        for(int i=0;i<buffer.size();i++)buffer.set(i,Float.intBitsToFloat(random.nextInt()));
                        for(int i=0;i<repeats;i++) noise.addToVolume(buffer,volume,xzScale,yScale,amplitude);
                        out.printf("volume %d %d %d %d%n",volume.size(),volume.maxBlockX(),volume.maxBlockY(),volume.maxBlockZ());
                        for(int z=0;z<sz;z++)for(int x=0;x<sx;x++)for(int y=0;y<sy;y++) {
                            int index=volume.indexUnchecked(x,y,z);
                            out.printf("cell %d %d %d %d %s%n",index,volume.blockX(x),volume.blockY(y),volume.blockZ(z),bits(buffer.get(index)));
                        }
                        for(int i=volume.size();i<buffer.size();i++)out.println("tail "+bits(buffer.get(i)));
                    }
                    case "stack" -> {
                        int count=integer(args);var builder=NoiseStack.builder();
                        for(int i=0;i<count;i++) {
                            int flatten=integer(args),id=integer(args);double frequency=number(args);float amplitude=small(args);
                            if(flatten!=0)builder.addStack((NoiseStack)samplers.get(id),frequency,amplitude);
                            else builder.add(samplers.get(id),frequency,amplitude);
                        }
                        NoiseStack stack;
                        try { stack=builder.build(); } catch(IllegalArgumentException e) { out.println("stack error");break; }
                        Field layersField=NoiseStack.class.getDeclaredField("layers");layersField.setAccessible(true);
                        Object[] layers=(Object[])layersField.get(stack);
                        out.printf("stack %d %s %s%n",layers.length,bits(stack.range().min()),bits(stack.range().max()));
                        for(int i=0;i<layers.length;i++) {
                            Field frequency=layers[i].getClass().getDeclaredField("frequency"),amplitude=layers[i].getClass().getDeclaredField("amplitude");
                            frequency.setAccessible(true);amplitude.setAccessible(true);
                            out.printf("layer %s %s %s%n",bits(frequency.getDouble(layers[i])),bits(amplitude.getFloat(layers[i])),bits(stack.getLayer(i).get(0,0,0)));
                        }
                        samplers.add(stack);
                    }
                    case "wrap" -> out.println("wrap "+bits((double)wrap.invoke(null,number(args))));
                    default -> throw new AssertionError();
                }
                if(args.hasMoreTokens())throw new AssertionError("extra argument");
            }
            if(out.checkError())throw new java.io.IOException("reference output failed");
        }
    }
}
