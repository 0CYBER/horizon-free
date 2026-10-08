package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.function.Function;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.resources.*;
import net.minecraft.tags.*;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.densityfunction.*;
import net.minecraft.world.level.levelgen.material.*;
import net.minecraft.world.level.levelgen.material.condition.*;
import net.minecraft.world.level.levelgen.material.rule.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.placement.CaveSurface;
import net.minecraft.world.level.levelgen.synth.*;

/** Original transport for explicitly constructed material conditions and caches. */
public final class VanillaMaterialConditionOracle {
    private static MappedRegistry<NormalNoise> noises;
    private static MappedRegistry<MaterialCondition> definitions;
    private static MappedRegistry<Biome> biomes;
    private static RegistryOps<JsonElement> ops;
    private static MappedRegistry<DensityFunction> densities;
    private static HolderGetter<DensityFunction> densityLookup;
    private static MappedRegistry<MaterialRule> ruleDefinitions;
    private static HolderGetter<MaterialRule> ruleLookup;
    private static JsonElement canonical(JsonElement value) {
        if(value.isJsonObject()) {var result=new JsonObject();for(var e:new TreeMap<>(value.getAsJsonObject().asMap()).entrySet())result.add(e.getKey(),canonical(e.getValue()));return result;}
        if(value.isJsonArray()){var result=new JsonArray();for(var e:value.getAsJsonArray())result.add(canonical(e));return result;}
        if(value.isJsonPrimitive() && value.getAsJsonPrimitive().isNumber())return new JsonPrimitive("number:"+Long.toUnsignedString(Double.doubleToLongBits(value.getAsDouble()),16));
        return value;
    }
    private static String encode(MaterialCondition value) {return MaterialCondition.CODEC.encodeStart(ops,value).result().map(v->canonical(v).toString()).orElse("error");}
    @SuppressWarnings("unchecked")
    private static RegistryOps<JsonElement> conditionOps(HolderGetter<MaterialCondition> lookup) {
        return RegistryOps.create(JsonOps.INSTANCE,new RegistryOps.RegistryInfoLookup(){
            public <T> Optional<HolderGetter<T>> lookup(ResourceKey<? extends Registry<? extends T>> key){
                if(key.equals(Registries.NOISE))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(noises);
                if(key.equals(Registries.DENSITY_FUNCTION))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.ofNullable(densityLookup);
                if(key.equals(Registries.MATERIAL_RULE))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.ofNullable(ruleLookup);
                if(key.equals(Registries.BIOME))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(biomes);
                if(key.equals(Registries.MATERIAL_CONDITION))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(lookup);
                return Optional.empty();
            }
        });
    }
    private static void loadBiomes(Path dir) throws Exception {
        biomes=new MappedRegistry<>(Registries.BIOME,Lifecycle.stable());
        var climate=VanillaBiomeTemperatureOracle.class.getDeclaredMethod("climate",JsonObject.class);climate.setAccessible(true);
        var make=VanillaBiomeTemperatureOracle.class.getDeclaredMethod("make",Object.class);make.setAccessible(true);
        var definitions=JsonParser.parseString(Files.readString(dir.resolve("biomes.json"))).getAsJsonObject();
        for(var e:definitions.entrySet()) {
            var f=new JsonObject();f.addProperty("json",e.getValue().getAsString());
            var biome=(Biome)make.invoke(null,climate.invoke(null,f));
            biomes.register(ResourceKey.create(Registries.BIOME,Identifier.parse(e.getKey())),biome,RegistrationInfo.BUILT_IN);
        }
        biomes.freeze();
        var tagResources=JsonParser.parseString(Files.readString(dir.resolve("biome_tags.json"))).getAsJsonObject();
        var loader=new TagLoader<Holder<Biome>>((id,required)->biomes.get(id).map(value->(Holder<Biome>)value),"tags/worldgen/biome");
        var tags=new HashMap<Identifier,List<TagLoader.EntryWithSource>>();
        for(var e:tagResources.entrySet()) {
            var file=TagFile.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(e.getValue().getAsString())).getOrThrow();
            var entries=new ArrayList<TagLoader.EntryWithSource>();for(var entry:file.entries())entries.add(new TagLoader.EntryWithSource(entry,"vanilla"));
            tags.put(Identifier.parse(e.getKey()),entries);
        }
        var resolved=new HashMap<TagKey<Biome>,List<Holder<Biome>>>();
        loader.build(tags).forEach((id,values)->resolved.put(TagKey.create(Registries.BIOME,id),List.copyOf(values)));
        if(resolved.size()!=tagResources.size())throw new AssertionError("unresolved biome tags");
        biomes.prepareTagReload(new TagLoader.LoadResult<>(Registries.BIOME,resolved)).apply();
    }
    private static Holder<Biome> biome(String name) {return biomes.getOrThrow(ResourceKey.create(Registries.BIOME,Identifier.parse(name)));}
    private static String test(ConditionEvaluator condition) {try{return Boolean.toString(condition.test());}catch(RuntimeException error){return "error";}}
    private static void loadConditions(Path dir) throws Exception {
        definitions=new MappedRegistry<>(Registries.MATERIAL_CONDITION,Lifecycle.stable());
        ops=conditionOps(definitions.createRegistrationLookup());
        var data=JsonParser.parseString(Files.readString(dir.resolve("conditions.json"))).getAsJsonObject();
        for(var e:new TreeMap<>(data.asMap()).entrySet())definitions.register(ResourceKey.create(Registries.MATERIAL_CONDITION,Identifier.parse(e.getKey())),MaterialCondition.DIRECT_CODEC.parse(ops,JsonParser.parseString(e.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);
        definitions.freeze();ops=conditionOps(definitions);
    }
    private static void loadRules(Path dir) throws Exception {
        if(!Files.exists(dir.resolve("rules.json")))return;
        densities=new MappedRegistry<>(Registries.DENSITY_FUNCTION,Lifecycle.stable());densityLookup=densities.createRegistrationLookup();
        var densityJson=JsonParser.parseString(Files.readString(dir.resolve("densities.json"))).getAsJsonObject();
        for(var e:new TreeMap<>(densityJson.asMap()).entrySet())densities.register(ResourceKey.create(Registries.DENSITY_FUNCTION,Identifier.parse(e.getKey())),DensityFunctions.DIRECT_CODEC.parse(ops,JsonParser.parseString(e.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);
        densities.freeze();densityLookup=densities;
        ruleDefinitions=new MappedRegistry<>(Registries.MATERIAL_RULE,Lifecycle.stable());ruleLookup=ruleDefinitions.createRegistrationLookup();
        var data=JsonParser.parseString(Files.readString(dir.resolve("rules.json"))).getAsJsonObject();
        for(var e:new TreeMap<>(data.asMap()).entrySet())ruleDefinitions.register(ResourceKey.create(Registries.MATERIAL_RULE,Identifier.parse(e.getKey())),MaterialRule.DIRECT_CODEC.parse(ops,JsonParser.parseString(e.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);
        ruleDefinitions.freeze();ruleLookup=ruleDefinitions;
    }
    private static VerticalAnchor anchor(JsonElement e) {
        var a=e.getAsJsonArray();int v=a.get(1).getAsInt();
        return switch(a.get(0).getAsInt()) {case 0->VerticalAnchor.absolute(v);case 1->VerticalAnchor.aboveBottom(v);case 2->VerticalAnchor.belowTop(v);case 3->VerticalAnchor.relativeToSeaLevel(v);default->throw new AssertionError();};
    }
    private static MaterialCondition condition(JsonElement e) {
        var a=e.getAsJsonArray();return switch(a.get(0).getAsString()) {
            case "temperature"->TemperatureCondition.INSTANCE;
            case "above"->AbovePreliminarySurfaceCondition.INSTANCE;
            case "biome"->{var selected=new ArrayList<Holder<Biome>>();for(var name:a.get(1).getAsJsonArray())selected.add(biome(name.getAsString()));yield new BiomeCondition(HolderSet.direct(selected));}
            case "noise"->new NoiseThresholdCondition(ResourceKey.create(Registries.NOISE,Identifier.parse(a.get(1).getAsString())),Double.longBitsToDouble(Long.parseUnsignedLong(a.get(2).getAsString(),16)),Double.longBitsToDouble(Long.parseUnsignedLong(a.get(3).getAsString(),16)),a.get(4).getAsBoolean());
            case "json"->MaterialCondition.CODEC.parse(ops,JsonParser.parseString(a.get(1).getAsString())).getOrThrow();
            case "hole"->HoleCondition.INSTANCE;
            case "steep"->SteepCondition.INSTANCE;
            case "y"->new YCondition(anchor(a.get(1)),a.get(2).getAsInt(),a.get(3).getAsBoolean());
            case "water"->new WaterCondition(a.get(1).getAsInt(),a.get(2).getAsInt(),a.get(3).getAsBoolean());
            case "stone"->new StoneDepthCondition(a.get(1).getAsInt(),a.get(2).getAsBoolean(),a.get(3).getAsInt(),a.get(4).getAsBoolean()?CaveSurface.CEILING:CaveSurface.FLOOR);
            case "gradient"->new VerticalGradientCondition(Identifier.parse(a.get(1).getAsString()),anchor(a.get(2)),anchor(a.get(3)));
            case "not"->new NotCondition(condition(a.get(1)));
            default->throw new AssertionError("unknown condition "+a);
        };
    }
    private static BlockState[] BLOCKS;
    private static DensityFunction[] oreFunctions;
    private static int label(BlockState state){if(ruleDefinitions!=null)return net.minecraft.world.level.block.Block.getId(state);for(int i=0;i<BLOCKS.length;i++)if(state==BLOCKS[i])return i;throw new AssertionError("unexpected rule result "+state);}
    private static MaterialRule rule(JsonElement value) {
        var a=value.getAsJsonArray();return switch(a.get(0).getAsString()){
            case "block"->new BlockRule(BLOCKS[a.get(1).getAsInt()]);
            case "band"->BandlandsRule.INSTANCE;
            case "ore"->new OreVeinRule(BLOCKS[7],BLOCKS[8],BLOCKS[9],Float.intBitsToFloat(Integer.parseUnsignedInt(a.get(1).getAsString(),16)),oreFunctions[0],oreFunctions[1],oreFunctions[2]);
            case "if"->new ConditionRule(condition(a.get(1)),rule(a.get(2)));
            case "sequence"->{var rules=new ArrayList<MaterialRule>();for(var child:a.get(1).getAsJsonArray())rules.add(rule(child));yield new SequenceRule(rules);}
            default->throw new AssertionError("unknown rule "+a);
        };
    }
    @SuppressWarnings("unchecked")
    private static void fixture(JsonObject f,PrintWriter out) throws Exception {
        long seed=f.get("seed").getAsLong();boolean legacy=f.get("legacy").getAsBoolean();
        var zero=DensityFunctions.zero();var router=new NoiseRouter(zero,zero,zero,zero,zero,zero,zero,zero);
        var random=RandomState.create(noises,seed,legacy,Blocks.STONE.defaultBlockState(),63,router);var system=random.surfaceSystem();
        int[] depth={0};float[] secondary={0};
        if(!f.has("real_noise")||!f.get("real_noise").getAsBoolean()) {
            var noiseField=RandomState.class.getDeclaredField("noiseInstances");noiseField.setAccessible(true);
            var noiseCache=(Map<ResourceKey<NormalNoise>,Noise>)noiseField.get(random);
            for(var key:noises.registryKeySet())noiseCache.put(key,new Noise(){
                public net.minecraft.util.Interval range(){return net.minecraft.util.Interval.INFINITE;}
                public float get(double x,double z){throw new AssertionError("unexpected 2D noise call");}
                public float get(double x,double y,double z){out.println("noise query "+key.identifier()+" "+(int)x+" "+(int)y+" "+(int)z);return secondary[0];}
            });
        }
        Noise noise=new Noise() {
            public net.minecraft.util.Interval range(){return net.minecraft.util.Interval.INFINITE;}
            public float get(double x,double z){throw new AssertionError("unexpected 2D lookup");}
            public float get(double x,double y,double z){if(y!=0)throw new AssertionError("unexpected secondary Y");out.println("secondary query "+(int)x+" "+(int)z);return secondary[0];}
        };
        var secondaryField=MaterialSystem.class.getDeclaredField("surfaceSecondaryNoise");secondaryField.setAccessible(true);secondaryField.set(system,noise);
        var getContext=VanillaAnchorOracle.class.getDeclaredMethod("context",JsonObject.class);getContext.setAccessible(true);
        var generation=(WorldGenerationContext)getContext.invoke(null,f);
        var ctor=MaterialRuleContext.class.getDeclaredConstructors()[0];ctor.setAccessible(true);
        String[] currentBiome={"minecraft:plains"};
        Function<BlockPos,Holder<Biome>> biomeGetter=pos->{
            out.println("biome query "+pos.getX()+" "+pos.getY()+" "+pos.getZ());
            if(currentBiome[0]==null)return null;
            if(currentBiome[0].equals("#throw"))throw new IllegalStateException("fixture biome lookup failure");
            return biome(currentBiome[0]);
        };
        Set<Holder<Biome>> possible=null;
        if(f.has("possible_biomes")&&!f.get("possible_biomes").isJsonNull()){possible=new HashSet<>();for(var name:f.getAsJsonArray("possible_biomes"))possible.add(biome(name.getAsString()));}
        float[] density={0}; int[] remaining={-1};
        DensitySampler sampler=new DensitySampler(){
            public float sampleValue(SamplerContext c,int x,int y,int z){
                out.println("density query "+x+" "+y+" "+z);
                if(remaining[0]==0)throw new IllegalStateException("fixture density failure");
                if(remaining[0]>0)remaining[0]--;
                return density[0];
            }
            public void sampleVolume(SamplerContext c,DensityBuffer b,DensityVolume v){
                out.println("density volume "+v.sizeX()+" "+v.sizeY()+" "+v.sizeZ()+" "+v.minBlockX()+" "+v.minBlockY()+" "+v.minBlockZ()+" "+v.stepBlockX()+" "+v.stepBlockY()+" "+v.stepBlockZ());
                DensitySampler.sampleVolumeNaive(c,b,v,this);
            }
        };
        oreFunctions=new DensityFunction[]{DensityFunctions.constant(12345),DensityFunctions.constant(12346),DensityFunctions.constant(12347)};
        float[] oreValues={1,1,-1};int[] oreRemaining={-1,-1,-1};
        if(f.has("ore_values"))for(int i=0;i<3;i++)oreValues[i]=Float.intBitsToFloat(Integer.parseUnsignedInt(f.getAsJsonArray("ore_values").get(i).getAsString(),16));
        var oreSamplers=new DensitySampler[3];
        for(int id=0;id<3;id++) {final int channel=id;oreSamplers[id]=new DensitySampler(){
            public float sampleValue(SamplerContext c,int x,int y,int z){
                out.println("ore "+channel+" query "+x+" "+y+" "+z);
                if(oreRemaining[channel]==0)throw new IllegalStateException("fixture ore density failure");
                if(oreRemaining[channel]>0)oreRemaining[channel]--;
                return oreValues[channel];
            }
            public void sampleVolume(SamplerContext c,DensityBuffer b,DensityVolume v){
                out.println("ore "+channel+" volume "+v.sizeX()+" "+v.sizeY()+" "+v.sizeZ()+" "+v.minBlockX()+" "+v.minBlockY()+" "+v.minBlockZ()+" "+v.stepBlockX()+" "+v.stepBlockY()+" "+v.stepBlockZ());
                DensitySampler.sampleVolumeNaive(c,b,v,this);
            }
        };}
        DensitySamplerSet densitySamplers=function->{
            if(function==system.preliminarySurfaceFunction())return sampler.bind(SamplerContext.EMPTY_UNCACHED);
            for(int i=0;i<3;i++)if(function==oreFunctions[i])return oreSamplers[i].bind(SamplerContext.EMPTY_UNCACHED);
            throw new AssertionError("unexpected density function");
        };
        if(ruleDefinitions!=null) {
            densitySamplers=random.samplersWithContext(SamplerContext.builder().enableCaches().useBufferArena(DensityBufferArena.GLOBAL).build());
            if(f.has("library_json"))for(var value:f.getAsJsonArray("library_json"))out.println("rule decode "+(MaterialRule.DIRECT_CODEC.parse(ops,JsonParser.parseString(value.getAsString())).result().isPresent()?"ok":"error"));
        }
        int[] dims={16,1,16,0,0,0,1,1,1};if(f.has("volume"))for(int i=0;i<9;i++)dims[i]=f.getAsJsonArray("volume").get(i).getAsInt();
        var expected=new DensityVolume(dims[0],dims[1],dims[2],dims[3],dims[4],dims[5],dims[6],dims[7],dims[8]);
        var context=(MaterialRuleContext)ctor.newInstance(system,random,expected,densitySamplers,biomeGetter,generation,possible);
        var updateXZ=MaterialRuleContext.class.getDeclaredMethod("updateXZ",int.class,int.class,int.class,int.class);updateXZ.setAccessible(true);
        var updateY=MaterialRuleContext.class.getDeclaredMethod("updateY",int.class,int.class,int.class,int.class);updateY.setAccessible(true);
        var depthField=MaterialRuleContext.class.getDeclaredField("surfaceDepth");depthField.setAccessible(true);
        var conditions=new ArrayList<ConditionEvaluator>();
        for(var c:f.getAsJsonArray("conditions")) {var value=condition(c);out.println("runtime "+encode(value));conditions.add(value.compile(context));}
        if(f.has("json"))for(var c:f.getAsJsonArray("json")) {
            var value=MaterialCondition.CODEC.parse(ops,JsonParser.parseString(c.getAsString())).result();
            if(value.isEmpty())out.println("decode error");else {out.println("decode "+encode(value.get()));try{conditions.add(value.get().compile(context));}catch(IllegalStateException|NoSuchElementException error){out.println("compile error");}}
        }
        var rules=new ArrayList<RuleEvaluator>();
        if(f.has("rules"))for(var r:f.getAsJsonArray("rules"))rules.add(rule(r).compile(context));
        if(ruleDefinitions!=null)for(var name:f.getAsJsonArray("library_rules"))rules.add(ruleDefinitions.getValue(Identifier.parse(name.getAsString())).compile(context));
        for(var item:f.getAsJsonArray("ops")) {
            var op=item.getAsJsonArray();switch(op.get(0).getAsString()) {
                case "ore_input"->{int channel=op.get(1).getAsInt();oreValues[channel]=Float.intBitsToFloat(Integer.parseUnsignedInt(op.get(2).getAsString(),16));oreRemaining[channel]=op.get(3).getAsInt();}
                case "density"->{density[0]=Float.intBitsToFloat(Integer.parseUnsignedInt(op.get(1).getAsString(),16));remaining[0]=op.get(2).getAsInt();}
                case "minimum"->{try{out.println("minimum "+context.getMinSurfaceLevel());}catch(RuntimeException error){out.println("minimum error");}}
                case "biome"->currentBiome[0]=op.get(1).isJsonNull()?null:op.get(1).getAsString();
                case "rule"->{try{rules.add(rule(op.get(1)).compile(context));out.println("rule added "+(rules.size()-1));}catch(RuntimeException error){out.println("rule compile error");}}
                case "apply"->{for(int i=0;i<rules.size();i++){
                    try{var value=rules.get(i).tryApply(op.get(1).getAsInt(),op.get(2).getAsInt(),op.get(3).getAsInt());out.println("rule "+i+" "+(value==null?"none":Integer.toString(label(value))));}
                    catch(RuntimeException error){out.println("rule "+i+" error");}
                }}
                case "input"->{depth[0]=op.get(1).getAsInt();secondary[0]=Float.intBitsToFloat(Integer.parseUnsignedInt(op.get(2).getAsString(),16));}
                case "xz"->{updateXZ.invoke(context,op.get(1).getAsInt(),op.get(2).getAsInt(),op.get(3).getAsInt(),op.get(4).getAsInt());depthField.setInt(context,depth[0]);}
                case "y"->updateY.invoke(context,op.get(1).getAsInt(),op.get(2).getAsInt(),op.get(3).getAsInt(),op.get(4).getAsInt());
                case "add"->{conditions.add(condition(op.get(1)).compile(context));out.println("added "+(conditions.size()-1));}
                case "eval"->{if(op.size()==1){for(int i=0;i<conditions.size();i++)out.println("eval "+i+" "+test(conditions.get(i)));}else{for(var i:op.get(1).getAsJsonArray())out.println("eval "+i.getAsInt()+" "+test(conditions.get(i.getAsInt())));}}
                case "secondary"->out.println("secondary "+Long.toUnsignedString(Double.doubleToLongBits(context.getSurfaceSecondary()),16));
                default->throw new AssertionError("unknown operation "+op);
            }
        }
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        BLOCKS=Arrays.stream(new String[]{"terracotta","orange_terracotta","yellow_terracotta","brown_terracotta","red_terracotta","white_terracotta","light_gray_terracotta","stone","air","water"}).map(name->BuiltInRegistries.BLOCK.getValue(Identifier.withDefaultNamespace(name)).defaultBlockState()).toArray(BlockState[]::new);
        Path dir=Path.of(args[1]);noises=new MappedRegistry<>(Registries.NOISE,Lifecycle.stable());
        var noiseDefinitions=JsonParser.parseString(Files.readString(dir.resolve("noises.json"))).getAsJsonObject();
        for(var e:noiseDefinitions.entrySet())noises.register(ResourceKey.create(Registries.NOISE,Identifier.parse(e.getKey())),NormalNoise.DIRECT_CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(e.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);
        noises.freeze();loadBiomes(dir);loadConditions(dir);loadRules(dir);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var name:definitions.keySet().stream().map(Object::toString).sorted().toList())out.println("registry "+name+" "+encode(definitions.getValue(Identifier.parse(name))));
            for(String line:Files.readAllLines(dir.resolve("inputs.txt")))fixture(JsonParser.parseString(line).getAsJsonObject(),out);
        }
    }
}
