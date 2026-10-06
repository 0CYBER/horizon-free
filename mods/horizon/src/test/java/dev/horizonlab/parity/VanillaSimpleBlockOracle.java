package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.resources.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.feature.SimpleBlockFeature;
import net.minecraft.world.level.levelgen.feature.Feature;
import net.minecraft.world.level.levelgen.placement.*;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.world.level.levelgen.feature.stateproviders.BlockStateProvider;

/** Actual native simple-block map codec and mutable world placement. */
public final class VanillaSimpleBlockOracle {
 private static final class Stop extends RuntimeException {}
 private static Optional<Feature> decode(RegistryOps<JsonElement> ops,JsonObject f) {
  var json=JsonParser.parseString(f.get("definition").getAsString());
  String mode=f.has("mode")?f.get("mode").getAsString():"map";
  if(mode.equals("map"))return SimpleBlockFeature.CODEC.codec().parse(ops,json).result().map(value->value);
  if(mode.equals("feature"))return Feature.CODEC.parse(ops,json).result().map(Holder::value);
  return PlacedFeature.CODEC.parse(ops,json).result().map(holder->new Feature(){
   public com.mojang.serialization.MapCodec<? extends Feature> codec(){return com.mojang.serialization.MapCodec.unit(this);}
   public boolean place(WorldGenLevel world,ChunkGenerator generator,RandomSource random,BlockPos p){return holder.value().place(world,generator,random,p);}
  });
 }
 private static BlockPos pos(JsonArray a){return new BlockPos(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt());}
 private static String xyz(BlockPos p){return p.getX()+" "+p.getY()+" "+p.getZ();}
 private static final class World implements InvocationHandler {
  final JsonObject f;final PrintWriter out;final RandomSource random;int calls;
  final TreeMap<BlockPos,BlockState> states=new TreeMap<>(Comparator.<BlockPos>comparingInt(BlockPos::getX).thenComparingInt(BlockPos::getY).thenComparingInt(BlockPos::getZ));
  World(JsonObject f,PrintWriter out,RandomSource random){this.f=f;this.out=out;this.random=random;for(var raw:f.getAsJsonArray("states")){var v=raw.getAsJsonArray();states.put(pos(v),Block.stateById(v.get(3).getAsInt()));}}
  void event(String text){out.println(text);if(calls++==f.get("fail").getAsInt())throw new Stop();}
  BlockState at(BlockPos p){return states.getOrDefault(p,Block.stateById(f.get("fallback").getAsInt()));}
  public Object invoke(Object proxy,Method method,Object[] args)throws Throwable {
   String name=method.getName();
   if(name.equals("getRandom"))return random;
   if(name.equals("getMinY"))return -64;
   if(name.equals("getHeight")&&(args==null||args.length==0))return 384;
   if(name.equals("getSeaLevel"))return 63;
   if(name.equals("getBlockState")){var p=(BlockPos)args[0];event("get "+xyz(p));var state=at(p);out.println("state "+Block.getId(state));return state;}
   if(name.equals("getFluidState")){var p=(BlockPos)args[0];event("fluid "+xyz(p));return (f.has("fluid")?Block.stateById(f.get("fluid").getAsInt()):at(p)).getFluidState();}
   if(name.equals("getRawBrightness")){var p=(BlockPos)args[0];event("light "+xyz(p)+" "+args[1]);return f.get("brightness").getAsInt();}
   if(name.equals("getBlockEntity")){event("entity "+xyz((BlockPos)args[0]));return null;}
   if(name.equals("setBlock")){
    var p=((BlockPos)args[0]).immutable();var state=(BlockState)args[1];int call=calls;event("set "+xyz(p)+" "+Block.getId(state)+" "+args[2]);
    int reject=f.get("reject").getAsInt();boolean result=reject==0||call%reject!=0;if(result)states.put(p,state);out.println("written "+result);return result;
   }
   if(name.equals("scheduleTick")){event("tick "+xyz((BlockPos)args[0])+" "+Block.getId(((Block)args[1]).defaultBlockState())+" "+args[2]);return null;}
   if(method.isDefault())return InvocationHandler.invokeDefault(proxy,method,args);
   throw new AssertionError(method);
  }
 }
 @SuppressWarnings("unchecked")
 public static void main(String[] args)throws Exception {
  net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
  if(args[0].equals("export")) {
   VanillaBlockSurvivalOracle.main(args);
   var data=new JsonObject();var doubles=new JsonArray();var moss=new JsonArray();
   for(var block:BuiltInRegistries.BLOCK){String name=BuiltInRegistries.BLOCK.getKey(block).toString();if(block instanceof DoublePlantBlock)doubles.add(name);if(block instanceof MossyCarpetBlock)moss.add(name);}
   data.add("double_plants",doubles);data.add("mossy_carpets",moss);Files.writeString(dir.resolve("placement-definitions.json"),data.toString());return;
  }
  VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());
  var bind=VanillaBlockSurvivalOracle.class.getDeclaredMethod("bindFluidTags",JsonObject.class);bind.setAccessible(true);bind.invoke(null,JsonParser.parseString(Files.readString(dir.resolve("fluid-tags.json"))).getAsJsonObject());
  var providers=new MappedRegistry<BlockStateProvider>(Registries.BLOCK_STATE_PROVIDER,com.mojang.serialization.Lifecycle.stable());
  var features=new MappedRegistry<Feature>(Registries.FEATURE,com.mojang.serialization.Lifecycle.stable());
  var placed=new MappedRegistry<PlacedFeature>(Registries.PLACED_FEATURE,com.mojang.serialization.Lifecycle.stable());
  var ops=RegistryOps.create(JsonOps.INSTANCE,new RegistryOps.RegistryInfoLookup(){public <T> Optional<HolderGetter<T>> lookup(ResourceKey<? extends Registry<? extends T>> key){
   if(key.equals(Registries.FEATURE))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(features);
   if(key.equals(Registries.PLACED_FEATURE))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(placed);
   if(key.equals(Registries.BLOCK_STATE_PROVIDER))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(providers);
   if(key.equals(Registries.FLUID))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(BuiltInRegistries.FLUID);
   return key.equals(Registries.BLOCK)?(Optional<HolderGetter<T>>)(Optional<?>)Optional.of(BuiltInRegistries.BLOCK):Optional.empty();
  }});
  for(var entry:JsonParser.parseString(Files.readString(dir.resolve("catalog.json"))).getAsJsonObject().entrySet()) {
   var provider=BlockStateProvider.DIRECT_CODEC.parse(ops,JsonParser.parseString(entry.getValue().getAsString())).getOrThrow();
   providers.register(ResourceKey.create(Registries.BLOCK_STATE_PROVIDER,Identifier.parse(entry.getKey())),provider,RegistrationInfo.BUILT_IN);
  }providers.freeze();
  var bundle=JsonParser.parseString(Files.readString(dir.resolve("feature-catalog.json"))).getAsJsonObject();
  for(var entry:bundle.getAsJsonObject("features").entrySet())features.register(ResourceKey.create(Registries.FEATURE,Identifier.parse(entry.getKey())),Feature.DIRECT_CODEC.parse(ops,JsonParser.parseString(entry.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);
  features.freeze();
  for(var entry:bundle.getAsJsonObject("placed").entrySet())placed.register(ResourceKey.create(Registries.PLACED_FEATURE,Identifier.parse(entry.getKey())),PlacedFeature.DIRECT_CODEC.parse(ops,JsonParser.parseString(entry.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);
  placed.freeze();
  var contextMethod=VanillaPlacementOracle.class.getDeclaredMethod("context",JsonObject.class);contextMethod.setAccessible(true);
  var bounds=new JsonObject();bounds.add("context",JsonParser.parseString("[-64,384,63]"));
  var generator=((PlacementContext)contextMethod.invoke(null,bounds)).generator();
  try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var inputs=Files.lines(dir.resolve("inputs.txt"))) {
   for(var line:(Iterable<String>)inputs::iterator){var f=JsonParser.parseString(line).getAsJsonObject();long seed=f.get("seed").getAsLong(),worldSeed=f.get("world_seed").getAsLong();boolean legacy=f.get("legacy").getAsBoolean();
    RandomSource random=legacy?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed),worldRandom=legacy?new LegacyRandomSource(worldSeed):new XoroshiroRandomSource(worldSeed);
    Optional<Feature> decoded;
    try{decoded=decode(ops,f);}catch(IllegalArgumentException|IllegalStateException error){decoded=Optional.empty();}
    if(decoded.isEmpty())out.println("decode error");else {
     out.println("decode ok");var handler=new World(f,out,worldRandom);var world=(WorldGenLevel)Proxy.newProxyInstance(WorldGenLevel.class.getClassLoader(),new Class<?>[]{WorldGenLevel.class},handler);var origin=pos(f.getAsJsonArray("origin"));
     for(int repeat=0;repeat<f.get("repeats").getAsInt();repeat++){
      handler.calls=0;try{out.println("place "+decoded.get().place(world,generator,random,origin));}catch(Stop|IllegalArgumentException error){out.println("place error");}
      out.println("next "+random.nextLong()+" "+worldRandom.nextLong());out.println("calls "+handler.calls);
      for(var entry:handler.states.entrySet())out.println("stored "+xyz(entry.getKey())+" "+Block.getId(entry.getValue()));out.println("end");
     }
    }out.println("tail "+random.nextLong()+" "+worldRandom.nextLong());
   }
  }
 }
}
