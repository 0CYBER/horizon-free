package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import com.google.gson.JsonParser;
import com.mojang.serialization.JsonOps;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.levelgen.XoroshiroRandomSource;
import net.minecraft.world.level.levelgen.synth.NormalNoise;

/** Original JSON transport; decoding and evaluation use vanilla classes. */
public final class VanillaWorldgenOracle {
    private static java.util.Map<net.minecraft.world.level.biome.MultiNoiseBiomeSourceParameterList.Preset,net.minecraft.world.level.biome.Climate.ParameterList<net.minecraft.resources.ResourceKey<net.minecraft.world.level.biome.Biome>>> biomePresets;
    private static void registryProbe(String path,long seed,boolean legacy,boolean optimized,String setting,PrintWriter out) throws Exception {
        registryProbe(path,seed,legacy,optimized,setting,null,out);
    }
    @SuppressWarnings("unchecked")
    private static void registryProbe(String path,long seed,boolean legacy,boolean optimized,String setting,String aquiferFixture,PrintWriter out) throws Exception {
        var bundle=JsonParser.parseString(Files.readString(Path.of(path))).getAsJsonObject();
        var noises=new net.minecraft.core.MappedRegistry<NormalNoise>(net.minecraft.core.registries.Registries.NOISE,com.mojang.serialization.Lifecycle.stable());
        var densities=new net.minecraft.core.MappedRegistry<net.minecraft.world.level.levelgen.densityfunction.DensityFunction>(net.minecraft.core.registries.Registries.DENSITY_FUNCTION,com.mojang.serialization.Lifecycle.stable());
        var noiseLookup=noises.createRegistrationLookup();var densityLookup=densities.createRegistrationLookup();
        var ops=net.minecraft.resources.RegistryOps.create(JsonOps.INSTANCE,new net.minecraft.resources.RegistryOps.RegistryInfoLookup() {
            public <T> java.util.Optional<net.minecraft.core.HolderGetter<T>> lookup(net.minecraft.resources.ResourceKey<? extends net.minecraft.core.Registry<? extends T>> key) {
                if(key.equals(net.minecraft.core.registries.Registries.NOISE))return (java.util.Optional<net.minecraft.core.HolderGetter<T>>)(java.util.Optional<?>)java.util.Optional.of(noiseLookup);
                if(key.equals(net.minecraft.core.registries.Registries.DENSITY_FUNCTION))return (java.util.Optional<net.minecraft.core.HolderGetter<T>>)(java.util.Optional<?>)java.util.Optional.of(densityLookup);
                return java.util.Optional.empty();
            }
        });
        for(var entry:bundle.getAsJsonObject("noises").entrySet()) {
            var key=net.minecraft.resources.ResourceKey.create(net.minecraft.core.registries.Registries.NOISE,net.minecraft.resources.Identifier.parse(entry.getKey()));
            var noise=NormalNoise.DIRECT_CODEC.parse(ops,JsonParser.parseString(entry.getValue().getAsString())).getOrThrow();
            noises.register(key,noise,net.minecraft.core.RegistrationInfo.BUILT_IN);
        }
        noises.freeze();
        java.util.Map<String,net.minecraft.world.level.levelgen.densityfunction.DensityFunction> functions=new java.util.TreeMap<>();
        for(var entry:bundle.getAsJsonObject("densities").entrySet()) {
            var key=net.minecraft.resources.ResourceKey.create(net.minecraft.core.registries.Registries.DENSITY_FUNCTION,net.minecraft.resources.Identifier.parse(entry.getKey()));
            var function=net.minecraft.world.level.levelgen.densityfunction.DensityFunctions.DIRECT_CODEC.parse(ops,JsonParser.parseString(entry.getValue().getAsString())).getOrThrow();
            densities.register(key,function,net.minecraft.core.RegistrationInfo.BUILT_IN);functions.put(entry.getKey(),function);
        }
        densities.freeze();
        var zero=net.minecraft.world.level.levelgen.densityfunction.DensityFunctions.zero();
        var router=new net.minecraft.world.level.levelgen.NoiseRouter(zero,zero,zero,zero,zero,zero,zero,zero);
        if(setting!=null) {
            var settings=JsonParser.parseString(bundle.getAsJsonObject("settings").get(setting).getAsString()).getAsJsonObject();
            router=net.minecraft.world.level.levelgen.NoiseRouter.CODEC.parse(ops,settings.get("noise_router")).getOrThrow();
            functions=new java.util.LinkedHashMap<>();
            functions.put("temperature",router.temperature());functions.put("vegetation",router.vegetation());
            functions.put("continents",router.continents());functions.put("erosion",router.erosion());
            functions.put("depth",router.depth());functions.put("ridges",router.ridges());
            functions.put("chunk_surface_level",router.chunkSurfaceLevel());functions.put("final_density",router.finalDensity());
        }
        var state=net.minecraft.world.level.levelgen.RandomState.create(noises,seed,legacy,net.minecraft.world.level.block.Blocks.STONE.defaultBlockState(),63,router);
        var compilerField=state.getClass().getDeclaredField("densityFunctionCompiler");compilerField.setAccessible(true);
        var compiler=(net.minecraft.world.level.levelgen.densityfunction.DensityFunctionCompiler)compilerField.get(state);
        var contextField=compiler.getClass().getDeclaredField("context");contextField.setAccessible(true);
        var compileContext=(net.minecraft.world.level.levelgen.densityfunction.DensityFunction.CompileContext)contextField.get(compiler);
        var countField=compiler.getClass().getDeclaredField("nextCacheId");countField.setAccessible(true);
        if(aquiferFixture!=null) {
            var fixture=JsonParser.parseString(aquiferFixture).getAsJsonObject();
            var settings=JsonParser.parseString(bundle.getAsJsonObject("settings").get(setting).getAsString()).getAsJsonObject();
            var configJson=fixture.has("config")?JsonParser.parseString(fixture.get("config").getAsString()):settings.get("aquifers");
            if(configJson==null){out.println("aquifer config error");return;}
            net.minecraft.world.level.levelgen.Aquifer.Config config;
            try{var decoded=net.minecraft.world.level.levelgen.Aquifer.Config.CODEC.parse(ops,configJson).result();
                if(decoded.isEmpty()){out.println("aquifer config error");return;}config=decoded.get();}
            catch(IllegalArgumentException | IllegalStateException e){out.println("aquifer config error");return;}
            noiseAquiferProbe(fixture,config,compiler,state,countField,out);return;
        }
        out.println(setting==null?"registry "+functions.size():"router "+setting+" "+functions.size());
        var sharedContext=createContext();
        for(var entry:functions.entrySet()) {
            var function=entry.getValue();var range=function.range();
            out.printf("entry %s %s %s %d%n",entry.getKey(),bits(range.min()),bits(range.max()),function.domainAxes());
            net.minecraft.world.level.levelgen.densityfunction.DensitySampler sampler;
            try { sampler=optimized?compiler.getSampler(function):function.compileSampler(compileContext); }
            catch(IllegalArgumentException | IllegalStateException e){out.println("compile error");continue;}
            out.println("compile ok "+countField.getInt(compiler));
            var context=setting==null?createContext():sharedContext;
            int[][] points={{0,0,0},{-17,-64,32},{30000000,319,-30000000},{1,-1,-1}};
            for(var point:points)out.println("density "+bits(sampler.sampleValue(context,point[0],point[1],point[2])));
            int[][] origins={{-17,-64,32},{-16,-64,32},{-17,0,32}},steps={{1,1,1},{4,8,4},{4,1,4}},extents={{2,9,2},{2,9,2},{3,1,2}};
            for(int v=0;v<(setting==null?2:3);v++) {
                var volume=new net.minecraft.world.level.levelgen.densityfunction.DensityVolume(extents[v][0],extents[v][1],extents[v][2],origins[v][0],origins[v][1],origins[v][2],steps[v][0],steps[v][1],steps[v][2]);
                var buffer=net.minecraft.world.level.levelgen.densityfunction.DensityBuffer.createUnpooled(volume.size());buffer.fill(0.0f);
                for(int repeat=0;repeat<2;repeat++) {
                    String status="ok";try{sampler.sampleVolume(context,buffer,volume);}catch(IllegalArgumentException | IllegalStateException e){status="error";}
                    out.println("volume "+status);for(int i=0;i<buffer.size();i++)out.println("cell "+bits(buffer.get(i)));
                }
            }
        }
        if(setting!=null) {
            var climate=state.createClimateSampler(sharedContext);
            out.println("climate caches "+countField.getInt(compiler));
            int[][] positions={{0,0,0},{-5,-16,8},{7500000,79,-7500000},{Integer.MIN_VALUE,Integer.MAX_VALUE,-1},{536870912,-536870913,17},{-4,0,8}};
            for(var position:positions) {
                var point=climate.sample(position[0],position[1],position[2]);
                out.printf("climate %d %d %d %d %d %d 0%n",point.temperature(),point.humidity(),point.continentalness(),point.erosion(),point.depth(),point.weirdness());
            }
            if(biomePresets==null)biomePresets=net.minecraft.world.level.biome.MultiNoiseBiomeSourceParameterList.knownPresets();
            var presets=new java.util.TreeMap<String,net.minecraft.world.level.biome.Climate.ParameterList<net.minecraft.resources.ResourceKey<net.minecraft.world.level.biome.Biome>>>();
            biomePresets.forEach((preset,list)->presets.put(preset.id().toString(),list));
            for(var preset:presets.entrySet()) {
                // Resolver lookup needs holder identities, not biome properties.
                // Registration placeholders retain actual keys without fabricating
                // biome implementations or invoking their gameplay behavior.
                var biomes=new net.minecraft.core.MappedRegistry<net.minecraft.world.level.biome.Biome>(net.minecraft.core.registries.Registries.BIOME,com.mojang.serialization.Lifecycle.stable());
                var lookup=biomes.createRegistrationLookup();
                var entries=new java.util.ArrayList<com.mojang.datafixers.util.Pair<net.minecraft.world.level.biome.Climate.ParameterPoint,net.minecraft.core.Holder<net.minecraft.world.level.biome.Biome>>>();
                for(var entry:preset.getValue().values())entries.add(com.mojang.datafixers.util.Pair.of(entry.getFirst(),lookup.getOrThrow(entry.getSecond())));
                var source=net.minecraft.world.level.biome.MultiNoiseBiomeSource.createFromList(new net.minecraft.world.level.biome.Climate.ParameterList<>(entries));
                out.println("biomes "+preset.getKey()+" "+entries.size());
                var resolver=source.createResolver(climate);
                for(var position:positions)out.println("biome "+resolver.getNoiseBiome(position[0],position[1],position[2]).unwrapKey().orElseThrow().identifier());
                int[][] minima={{-5,-16,8},{-4,-16,8},{Integer.MAX_VALUE-1,Integer.MIN_VALUE,-1}},extents={{3,4,2},{3,4,2},{2,2,2}};
                for(int v=0;v<3;v++) {
                    var min=minima[v];var size=extents[v];
                    var bulk=source.createResolverForChunk(climate,min[0],min[1],min[2],size[0],size[1],size[2]);
                    var queries=new java.util.ArrayList<int[]>();
                    for(int z=0;z<size[2];z++)for(int x=0;x<size[0];x++)for(int y=0;y<size[1];y++)queries.add(new int[]{min[0]+x,min[1]+y,min[2]+z});
                    var forward=new java.util.ArrayList<>(queries);var reverse=new java.util.ArrayList<>(queries);java.util.Collections.reverse(reverse);queries.addAll(reverse);queries.addAll(forward.subList(0,3));
                    for(var position:queries)out.println("biomecell "+bulk.getNoiseBiome(position[0],position[1],position[2]).unwrapKey().orElseThrow().identifier());
                }
            }

        }

    }
    private static net.minecraft.world.level.levelgen.densityfunction.SamplerContext createContext() {
            return net.minecraft.world.level.levelgen.densityfunction.SamplerContext.builder().enableCaches().useBufferArena(new net.minecraft.world.level.levelgen.densityfunction.DensityBufferArena() {
                public net.minecraft.world.level.levelgen.densityfunction.ScopedDensityBuffer acquire(int size) { var buffer=net.minecraft.world.level.levelgen.densityfunction.DensityBufferArena.GLOBAL.acquire(size);buffer.fill(0.0f);return buffer; }
                public void release(net.minecraft.world.level.levelgen.densityfunction.ScopedDensityBuffer buffer) { net.minecraft.world.level.levelgen.densityfunction.DensityBufferArena.GLOBAL.release(buffer); }
            }).build();
    }
    private static void leafOrder(Object node,PrintWriter out) throws Exception {
        try {
            var value=node.getClass().getDeclaredField("value");value.setAccessible(true);out.println("leaf "+value.get(node));
        } catch(NoSuchFieldException e) {
            var children=node.getClass().getDeclaredField("children");children.setAccessible(true);
            for(var child:(Object[])children.get(node))leafOrder(child,out);
        }
    }
    @SuppressWarnings("unchecked")
    private static void indexProbe(int branching,String json,PrintWriter out) throws Exception {
        var input=JsonParser.parseString(json).getAsJsonObject();
        var entries=new java.util.ArrayList<com.mojang.datafixers.util.Pair<net.minecraft.world.level.biome.Climate.ParameterPoint,Integer>>();
        for(var raw:input.getAsJsonArray("entries")) {
            var entry=raw.getAsJsonObject();var coordinates=entry.getAsJsonArray("coordinates");
            var p=new net.minecraft.world.level.biome.Climate.Parameter[6];
            for(int i=0;i<6;i++){var coord=coordinates.get(i).getAsJsonArray();p[i]=new net.minecraft.world.level.biome.Climate.Parameter(coord.get(0).getAsLong(),coord.get(1).getAsLong());}
            entries.add(com.mojang.datafixers.util.Pair.of(new net.minecraft.world.level.biome.Climate.ParameterPoint(p[0],p[1],p[2],p[3],p[4],p[5],entry.get("offset").getAsLong()),entries.size()));
        }
        // The Rust bounded API accepts branching 2..32. Other reference rebuild
        // values can diverge or fail to terminate and are outside this transport.
        if(branching<2 || branching>32){out.println("index error");return;}
        var constructor=net.minecraft.world.level.biome.Climate.ParameterList.class.getDeclaredConstructor(java.util.List.class,int.class);constructor.setAccessible(true);
        net.minecraft.world.level.biome.Climate.ParameterList<Integer> index;
        try{index=(net.minecraft.world.level.biome.Climate.ParameterList<Integer>)constructor.newInstance(entries,branching);}
        catch(java.lang.reflect.InvocationTargetException e){out.println("index error");return;}
        var indexField=index.getClass().getDeclaredField("index");indexField.setAccessible(true);var tree=indexField.get(index);
        var rootField=tree.getClass().getDeclaredField("root");rootField.setAccessible(true);
        var lastField=tree.getClass().getDeclaredField("lastResult");lastField.setAccessible(true);var last=(ThreadLocal<Object>)lastField.get(tree);
        out.println("index ok "+entries.size());leafOrder(rootField.get(tree),out);
        for(var target:input.getAsJsonArray("targets")) {
            var values=target.getAsJsonArray();var point=new net.minecraft.world.level.biome.Climate.TargetPoint(values.get(0).getAsLong(),values.get(1).getAsLong(),values.get(2).getAsLong(),values.get(3).getAsLong(),values.get(4).getAsLong(),values.get(5).getAsLong());
            int brute=index.findValueBruteForce(point);out.println("brute "+brute+" "+entries.get(brute).getFirst().fitness(point));
            try{int found=index.findValue(point);out.println("cached "+found+" "+entries.get(found).getFirst().fitness(point));}
            catch(NullPointerException e){out.println("cached error");}
            var previous=last.get();last.remove();
            try{int found=index.findValue(point);out.println("fresh "+found+" "+entries.get(found).getFirst().fitness(point));}
            catch(NullPointerException e){out.println("fresh error");}
            finally{if(previous==null)last.remove();else last.set(previous);}
        }
    }
    private static void settingsProbe(net.minecraft.world.level.levelgen.NoiseSettings settings,PrintWriter out) {
        out.println("settings "+settings.minY()+" "+settings.height());
        int[][] accessors={{-64,384},{0,128},{-2032,4064},{17,15},{2048,16},{Integer.MAX_VALUE,1},{Integer.MIN_VALUE,Integer.MAX_VALUE}};
        for(var accessor:accessors) {
            var clamped=settings.clampToHeightAccessor(net.minecraft.world.level.LevelHeightAccessor.create(accessor[0],accessor[1]));
            out.println("clamped "+clamped.minY()+" "+clamped.height());
        }
    }
    private static net.minecraft.world.level.block.state.BlockState fluidState(int label) {
        return switch(label) {
            case 0 -> net.minecraft.world.level.block.Blocks.WATER.defaultBlockState();
            case 1 -> net.minecraft.world.level.block.Blocks.LAVA.defaultBlockState();
            case 2 -> net.minecraft.world.level.block.Blocks.AIR.defaultBlockState();
            case 3 -> net.minecraft.world.level.block.Blocks.STONE.defaultBlockState();
            case 4 -> net.minecraft.world.level.block.Blocks.CAVE_AIR.defaultBlockState();
            case 5 -> net.minecraft.world.level.block.Blocks.VOID_AIR.defaultBlockState();
            case 6 -> net.minecraft.world.level.block.Blocks.WATER.defaultBlockState().setValue(net.minecraft.world.level.block.LiquidBlock.LEVEL,1);
            case 7 -> net.minecraft.world.level.block.Blocks.LAVA.defaultBlockState().setValue(net.minecraft.world.level.block.LiquidBlock.LEVEL,1);
            default -> throw new AssertionError("unknown fluid label");
        };
    }
    private static int fluidLabel(net.minecraft.world.level.block.state.BlockState state) {
        for(int label=0;label<8;label++)if(fluidState(label)==state)return label;
        throw new AssertionError("unknown fluid state");
    }
    private static void aquiferProbe(String line,PrintWriter out) throws Exception {
        if(net.minecraft.SharedConstants.DEBUG_DISABLE_FLUID_GENERATION)throw new AssertionError("reference requires regular fluid generation");
        var args=line.split(" ");int sea=Integer.parseInt(args[1]),label=Integer.parseInt(args[2]),x=Integer.parseInt(args[3]),y=Integer.parseInt(args[4]),z=Integer.parseInt(args[5]);
        double density=Double.longBitsToDouble(Long.parseUnsignedLong(args[6],16));
        var empty=new net.minecraft.world.level.levelgen.Aquifer.FluidStatus(-4064,fluidState(2));
        var lava=new net.minecraft.world.level.levelgen.Aquifer.FluidStatus(-54,fluidState(1));
        var fluid=new net.minecraft.world.level.levelgen.Aquifer.FluidStatus(sea,fluidState(label));
        var method=net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator.class.getDeclaredMethod("lambda$createFluidPicker$0",net.minecraft.world.level.levelgen.Aquifer.FluidStatus.class,int.class,net.minecraft.world.level.levelgen.Aquifer.FluidStatus.class,net.minecraft.world.level.levelgen.Aquifer.FluidStatus.class,int.class,int.class,int.class);method.setAccessible(true);
        net.minecraft.world.level.levelgen.Aquifer.FluidPicker picker=(px,py,pz)->{
            try{return (net.minecraft.world.level.levelgen.Aquifer.FluidStatus)method.invoke(null,empty,sea,lava,fluid,px,py,pz);}
            catch(ReflectiveOperationException e){throw new AssertionError(e);}
        };
        var status=picker.computeFluid(x,y,z);out.println("fluid "+status.fluidLevel()+" "+fluidLabel(status.fluidType())+" "+fluidLabel(status.at(y)));
        var calls=new java.util.concurrent.atomic.AtomicInteger();
        var aquifer=net.minecraft.world.level.levelgen.Aquifer.createDisabled((px,py,pz)->{calls.incrementAndGet();return picker.computeFluid(px,py,pz);});
        var substance=aquifer.computeSubstance(x,y,z,density);
        out.println("substance "+(substance==null?"default_solid":Integer.toString(fluidLabel(substance)))+" "+aquifer.shouldScheduleFluidUpdate()+" "+calls.get());
    }
    private static void noiseAquiferProbe(com.google.gson.JsonObject fixture,net.minecraft.world.level.levelgen.Aquifer.Config config,net.minecraft.world.level.levelgen.densityfunction.DensityFunctionCompiler compiler,net.minecraft.world.level.levelgen.RandomState state,java.lang.reflect.Field countField,PrintWriter out) throws Exception {
        var functions=new net.minecraft.world.level.levelgen.densityfunction.DensityFunction[]{config.barrierNoise(),config.fluidLevelFloodednessNoise(),config.fluidLevelSpreadNoise(),config.lavaNoise(),config.exclusion(),config.surfaceLevel()};
        var samplers=new net.minecraft.world.level.levelgen.densityfunction.DensitySampler[6];
        try{for(int i=0;i<6;i++)samplers[i]=compiler.getSampler(functions[i]);}
        catch(IllegalArgumentException | IllegalStateException e){out.println("aquifer compile error");return;}
        out.println("aquifer compile ok "+countField.getInt(compiler));
        var events=new java.util.ArrayList<String>();var context=createContext();var binding=new java.util.concurrent.atomic.AtomicInteger();
        net.minecraft.world.level.levelgen.densityfunction.DensitySamplerSet set=function->{
            int field=binding.getAndIncrement();var inner=samplers[field];
            if(fixture.has("overrides") && !fixture.getAsJsonArray("overrides").get(field).isJsonNull())inner=net.minecraft.world.level.levelgen.densityfunction.DensityFunctions.constant(Float.intBitsToFloat(Integer.parseUnsignedInt(fixture.getAsJsonArray("overrides").get(field).getAsString(),16))).compileSampler(null);
            var source=inner;
            return new net.minecraft.world.level.levelgen.densityfunction.DensitySampler(){
                private int calls;
                private void enter(String event){events.add(event);calls++;if(fixture.has("fail") && fixture.getAsJsonArray("fail").get(0).getAsInt()==field && fixture.getAsJsonArray("fail").get(1).getAsInt()==calls){events.add("sample failure "+field);throw new IllegalStateException("injected sampler failure");}}
                public float sampleValue(net.minecraft.world.level.levelgen.densityfunction.SamplerContext ctx,int x,int y,int z){
                    enter("sample "+field+" value "+x+" "+y+" "+z);
                    try{float value=source.sampleValue(ctx,x,y,z);events.add("value "+bits(value));return value;}
                    catch(IllegalArgumentException | IllegalStateException e){events.add("value error");throw e;}
                }
                public void sampleVolume(net.minecraft.world.level.levelgen.densityfunction.SamplerContext ctx,net.minecraft.world.level.levelgen.densityfunction.DensityBuffer buffer,net.minecraft.world.level.levelgen.densityfunction.DensityVolume volume){
                    enter("sample "+field+" volume "+volume.sizeX()+" "+volume.sizeY()+" "+volume.sizeZ()+" "+volume.minBlockX()+" "+volume.minBlockY()+" "+volume.minBlockZ()+" "+volume.stepBlockX()+" "+volume.stepBlockY()+" "+volume.stepBlockZ());
                    String result="ok";
                    try{source.sampleVolume(ctx,buffer,volume);}catch(IllegalArgumentException | IllegalStateException e){result="error";throw e;}
                    finally{events.add("sample volume "+result);for(int i=0;i<buffer.size();i++)events.add("sample cell "+bits(buffer.get(i)));}
                }
            }.bind(context);
        };
        int sea=fixture.get("sea").getAsInt(),label=fixture.get("fluid").getAsInt();
        var empty=new net.minecraft.world.level.levelgen.Aquifer.FluidStatus(-4064,fluidState(2));
        var lava=new net.minecraft.world.level.levelgen.Aquifer.FluidStatus(-54,fluidState(1));
        var fluid=new net.minecraft.world.level.levelgen.Aquifer.FluidStatus(sea,fluidState(label));
        var method=net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator.class.getDeclaredMethod("lambda$createFluidPicker$0",net.minecraft.world.level.levelgen.Aquifer.FluidStatus.class,int.class,net.minecraft.world.level.levelgen.Aquifer.FluidStatus.class,net.minecraft.world.level.levelgen.Aquifer.FluidStatus.class,int.class,int.class,int.class);method.setAccessible(true);
        net.minecraft.world.level.levelgen.Aquifer.FluidPicker picker=(x,y,z)->{try{var result=(net.minecraft.world.level.levelgen.Aquifer.FluidStatus)method.invoke(null,empty,sea,lava,fluid,x,y,z);events.add("pick "+x+" "+y+" "+z+" "+result.fluidLevel()+" "+fluidLabel(result.fluidType()));return result;}catch(ReflectiveOperationException e){throw new AssertionError(e);}};
        var size=fixture.getAsJsonArray("sizes");var min=fixture.getAsJsonArray("minimum");var steps=fixture.getAsJsonArray("steps");
        var volume=new net.minecraft.world.level.levelgen.densityfunction.DensityVolume(size.get(0).getAsInt(),size.get(1).getAsInt(),size.get(2).getAsInt(),min.get(0).getAsInt(),min.get(1).getAsInt(),min.get(2).getAsInt(),steps.get(0).getAsInt(),steps.get(1).getAsInt(),steps.get(2).getAsInt());
        net.minecraft.world.level.levelgen.Aquifer aquifer;
        try{aquifer=config.create(set,state.getOrCreateRandomFactory(net.minecraft.resources.Identifier.withDefaultNamespace("aquifer")),volume,picker);}
        catch(IllegalArgumentException | IllegalStateException e){events.forEach(out::println);out.println("aquifer create error");return;}
        events.forEach(out::println);events.clear();
        var skip=aquifer.getClass().getDeclaredField("skipSamplingAboveY");skip.setAccessible(true);out.println("aquifer ready "+skip.getInt(aquifer));
        for(var raw:fixture.getAsJsonArray("queries")){
            var query=raw.getAsJsonArray();String result;
            try{var block=aquifer.computeSubstance(query.get(0).getAsInt(),query.get(1).getAsInt(),query.get(2).getAsInt(),Double.longBitsToDouble(Long.parseUnsignedLong(query.get(3).getAsString(),16)));result=block==null?"default_solid":Integer.toString(fluidLabel(block));}
            catch(IllegalArgumentException | IllegalStateException | IndexOutOfBoundsException e){result="error";}
            events.forEach(out::println);events.clear();out.println("aquifer result "+result+" "+aquifer.shouldScheduleFluidUpdate());
        }
        var locationsField=aquifer.getClass().getDeclaredField("aquiferLocationCache");locationsField.setAccessible(true);var locations=(long[])locationsField.get(aquifer);
        var statusesField=aquifer.getClass().getDeclaredField("aquiferCache");statusesField.setAccessible(true);var statuses=(net.minecraft.world.level.levelgen.Aquifer.FluidStatus[])statusesField.get(aquifer);
        for(int i=0;i<locations.length;i++)out.println("aquifer cell "+i+" "+locations[i]+" "+(statuses[i]==null?"none":statuses[i].fluidLevel()+" "+fluidLabel(statuses[i].fluidType())));
        var surfacesField=aquifer.getClass().getDeclaredField("surfaceLevelCache");surfacesField.setAccessible(true);
        var surfaces=(it.unimi.dsi.fastutil.longs.Long2IntMap)surfacesField.get(aquifer);var sorted=new java.util.TreeMap<Long,Integer>();surfaces.long2IntEntrySet().forEach(entry->sorted.put(entry.getLongKey(),entry.getIntValue()));
        sorted.forEach((key,value)->out.println("aquifer surface "+key+" "+value));
    }

    private static void tagEntryProbe(String json,PrintWriter out){
        var decoded=net.minecraft.tags.TagEntry.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(json)).result();
        if(decoded.isEmpty())out.println("tag entry error");else out.println("tag entry "+decoded.get());
    }
    private static void tagFileProbe(String json,PrintWriter out){
        var decoded=net.minecraft.tags.TagFile.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(json)).result();
        if(decoded.isEmpty()){out.println("tag file error");return;}
        var file=decoded.get();out.println("tag file "+file.replace()+" "+file.entries().size());
        for(var entry:file.entries())out.println("tag entry "+entry);
    }
    private static void tagsProbe(com.google.gson.JsonObject fixture,PrintWriter out)throws Exception {
        out.println("tags resources "+fixture.getAsJsonArray("files").size());
        var definitions=new java.util.HashMap<net.minecraft.resources.Identifier,java.util.List<net.minecraft.tags.TagLoader.EntryWithSource>>();
        for(var raw:fixture.getAsJsonArray("files")){
            var row=raw.getAsJsonArray();var name=net.minecraft.resources.Identifier.parse(row.get(0).getAsString());String source=row.get(1).getAsString();
            var json=JsonParser.parseString(row.get(2).getAsString());var entries=definitions.computeIfAbsent(name,ignored->new java.util.ArrayList<>());
            var decoded=net.minecraft.tags.TagFile.CODEC.parse(JsonOps.INSTANCE,json).result();
            if(decoded.isEmpty()){out.println("tag resource error "+name+" "+source);continue;}
            var file=decoded.get();if(file.replace())entries.clear();
            for(var entry:file.entries())entries.add(new net.minecraft.tags.TagLoader.EntryWithSource(entry,source));
        }
        var elements=fixture.getAsJsonObject("elements");var trace=new java.util.ArrayList<String>();
        var loader=new net.minecraft.tags.TagLoader<String>((id,required)->{
            trace.add("tag lookup "+id+" "+required);
            return elements.has(id.toString())?java.util.Optional.of(elements.get(id.toString()).getAsString()):java.util.Optional.empty();
        },"tags/block");
        var result=loader.build(definitions);trace.forEach(out::println);
        var sorted=new java.util.TreeMap<String,java.util.List<String>>();result.forEach((key,value)->sorted.put(key.toString(),value));
        out.println("tags built "+sorted.size());
        for(var entry:sorted.entrySet()){out.println("tag members "+entry.getKey()+" "+entry.getValue().size());for(var value:entry.getValue())out.println("tag member "+value);}
        definitions.keySet().stream().filter(name->!result.containsKey(name)).map(Object::toString).sorted().forEach(name->out.println("tag missing "+name));
    }
    private static void tagRegistryProbe(String path,PrintWriter out)throws Exception {
        var bundle=JsonParser.parseString(Files.readString(Path.of(path))).getAsJsonObject();
        var fixture=new com.google.gson.JsonObject();var files=new com.google.gson.JsonArray();
        var sorted=new java.util.TreeMap<String,com.google.gson.JsonElement>();bundle.getAsJsonObject("tags").entrySet().forEach(entry->sorted.put(entry.getKey(),entry.getValue()));
        sorted.forEach((name,json)->{var row=new com.google.gson.JsonArray();row.add(name);row.add("vanilla");row.add(json);files.add(row);});fixture.add("files",files);
        // This lookup table must agree with the actual reference registry.
        var elements=bundle.getAsJsonObject("block_elements");
        if(elements.size()!=net.minecraft.core.registries.BuiltInRegistries.BLOCK.keySet().size())throw new AssertionError("block identifiers differ");
        for(var name:elements.keySet())if(!net.minecraft.core.registries.BuiltInRegistries.BLOCK.containsKey(net.minecraft.resources.Identifier.parse(name)))throw new AssertionError("unknown exported block");
        fixture.add("elements",elements);tagsProbe(fixture,out);
    }

    private static String bits(float value) { return Integer.toUnsignedString(Float.floatToIntBits(value)); }
    public static void main(String[] paths) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var octavesField=NormalNoise.class.getDeclaredField("octaves");octavesField.setAccessible(true);
        var factorField=NormalNoise.class.getDeclaredField("normalizationFactor");factorField.setAccessible(true);
        try(var reader=Files.newBufferedReader(Path.of(paths[0]),StandardCharsets.UTF_8);
            var out=new PrintWriter(Files.newBufferedWriter(Path.of(paths[1]),StandardCharsets.UTF_8))) {
            String line;
            while((line=reader.readLine())!=null) {
                if(line.startsWith("tag_entry ")){tagEntryProbe(line.substring(10),out);continue;}
                if(line.startsWith("tag_file ")){tagFileProbe(line.substring(9),out);continue;}
                if(line.startsWith("tag_registry ")){tagRegistryProbe(line.substring(13),out);continue;}
                if(line.startsWith("tags ")){tagsProbe(JsonParser.parseString(line.substring(5)).getAsJsonObject(),out);continue;}
                if(line.startsWith("noise_aquifer ")){var fields=line.split(" ",6);registryProbe(fields[3],Long.parseLong(fields[1]),fields[2].equals("1"),true,fields[4],fields[5],out);continue;}
                if(line.startsWith("aquifer ")){aquiferProbe(line,out);continue;}
                if(line.startsWith("noise_settings ")) {
                    var decoded=net.minecraft.world.level.levelgen.NoiseSettings.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(line.substring("noise_settings ".length()))).result();
                    if(decoded.isEmpty())out.println("settings error");else settingsProbe(decoded.get(),out);continue;
                }
                if(line.startsWith("settings_create ")) {
                    var values=line.split(" ");
                    try{settingsProbe(net.minecraft.world.level.levelgen.NoiseSettings.create(Integer.parseInt(values[1]),Integer.parseInt(values[2])),out);}
                    catch(IllegalStateException e){out.println("settings error");}continue;
                }
                if(line.startsWith("climate_parameter ")) {
                    var decoded=net.minecraft.world.level.biome.Climate.Parameter.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(line.substring("climate_parameter ".length()))).result();
                    if(decoded.isEmpty())out.println("parameter error");else{var p=decoded.get();out.println("parameter "+p.min()+" "+p.max());}continue;
                }
                if(line.startsWith("climate_point ")) {
                    var decoded=net.minecraft.world.level.biome.Climate.ParameterPoint.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(line.substring("climate_point ".length()))).result();
                    if(decoded.isEmpty())out.println("parameterpoint error");else{
                        var p=decoded.get();out.print("parameterpoint");
                        for(var coordinate:new net.minecraft.world.level.biome.Climate.Parameter[]{p.temperature(),p.humidity(),p.continentalness(),p.erosion(),p.depth(),p.weirdness()})out.print(" "+coordinate.min()+" "+coordinate.max());
                        out.println(" "+p.offset());
                    }continue;
                }
                if(line.startsWith("index ")) {var fields=line.split(" ",3);indexProbe(Integer.parseInt(fields[1]),fields[2],out);continue;}
                if(line.startsWith("quantize ")) {
                    var values=line.split(" ");out.print("quantized");
                    for(int i=1;i<=6;i++)out.print(" "+net.minecraft.world.level.biome.Climate.quantizeCoord(Float.intBitsToFloat(Integer.parseUnsignedInt(values[i],16))));
                    out.println();out.println("unquantized "+bits(net.minecraft.world.level.biome.Climate.unquantizeCoord(Long.parseLong(values[7]))));continue;
                }
                String[] args=line.split(" ",5);
                if(args[0].equals("router")){var fields=args[4].split(" ",2);registryProbe(fields[0],Long.parseLong(args[1]),args[2].equals("1"),args[3].equals("1"),fields[1],out);continue;}
                if(args[0].equals("registry")){registryProbe(args[4],Long.parseLong(args[1]),args[2].equals("1"),args[3].equals("1"),null,out);continue;}
                if(!args[0].equals("noise"))throw new AssertionError("unknown command");
                long seed=Long.parseLong(args[1]);boolean legacy=args[2].equals("1"),nether=args[3].equals("1");
                var decoded=NormalNoise.DIRECT_CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(args[4])).result();
                if(decoded.isEmpty()){out.println("decode error");continue;}
                var noise=decoded.get();var octaves=(java.util.List<?>)octavesField.get(noise);
                out.printf("decode %d %s %s %s%n",octaves.size(),Long.toUnsignedString(Double.doubleToLongBits(factorField.getDouble(noise))),bits(noise.range().min()),bits(noise.range().max()));
                for(var octave:octaves) {
                    var index=octave.getClass().getDeclaredMethod("octaveIndex");index.setAccessible(true);
                    var frequency=octave.getClass().getDeclaredMethod("frequency");frequency.setAccessible(true);
                    var amplitude=octave.getClass().getDeclaredMethod("amplitude");amplitude.setAccessible(true);
                    out.printf("octave %d %s %s%n",index.invoke(octave),Long.toUnsignedString(Double.doubleToLongBits((Double)frequency.invoke(octave))),Long.toUnsignedString(Double.doubleToLongBits((Double)amplitude.invoke(octave))));
                }
                net.minecraft.util.RandomSource random=legacy?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                net.minecraft.world.level.levelgen.synth.Noise sampler;
                try { sampler=nether?noise.createForLegacyNetherBiome(random):noise.create(random); }
                catch(IllegalArgumentException | IllegalStateException e){out.println("sample error "+random.nextLong());continue;}
                double[][] points={{0,0,0},{-17.25,64.125,32.5},{30e6,-64,-30e6},{1.25,-.5,-3.75},{33554432,-2048,-33554433}};
                for(var point:points)out.printf("query %s %s%n",bits(sampler.get(point[0],point[2])),bits(sampler.get(point[0],point[1],point[2])));
                out.println("parent "+random.nextLong());
            }
        }
    }
}
