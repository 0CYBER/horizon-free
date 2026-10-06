package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.StringTokenizer;
import net.minecraft.core.Direction.Axis;
import net.minecraft.util.RandomSource;
import net.minecraft.util.Interval;
import net.minecraft.util.CubicSpline;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.levelgen.Beardifier;
import net.minecraft.world.level.levelgen.blending.Blender;
import net.minecraft.util.context.ContextMap;
import net.minecraft.world.level.levelgen.XoroshiroRandomSource;
import net.minecraft.world.level.levelgen.densityfunction.*;
import net.minecraft.world.level.levelgen.densityfunction.generator.*;
import net.minecraft.world.level.levelgen.synth.*;
import net.minecraft.world.level.levelgen.densityfunction.op.*;

/** Original transport: all density arithmetic is evaluated by vanilla classes. */
public final class VanillaDensityOracle {
    private static int integer(StringTokenizer args) { return Integer.parseInt(args.nextToken()); }
    private static double number(StringTokenizer args) { return Double.longBitsToDouble(Long.parseUnsignedLong(args.nextToken(),16)); }
    private static float small(StringTokenizer args) { return Float.intBitsToFloat(Integer.parseUnsignedInt(args.nextToken(),16)); }
    private static String bits(float value) { return Integer.toUnsignedString(Float.floatToIntBits(value)); }
    private static DensitySampler construct(String nested,Class<?>[] types,Object... arguments) throws Exception {
        var constructor=Class.forName("net.minecraft.world.level.levelgen.densityfunction.op."+nested).getDeclaredConstructor(types);
        constructor.setAccessible(true);return (DensitySampler)constructor.newInstance(arguments);
    }
    private static void intervalOutput(PrintWriter out,String name,java.util.function.Supplier<Interval> compute) {
        try { var result=compute.get();out.printf("bounds %s %s %s %d %d%n",name,bits(result.min()),bits(result.max()),result.isNaI()?1:0,result.hashCode()); }
        catch(IllegalArgumentException | IllegalStateException e) { out.println("bounds "+name+" error"); }
    }
    private static void fbmProbe(RandomSource random,int octave,double smear,double amplitude,PrintWriter out) throws Exception {
        NoiseStack stack;
        try { stack=BlendedNoise.createFbm(random,octave,smear,amplitude); }
        catch(IllegalArgumentException e) { out.println("fbm error");out.println("fbm-parent "+random.nextLong());return; }
        var layersField=NoiseStack.class.getDeclaredField("layers");layersField.setAccessible(true);Object[] layers=(Object[])layersField.get(stack);
        out.printf("fbm %d %s %s%n",layers.length,bits(stack.range().min()),bits(stack.range().max()));
        for(int i=0;i<layers.length;i++) {
            var frequency=layers[i].getClass().getDeclaredField("frequency");var gain=layers[i].getClass().getDeclaredField("amplitude");frequency.setAccessible(true);gain.setAccessible(true);
            out.printf("fbm-layer %s %s %s%n",Long.toUnsignedString(Double.doubleToLongBits(frequency.getDouble(layers[i]))),bits(gain.getFloat(layers[i])),bits(stack.getLayer(i).get(-17.25,64.125,32.5)));
        }
        double[][] points={{0,0,0},{-17.25,64.125,32.5},{3e7,-64,-3e7},{Double.POSITIVE_INFINITY,Double.POSITIVE_INFINITY,Double.POSITIVE_INFINITY}};
        for(var point:points)out.printf("fbm-query %s %s%n",bits(stack.get(point[0],point[2])),bits(stack.get(point[0],point[1],point[2])));
        var volume=new DensityVolume(2,5,2,-17,-64,32,4,1,4);var buffer=DensityBuffer.createUnpooled(volume.size()+2);
        for(int i=0;i<buffer.size();i++)buffer.set(i,(i-10.0f)*.125f);
        stack.addToVolume(buffer,volume,.125,.25,-.5f);
        for(int i=0;i<buffer.size();i++)out.println("fbm-cell "+bits(buffer.get(i)));
        out.println("fbm-parent "+random.nextLong());
    }
    private static DensityFunction.CompileContext shiftContext(Noise noise) {
        return new DensityFunction.CompileContext() {
            public Noise createNoiseSampler(net.minecraft.core.Holder<NormalNoise> holder) { return noise; }
            public RandomSource createRandom(net.minecraft.resources.Identifier id) { throw new AssertionError("unexpected random lookup"); }
            public RandomSource createEndIslandRandom() { throw new AssertionError("unexpected end lookup"); }
        };
    }
    private static Interval[] splineRanges = {Interval.INFINITE,Interval.INFINITE,Interval.INFINITE};
    private static final class TransportFunction implements DensityFunction {
        private final DensitySampler sampler;
        private final int id;
        TransportFunction(int id,DensitySampler sampler) { this.id=id;this.sampler=sampler; }
        public DensitySampler compileSampler(DensityFunction.CompileContext context) { return sampler; }
        public DensityFunction rewriteChildren(DfRewriteRule rule) { return this; }
        public Interval range() { return splineRanges[id % 3]; }
        public int domainAxes() { return 7; }
        public com.mojang.serialization.MapCodec<? extends DensityFunction> codec() { throw new AssertionError("unexpected codec lookup"); }
    }
    private static final class BoundFunction implements DensityFunction {
        private final DensitySampler sampler;
        private final Interval range;
        private final int axes;
        BoundFunction(DensitySampler sampler,Interval range,int axes){this.sampler=sampler;this.range=range;this.axes=axes;}
        public DensitySampler compileSampler(DensityFunction.CompileContext context){return sampler;}
        public DensityFunction rewriteChildren(DfRewriteRule rule){return this;}
        public Interval range(){return range;}
        public int domainAxes(){return axes;}
        public com.mojang.serialization.MapCodec<? extends DensityFunction> codec(){throw new AssertionError("unexpected codec lookup");}
    }
    private static int cacheCount(DensityFunctionCompiler compiler) throws Exception {
        var field=DensityFunctionCompiler.class.getDeclaredField("nextCacheId");field.setAccessible(true);return field.getInt(compiler);
    }
    // The concrete structure key has an erased DensitySampler runtime contract.
    // Widen it solely to attach transport samplers that expose delegation calls.
    @SuppressWarnings("unchecked")
    private static net.minecraft.util.context.ContextKey<DensitySampler> structureKey() {
        return (net.minecraft.util.context.ContextKey<DensitySampler>)(net.minecraft.util.context.ContextKey<?>)Beardifier.CONTEXT_KEY;
    }
    public static void main(String[] paths) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var samplers=new ArrayList<DensitySampler>();
        var noises=new ArrayList<NoiseFunction.Sampler>();
        var splines=new ArrayList<CubicSpline<SplineFunction.Coordinate>>();
        var functions=new java.util.HashMap<Integer,DensityFunction>();
        var context=SamplerContext.EMPTY_UNCACHED;
        var counters=new ArrayList<long[]>();
        var graph=new ArrayList<DensityFunction>();
        var graphFunctions=new net.minecraft.core.MappedRegistry<DensityFunction>(net.minecraft.core.registries.Registries.DENSITY_FUNCTION,com.mojang.serialization.Lifecycle.stable());
        var namedFunctions=new java.util.HashMap<String,net.minecraft.core.Holder<DensityFunction>>();
        var graphSplines=new ArrayList<CubicSpline<SplineFunction.Coordinate>>();
        var graphNoises=new ArrayList<net.minecraft.core.Holder<NormalNoise>>();
        var graphContext=shiftContext(null);
        net.minecraft.core.MappedRegistry<NormalNoise> graphRegistry=null;
        long graphWorldSeed=0;boolean graphLegacy=false;
        var graphCompiler=new DensityFunctionCompiler(shiftContext(null));
        try(var reader=Files.newBufferedReader(Path.of(paths[0]),StandardCharsets.UTF_8);
            var out=new PrintWriter(Files.newBufferedWriter(Path.of(paths[1]),StandardCharsets.UTF_8))) {
            String line;
            while((line=reader.readLine())!=null) {
                var args=new StringTokenizer(line);
                switch(args.nextToken()) {
                    case "constant" -> { samplers.add(new ConstantFunction.Sampler(small(args)));out.println("create "+(samplers.size()-1)); }
                    case "gradient" -> {
                        var axis=Axis.values()[integer(args)];var tiling=TilingMode.values()[integer(args)];
                        int from=integer(args),to=integer(args);float low=small(args),high=small(args);
                        samplers.add(new GradientFunction(axis,tiling,from,to,low,high).compileSampler(null));
                        out.println("create "+(samplers.size()-1));
                    }
                    case "noise" -> {
                        String kind=args.nextToken(),stream=args.nextToken();long seed=Long.parseLong(args.nextToken());double scale=number(args);
                        RandomSource random=stream.equals("legacy")?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                        Noise noise=switch(kind) { case "normal"->NormalNoise.builder().setBaseOctave(-7).setOctaveCount(4).build().create(random);case "perlin"->new PerlinNoise(random);case "simplex"->new SimplexNoise(random);case "smeared"->new SmearedPerlinNoise(random,scale);default->throw new AssertionError(); };
                        var sampler=new NoiseFunction.Sampler(noise,number(args),number(args));noises.add(sampler);samplers.add(sampler);
                        out.printf("noise %d %d%n",noises.size()-1,samplers.size()-1);
                    }
                    case "shift" -> {
                        var noise=noises.get(integer(args));int x=integer(args),y=integer(args),z=integer(args);
                        samplers.add(y<0?new NoiseFunction.ShiftedXzSampler(samplers.get(x),samplers.get(z),noise.noise(),noise.xzScale(),noise.yScale()):new NoiseFunction.ShiftedXyzSampler(samplers.get(x),samplers.get(y),samplers.get(z),noise.noise(),noise.xzScale(),noise.yScale()));
                        out.println("create "+(samplers.size()-1));
                    }
                    case "unary" -> {
                        String kind=args.nextToken();var input=samplers.get(integer(args));float a=small(args),b=small(args);
                        DensitySampler result=switch(kind) {
                            case "abs" -> new UnaryFunction.AbsSampler(input);
                            case "negate" -> new UnaryFunction.NegateSampler(input);
                            case "square" -> new UnaryFunction.SquareSampler(input);
                            case "cube" -> new UnaryFunction.CubeSampler(input);
                            case "sqrt" -> new UnaryFunction.SqrtSampler(input);
                            case "log" -> new UnaryFunction.LogSampler(input);
                            case "reciprocal" -> new UnaryFunction.ReciprocalSampler(input);
                            case "sign" -> new UnaryFunction.SignSampler(input);
                            case "squeeze" -> new UnaryFunction.SqueezeSampler(input);
                            case "leaky" -> new UnaryFunction.LeakyReLUSampler(input,a);
                            case "add" -> new BinaryFunction.ConstAddSampler(input,a);
                            case "sub" -> new BinaryFunction.ConstSubSampler(a,input);
                            case "mul" -> new BinaryFunction.ConstMulSampler(input,a);
                            case "div" -> new BinaryFunction.ConstDivSampler(a,input);
                            case "min" -> new BinaryFunction.ConstMinSampler(input,a);
                            case "max" -> new BinaryFunction.ConstMaxSampler(input,a);
                            case "clamp" -> new ClampFunction.Sampler(input,a,b);
                            default -> throw new AssertionError();
                        };
                        samplers.add(result);out.println("create "+(samplers.size()-1));
                    }
                    case "round" -> {
                        var type=RoundFunction.Type.values()[integer(args)];var input=samplers.get(integer(args));int multiple=integer(args);
                        var result=multiple<0?construct("RoundFunction$IntegerMultipleSampler",new Class<?>[]{RoundFunction.Type.class,DensitySampler.class},type,input):
                            construct("RoundFunction$Sampler",new Class<?>[]{RoundFunction.Type.class,DensitySampler.class,DensitySampler.class},type,input,samplers.get(multiple));
                        samplers.add(result);out.println("create "+(samplers.size()-1));
                    }
                    case "power" -> {
                        int mode=integer(args);DensitySampler result;
                        if(mode==0)result=construct("PowFunction$Sampler",new Class<?>[]{DensitySampler.class,DensitySampler.class},samplers.get(integer(args)),samplers.get(integer(args)));
                        else if(mode==1)result=construct("PowFunction$ConstBaseSampler",new Class<?>[]{double.class,DensitySampler.class},number(args),samplers.get(integer(args)));
                        else if(mode==2)result=construct("PowFunction$ConstExponentSampler",new Class<?>[]{DensitySampler.class,double.class},samplers.get(integer(args)),number(args));
                        else throw new AssertionError();
                        samplers.add(result);out.println("create "+(samplers.size()-1));
                    }
                    case "binary" -> {
                        String kind=args.nextToken();var left=samplers.get(integer(args));var right=samplers.get(integer(args));float bound=small(args);
                        DensitySampler result=switch(kind) {
                            case "add" -> new BinaryFunction.AddSampler(left,right);
                            case "sub" -> new BinaryFunction.SubSampler(left,right);
                            case "mul" -> new BinaryFunction.MulSampler(left,right);
                            case "div" -> new BinaryFunction.DivSampler(left,right);
                            case "min" -> new BinaryFunction.MinSampler(left,right,bound);
                            case "max" -> new BinaryFunction.MaxSampler(left,right,bound);
                            default -> throw new AssertionError();
                        };
                        samplers.add(result);out.println("create "+(samplers.size()-1));
                    }
                    case "choice" -> {
                        int mode=integer(args);var input=samplers.get(integer(args));float min=small(args),max=small(args);
                        DensitySampler result;
                        if(mode==1)result=construct("RangeChoiceFunction$ConstSampler",new Class<?>[]{DensitySampler.class,float.class,float.class,float.class,float.class},input,min,max,small(args),small(args));
                        else result=construct("RangeChoiceFunction$Sampler",new Class<?>[]{DensitySampler.class,float.class,float.class,DensitySampler.class,DensitySampler.class},input,min,max,samplers.get(integer(args)),samplers.get(integer(args)));
                        samplers.add(result);out.println("create "+(samplers.size()-1));
                    }
                    case "lerp" -> {
                        int mode=integer(args);var alpha=samplers.get(integer(args));
                        DensitySampler result=switch(mode) {
                            case 0 -> new LerpFunction.Sampler(alpha,samplers.get(integer(args)),samplers.get(integer(args)));
                            case 1 -> new LerpFunction.ConstFirstSampler(alpha,small(args),samplers.get(integer(args)));
                            case 2 -> new LerpFunction.ConstSecondSampler(alpha,samplers.get(integer(args)),small(args));
                            default -> throw new AssertionError();
                        };
                        samplers.add(result);out.println("create "+(samplers.size()-1));
                    }
                    case "interval" -> {
                        int mode=integer(args);var input=samplers.get(integer(args));int count=integer(args);
                        float[] thresholds=new float[count];for(int i=0;i<count;i++)thresholds[i]=small(args);
                        DensitySampler[] branches=new DensitySampler[count+1];for(int i=0;i<=count;i++)branches[i]=samplers.get(integer(args));
                        var result=mode==0?construct("IntervalSelectFunction$Sampler",new Class<?>[]{DensitySampler.class,float[].class,DensitySampler[].class},input,thresholds,branches):
                            construct("IntervalSelectFunction$SingleThresholdSampler",new Class<?>[]{DensitySampler.class,float.class,DensitySampler.class,DensitySampler.class},input,thresholds[0],branches[0],branches[1]);
                        samplers.add(result);out.println("create "+(samplers.size()-1));
                    }
                    case "bounds" -> {
                        int mode=integer(args);float a=small(args),b=small(args),c=small(args),d=small(args),e=small(args),f=small(args);
                        Interval left;
                        try { left=switch(mode) {case 0->Interval.of(a,b);case 1->Interval.ofSymmetric(a);case 2->Interval.ofExact(a);case 3->Interval.encapsulating(a,b);case 4->Interval.NaI;case 5->Interval.INFINITE;default->throw new AssertionError();}; }
                        catch(IllegalArgumentException error) { out.println("bounds create error");break; }
                        final var first=left;var right=Interval.encapsulating(c,d);var third=Interval.encapsulating(e,f);
                        intervalOutput(out,"create",()->first);intervalOutput(out,"right",()->right);
                        intervalOutput(out,"add",()->Interval.add(first,right));intervalOutput(out,"sub",()->Interval.sub(first,right));
                        intervalOutput(out,"mul",()->Interval.mul(first,right));intervalOutput(out,"div",()->Interval.div(first,right));
                        intervalOutput(out,"min",()->Interval.min(first,right));intervalOutput(out,"max",()->Interval.max(first,right));
                        intervalOutput(out,"reciprocal",()->Interval.reciprocal(first));intervalOutput(out,"abs",()->Interval.abs(first));
                        intervalOutput(out,"square",()->Interval.square(first));intervalOutput(out,"sign",()->Interval.sign(first));
                        intervalOutput(out,"power",()->Interval.pow(first,right));
                        intervalOutput(out,"clamp",()->Interval.clamp(first,c,d));
                        intervalOutput(out,"log",()->Interval.log(first));
                        intervalOutput(out,"lerp-values",()->Interval.lerp(first,c,d));intervalOutput(out,"lerp-ranges",()->Interval.lerp(first,right,third));
                        intervalOutput(out,"map-negate",()->Interval.mapMonotonic(first,v->-v));intervalOutput(out,"map-cube",()->Interval.mapMonotonic(first,v->v*v*v));
                        intervalOutput(out,"map-sqrt",()->Interval.mapMonotonic(first,v->(float)Math.sqrt(v)));intervalOutput(out,"map-reciprocal",()->Interval.mapMonotonic(first,v->1.0f/v));
                        intervalOutput(out,"hull",()->Interval.encapsulating(first,right,third));
                        intervalOutput(out,"empty",()->Interval.encapsulating(new Interval[0]));
                        out.printf("bounds query %d %d %d %d%n",first.contains(e)?1:0,first.intersects(right)?1:0,first.equals(right)?1:0,first.equals(first)?1:0);
                    }
                    case "offset" -> {
                        int kind=integer(args);var contextForShift=shiftContext(noises.get(integer(args)).noise());
                        var sampler=switch(kind) {case 0->new ShiftNoiseFunction.Shift(null).compileSampler(contextForShift);case 1->new ShiftNoiseFunction.ShiftA(null).compileSampler(contextForShift);case 2->new ShiftNoiseFunction.ShiftB(null).compileSampler(contextForShift);default->throw new AssertionError();};
                        samplers.add(sampler);out.println("create "+(samplers.size()-1));
                    }
                    case "blended", "blendfail" -> {
                        String stream=args.nextToken();long seed=Long.parseLong(args.nextToken());
                        var definition=new BlendedNoise(number(args),number(args),number(args),number(args),number(args));
                        intervalOutput(out,"blended",definition::range);
                        RandomSource random=stream.equals("legacy")?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                        DensitySampler sampler=null;
                        try { sampler=definition.compileSampler(random); } catch(IllegalArgumentException e) { }
                        out.printf("blended %s %d%n",sampler==null?"error":"ok",random.nextLong());
                        if(sampler!=null) { samplers.add(sampler);out.println("create "+(samplers.size()-1)); }
                    }
                    case "blendfbm" -> {
                        String stream=args.nextToken();long seed=Long.parseLong(args.nextToken());int octave=integer(args);double smear=number(args),amplitude=number(args);
                        RandomSource random=stream.equals("legacy")?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                        fbmProbe(random,octave,smear,amplitude,out);
                    }
                    case "distance" -> {
                        var metric=DistanceMetric.values()[integer(args)];var point=new net.minecraft.core.Vec3i(integer(args),integer(args),integer(args));
                        samplers.add(new DistanceToPointFunction(point,metric).compileSampler(null));out.println("create "+(samplers.size()-1));
                    }
                    case "metric" -> {
                        float x=small(args),y=small(args),z=small(args);
                        for(var metric:DistanceMetric.values())out.printf("metric %s %s%n",bits(metric.compute(x,y)),bits(metric.compute(x,y,z)));
                    }
                    case "wfield" -> {
                        int field=integer(args),fallback=integer(args);
                        var key=switch(field){case 0->Blender.ALPHA_KEY;case 1->Blender.OFFSET_KEY;case 2->Beardifier.CONTEXT_KEY;default->throw new AssertionError();};
                        samplers.add(fallback<0?SimpleDensityFunction.values()[field].compileSampler(null):new ContextBoundSampler(key,samplers.get(fallback)));out.println("create "+(samplers.size()-1));
                    }
                    case "blenddensity" -> {
                        samplers.add(new BlendDensityFunction(new BoundFunction(samplers.get(integer(args)),Interval.INFINITE,7)).compileSampler(null));out.println("create "+(samplers.size()-1));
                    }
                    case "context","wcontext" -> {
                        boolean cached=integer(args)!=0;
                        var builder=SamplerContext.builder().useBufferArena(new DensityBufferArena() {
                            public ScopedDensityBuffer acquire(int size) { var result=DensityBufferArena.GLOBAL.acquire(size);result.fill(0.0f);return result; }
                            public void release(ScopedDensityBuffer buffer) { DensityBufferArena.GLOBAL.release(buffer); }
                        });
                        if(line.startsWith("wcontext ")) {
                            var fields=ContextMap.builder();
                            int alpha=integer(args),offset=integer(args),beard=integer(args),blender=integer(args);
                            if(alpha>=0)fields.set(Blender.ALPHA_KEY,samplers.get(alpha));
                            if(offset>=0)fields.set(Blender.OFFSET_KEY,samplers.get(offset));
                            if(beard>=0)fields.set(structureKey(),samplers.get(beard));
                            if(blender!=0)fields.set(Blender.CONTEXT_KEY,Blender.empty());
                            builder.setUserFields(fields.build());
                        }
                        if(cached)builder.enableCaches();context=builder.build();out.println("context");
                    }
                    case "counted" -> {
                        var input=samplers.get(integer(args));var calls=new long[2];counters.add(calls);
                        samplers.add(new DensitySampler() {
                            public float sampleValue(SamplerContext ctx,int x,int y,int z) { calls[0]++;return input.sampleValue(ctx,x,y,z); }
                            public void sampleVolume(SamplerContext ctx,DensityBuffer buffer,DensityVolume volume) { calls[1]++;input.sampleVolume(ctx,buffer,volume); }
                        });
                        out.printf("counted %d %d%n",counters.size()-1,samplers.size()-1);
                    }
                    case "counts" -> { var calls=counters.get(integer(args));out.printf("counts %d %d%n",calls[0],calls[1]); }
                    case "greset" -> {graphFunctions=new net.minecraft.core.MappedRegistry<>(net.minecraft.core.registries.Registries.DENSITY_FUNCTION,com.mojang.serialization.Lifecycle.stable());namedFunctions.clear();graph.clear();graphSplines.clear();graphNoises.clear();graphContext=shiftContext(null);graphCompiler=new DensityFunctionCompiler(graphContext);context=SamplerContext.EMPTY_UNCACHED;out.println("graph-reset");}
                    case "gend" -> {graph.add(new EndIslandFunction());out.println("graph-create "+(graph.size()-1));}
                    case "gseed" -> {
                        long seed=Long.parseLong(args.nextToken());boolean legacy=integer(args)!=0;
                        graphWorldSeed=seed;graphLegacy=legacy;
                        graphRegistry=new net.minecraft.core.MappedRegistry<>(net.minecraft.core.registries.Registries.NOISE,com.mojang.serialization.Lifecycle.stable());
                        // Original fixtures supply constructor-only material inputs;
                        // RandomState performs all seed derivation and noise creation.
                        for(var key:java.util.List.of(net.minecraft.world.level.levelgen.Noises.CLAY_BANDS_OFFSET,net.minecraft.world.level.levelgen.Noises.SURFACE,net.minecraft.world.level.levelgen.Noises.SURFACE_SECONDARY,net.minecraft.world.level.levelgen.Noises.BADLANDS_PILLAR,net.minecraft.world.level.levelgen.Noises.BADLANDS_PILLAR_ROOF,net.minecraft.world.level.levelgen.Noises.BADLANDS_SURFACE,net.minecraft.world.level.levelgen.Noises.ICEBERG_PILLAR,net.minecraft.world.level.levelgen.Noises.ICEBERG_PILLAR_ROOF,net.minecraft.world.level.levelgen.Noises.ICEBERG_SURFACE))
                            graphRegistry.register(key,NormalNoise.builder().setOctaveCount(0).build(),net.minecraft.core.RegistrationInfo.BUILT_IN);
                        graphContext=shiftContext(null);graphCompiler=new DensityFunctionCompiler(graphContext);
                        graphFunctions=new net.minecraft.core.MappedRegistry<>(net.minecraft.core.registries.Registries.DENSITY_FUNCTION,com.mojang.serialization.Lifecycle.stable());namedFunctions.clear();graph.clear();graphSplines.clear();graphNoises.clear();context=SamplerContext.EMPTY_UNCACHED;out.println("graph-seed");
                    }
                    case "gready" -> {
                        graphRegistry.freeze();
                        var zero=DensityFunctions.zero();var router=new net.minecraft.world.level.levelgen.NoiseRouter(zero,zero,zero,zero,zero,zero,zero,zero);
                        var state=net.minecraft.world.level.levelgen.RandomState.create(graphRegistry,graphWorldSeed,graphLegacy,net.minecraft.world.level.block.Blocks.STONE.defaultBlockState(),63,router);
                        var compilerField=state.getClass().getDeclaredField("densityFunctionCompiler");compilerField.setAccessible(true);graphCompiler=(DensityFunctionCompiler)compilerField.get(state);
                        var contextField=DensityFunctionCompiler.class.getDeclaredField("context");contextField.setAccessible(true);graphContext=(DensityFunction.CompileContext)contextField.get(graphCompiler);
                        out.println("graph-ready");
                    }
                    case "gregister" -> {
                        String name=args.nextToken();int base=integer(args);double amplitude=number(args);int count=integer(args),normalization=integer(args),modifiers=integer(args);
                        var builder=NormalNoise.builder().setBaseOctave(base).setBaseAmplitude(amplitude).setOctaveCount(count);
                        if(normalization==2)builder.setLegacyNormalization();else builder.setNormalize(normalization==1);
                        for(int i=0;i<modifiers;i++)builder.setAmplitudeModifier(i,number(args));
                        var key=net.minecraft.resources.ResourceKey.create(net.minecraft.core.registries.Registries.NOISE,net.minecraft.resources.Identifier.parse(name));
                        graphNoises.add(graphRegistry.register(key,builder.build(),net.minecraft.core.RegistrationInfo.BUILT_IN));out.println("graph-noise "+(graphNoises.size()-1));
                    }
                    case "ginstance" -> {var holder=graphNoises.get(integer(args));var a=graphContext.createNoiseSampler(holder);var b=graphContext.createNoiseSampler(holder);out.println("graph-instance "+(a==b?1:0));}
                    case "gsconstant" -> {graphSplines.add(CubicSpline.constant(small(args)));out.println("graph-spline "+(graphSplines.size()-1));}
                    case "gspoints" -> {
                        var coordinate=new SplineFunction.Coordinate(graph.get(integer(args)));int count=integer(args);
                        var locations=new float[count];var derivatives=new float[count];var values=new ArrayList<CubicSpline<SplineFunction.Coordinate>>();
                        for(int i=0;i<count;i++){locations[i]=small(args);derivatives[i]=small(args);values.add(graphSplines.get(integer(args)));}
                        graphSplines.add(new CubicSpline.Multipoint<>(coordinate,locations,values,derivatives));out.println("graph-spline "+(graphSplines.size()-1));
                    }
                    case "gref" -> {graph.add(new DensityFunctions.HolderHolder(net.minecraft.core.Holder.direct(graph.get(integer(args)))));out.println("graph-create "+(graph.size()-1));}
                    case "gnamed" -> {
                        String name=args.nextToken();var value=graph.get(integer(args));
                        // Distinct equal constant objects may have distinct registered holders.
                        if(value instanceof ConstantFunction constant)value=new ConstantFunction(constant.value());
                        var key=net.minecraft.resources.ResourceKey.create(net.minecraft.core.registries.Registries.DENSITY_FUNCTION,net.minecraft.resources.Identifier.parse(name));
                        var holder=graphFunctions.register(key,value,net.minecraft.core.RegistrationInfo.BUILT_IN);
                        // Bind each transport entry immediately, as freeze binds a loaded registry.
                        var bind=net.minecraft.core.Holder.Reference.class.getDeclaredMethod("bindValue",Object.class);bind.setAccessible(true);bind.invoke(holder,value);namedFunctions.put(name,holder);
                        graph.add(new DensityFunctions.HolderHolder(holder));out.println("graph-create "+(graph.size()-1));
                    }
                    case "glookup" -> {graph.add(new DensityFunctions.HolderHolder(namedFunctions.get(args.nextToken())));out.println("graph-create "+(graph.size()-1));}
                    case "gspline" -> {graph.add(new SplineFunction(graphSplines.get(integer(args))));out.println("graph-create "+(graph.size()-1));}
                    case "gbind" -> {
                        var sampler=samplers.get(integer(args));var range=Interval.encapsulating(small(args),small(args));int axes=integer(args);
                        graph.add(new BoundFunction(sampler,range,axes));out.println("graph-create "+(graph.size()-1));
                    }
                    case "gconst","ggradient","gunary","gbinary","gclamp","gchoice","glerp","gslice","gcache","ginterp","gsurface","ginterval","gdistance","gnoise","goffset","gblended","ground","gpow","gwfield","gblenddensity" -> {
                        String command=line.substring(0,line.indexOf(' '));
                        DensityFunction node=switch(command){
                            case "gconst"->new ConstantFunction(small(args));
                            case "gwfield"->SimpleDensityFunction.values()[integer(args)];
                            case "gblenddensity"->new BlendDensityFunction(graph.get(integer(args)));
                            case "gnoise"->new NoiseFunction(graphNoises.get(integer(args)),number(args),number(args),graph.get(integer(args)),graph.get(integer(args)),graph.get(integer(args)));
                            case "goffset"->{var noise=graphNoises.get(integer(args));yield switch(integer(args)){case 0->new ShiftNoiseFunction.Shift(noise);case 1->new ShiftNoiseFunction.ShiftA(noise);case 2->new ShiftNoiseFunction.ShiftB(noise);default->throw new AssertionError();};}
                            case "gblended"->new BlendedNoise(number(args),number(args),number(args),number(args),number(args));
                            case "ground"->new RoundFunction(RoundFunction.Type.values()[integer(args)],graph.get(integer(args)),graph.get(integer(args)));
                            case "gpow"->new PowFunction(graph.get(integer(args)),graph.get(integer(args)));
                            case "gdistance"->new DistanceToPointFunction(new net.minecraft.core.Vec3i(integer(args),integer(args),integer(args)),DistanceMetric.values()[integer(args)]);
                            case "ginterval"->{
                                var input=graph.get(integer(args));int count=integer(args);
                                float[] thresholds=new float[count];for(int i=0;i<count;i++)thresholds[i]=small(args);
                                var branches=new ArrayList<DensityFunction>();for(int i=0;i<=count;i++)branches.add(graph.get(integer(args)));
                                yield new IntervalSelectFunction(input,it.unimi.dsi.fastutil.floats.FloatArrayList.wrap(thresholds),branches);
                            }
                            case "ggradient"->new GradientFunction(Axis.values()[integer(args)],TilingMode.values()[integer(args)],integer(args),integer(args),small(args),small(args));
                            case "gunary"->new UnaryFunction(UnaryFunction.Type.values()[new int[]{0,1,2,4,5,6,7,8,10,9,3}[integer(args)]],graph.get(integer(args)));
                            case "gbinary"->new BinaryFunction(BinaryFunction.Type.values()[integer(args)],graph.get(integer(args)),graph.get(integer(args)));
                            case "gclamp"->new ClampFunction(graph.get(integer(args)),small(args),small(args));
                            case "gchoice"->new RangeChoiceFunction(graph.get(integer(args)),small(args),small(args),graph.get(integer(args)),graph.get(integer(args)));
                            case "glerp"->new LerpFunction(graph.get(integer(args)),graph.get(integer(args)),graph.get(integer(args)));
                            case "gslice"->new SliceFunction(Axis.values()[integer(args)],integer(args),graph.get(integer(args)));
                            case "gcache"->new CacheFunction(graph.get(integer(args)));
                            case "ginterp"->new InterpolatedFunction(graph.get(integer(args)),integer(args),integer(args));
                            case "gsurface"->new FindTopSurfaceFunction(graph.get(integer(args)),graph.get(integer(args)),integer(args),integer(args));
                            default->throw new AssertionError();
                        };
                        graph.add(node);out.println("graph-create "+(graph.size()-1));
                    }
                    case "ginfo" -> {var node=graph.get(integer(args));intervalOutput(out,"graph",node::range);out.println("graph-axes "+node.domainAxes());}
                    case "gsame" -> out.println("graph-same "+(graph.get(integer(args)).equals(graph.get(integer(args)))?1:0));
                    case "gcompile","gtry" -> {
                        String command=line.substring(0,line.indexOf(' '));int mode=integer(args);var node=graph.get(integer(args));
                        if(command.equals("gtry")){
                            String status="ok";
                            try{if(mode==0)node.compileSampler(graphContext);else graphCompiler.getSampler(node);}
                            catch(IllegalArgumentException | IllegalStateException e){status="error";}
                            out.printf("graph-try %s %d%n",status,cacheCount(graphCompiler));
                        }else{
                            var sampler=mode==0?node.compileSampler(graphContext):graphCompiler.getSampler(node);
                            int same=mode==0?-1:(sampler==graphCompiler.getSampler(node)?1:0);
                            samplers.add(sampler);out.printf("graph-compile %d %d %d%n",samplers.size()-1,cacheCount(graphCompiler),same);
                        }
                    }
                    case "spconstant" -> {
                        splines.add(CubicSpline.constant(small(args)));out.println("spline "+(splines.size()-1));
                    }
                    case "sppoints" -> {
                        int id=integer(args),count=integer(args);var function=functions.computeIfAbsent(id,key->new TransportFunction(key,samplers.get(key)));
                        var locations=new float[count];var derivatives=new float[count];var values=new ArrayList<CubicSpline<SplineFunction.Coordinate>>();
                        for(int i=0;i<count;i++){locations[i]=small(args);derivatives[i]=small(args);values.add(splines.get(integer(args)));}
                        splines.add(new CubicSpline.Multipoint<>(new SplineFunction.Coordinate(function),locations,values,derivatives));out.println("spline "+(splines.size()-1));
                    }
                    case "spfail" -> {
                        var spline=splines.get(integer(args));
                        try {new SplineFunction(spline).compileSampler(shiftContext(null));out.println("spline-compile ok");}
                        catch(IllegalArgumentException e){out.println("spline-compile error");}
                    }
                    case "sprange" -> {
                        var spline=splines.get(integer(args));
                        for(int i=0;i<3;i++)splineRanges[i]=Interval.encapsulating(small(args),small(args));
                        intervalOutput(out,"spline",spline::range);
                        java.util.Arrays.fill(splineRanges,Interval.INFINITE);
                    }
                    case "spsampler" -> {
                        samplers.add(new SplineFunction(splines.get(integer(args))).compileSampler(shiftContext(null)));out.println("create "+(samplers.size()-1));
                    }
                    case "surface" -> {
                        int mode=integer(args);var density=samplers.get(integer(args));var upper=samplers.get(integer(args));int lower=integer(args),height=integer(args);
                        var sampler=construct("FindTopSurfaceFunction$Sampler",new Class<?>[]{DensitySampler.class,DensitySampler.class,int.class,int.class},density,upper,lower,height);
                        samplers.add(mode==0?sampler:new SliceFunction.YSampler(sampler,0));out.println("create "+(samplers.size()-1));
                    }
                    case "interpolate" -> {
                        var input=samplers.get(integer(args));int xz=integer(args),y=integer(args);
                        samplers.add(construct("InterpolatedFunction$Sampler",new Class<?>[]{DensitySampler.class,int.class,int.class,float.class,float.class},input,xz,y,1.0f/xz,1.0f/y));
                        out.println("create "+(samplers.size()-1));
                    }
                    case "slice" -> {
                        int mode=integer(args);var input=samplers.get(integer(args));int a=integer(args),b=integer(args);
                        DensitySampler sampler=switch(mode) {
                            case 0->new SliceFunction.XSampler(input,a);
                            case 1->new SliceFunction.YSampler(input,a);
                            case 2->new SliceFunction.ZSampler(input,a);
                            case 3->new SliceFunction.XzSampler(input,a,b);
                            default->throw new AssertionError();
                        };
                        samplers.add(sampler);out.println("create "+(samplers.size()-1));
                    }
                    case "cache" -> { int id=integer(args);samplers.add(new CachingDensitySampler(id,samplers.get(integer(args))));out.println("create "+(samplers.size()-1)); }
                    case "lookup" -> {
                        var volume=new DensityVolume(integer(args),integer(args),integer(args),integer(args),integer(args),integer(args),integer(args),integer(args),integer(args));
                        out.println("lookup "+volume.indexOfBlock(integer(args),integer(args),integer(args)));
                    }
                    case "query" -> {
                        var sampler=samplers.get(integer(args));int x=integer(args),y=integer(args),z=integer(args);
                        try { out.println("query "+bits(sampler.sampleValue(context,x,y,z))); }
                        catch(ArithmeticException e) { out.println("query error"); }
                    }
                    case "volume" -> {
                        var sampler=samplers.get(integer(args));
                        int sx=integer(args),sy=integer(args),sz=integer(args),mx=integer(args),my=integer(args),mz=integer(args),dx=integer(args),dy=integer(args),dz=integer(args);
                        int tail=integer(args);long seed=Long.parseLong(args.nextToken());int repeats=integer(args);
                        var volume=new DensityVolume(sx,sy,sz,mx,my,mz,dx,dy,dz);var buffer=DensityBuffer.createUnpooled(volume.size()+tail);var random=new LegacyRandomSource(seed);
                        for(int i=0;i<buffer.size();i++)buffer.set(i,Float.intBitsToFloat(random.nextInt()));
                        for(int pass=0;pass<repeats;pass++) {
                            String status="ok";
                            try { sampler.sampleVolume(context,buffer,volume); } catch(ArithmeticException | IllegalArgumentException e) { status="error"; }
                            out.printf("volume %d %s%n",pass,status);
                            for(int i=0;i<buffer.size();i++)out.printf("cell %d %s%n",i,bits(buffer.get(i)));
                        }
                    }
                    default -> throw new AssertionError();
                }
                if(args.hasMoreTokens())throw new AssertionError("extra argument");
            }
            if(out.checkError())throw new java.io.IOException("reference output failed");
        }
    }
}
