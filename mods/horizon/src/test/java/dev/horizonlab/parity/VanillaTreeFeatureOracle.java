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
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.entity.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.feature.Feature;
import net.minecraft.world.level.levelgen.feature.stateproviders.BlockStateProvider;

/**
 * Actual native tree, fallen-tree and ore features over a layered synthetic
 * world backed by proto-chunk sections. Logs ordered level writes, beehive
 * occupants, post-processing marks, results, random tails and the final diff.
 */
public final class VanillaTreeFeatureOracle {
 private static final int MIN_Y=-64,HEIGHT=384;
 private static BlockPos pos(JsonArray a){return new BlockPos(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt());}
 private static String xyz(BlockPos p){return p.getX()+" "+p.getY()+" "+p.getZ();}
 private static final class Chunks {
  static final PalettedContainerFactory FACTORY;
  static {
   Holder<Biome> biome=Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},Biomes.PLAINS);
   var ids=new IdMapper<Holder<Biome>>();ids.add(biome);
   FACTORY=new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),Blocks.AIR.defaultBlockState(),null,Strategy.createForBiomes(ids),biome,null);
  }
 }
 /** Shared with the Rust fixture: layered column, pseudo-random holes and obstacles, overrides. */
 static final class Terrain {
  final int[][] layers;final int[][] boxes;final int holeMod,holeState,obstacleMod,obstacleState,obstacleMin;final Map<BlockPos,Integer> overrides=new HashMap<>();
  Terrain(JsonObject f){
   var raw=f.getAsJsonArray("layers");layers=new int[raw.size()][];for(int i=0;i<raw.size();i++){var l=raw.get(i).getAsJsonArray();layers[i]=new int[]{l.get(0).getAsInt(),l.get(1).getAsInt()};}
   var rawBoxes=f.has("boxes")?f.getAsJsonArray("boxes"):new JsonArray();boxes=new int[rawBoxes.size()][7];
   for(int i=0;i<rawBoxes.size();i++)for(int j=0;j<7;j++)boxes[i][j]=rawBoxes.get(i).getAsJsonArray().get(j).getAsInt();
   holeMod=f.get("hole_mod").getAsInt();holeState=f.get("hole_state").getAsInt();obstacleMod=f.get("obstacle_mod").getAsInt();obstacleState=f.get("obstacle_state").getAsInt();obstacleMin=f.get("obstacle_min").getAsInt();
   for(var e:f.getAsJsonArray("states")){var v=e.getAsJsonArray();overrides.put(pos(v),v.get(3).getAsInt());}
  }
  static int hash(int x,int y,int z){int h=x*73856093^y*19349663^z*83492791;h^=h>>>13;h*=0x5bd1e995;h^=h>>>15;return h&0x7fffffff;}
  int at(int x,int y,int z){
   var o=overrides.get(new BlockPos(x,y,z));if(o!=null)return o;
   for(var b:boxes)if(x>=b[0]&&y>=b[1]&&z>=b[2]&&x<=b[3]&&y<=b[4]&&z<=b[5])return b[6];
   int base=-1;for(var l:layers)if(y<=l[0]){base=l[1];break;}
   int h=hash(x,y,z);
   if(base>=0){if(holeMod>0&&h%holeMod==0)return holeState;return base;}
   if(obstacleMod>0&&y>=obstacleMin&&h%obstacleMod==0)return obstacleState;
   return Block.getId(Blocks.AIR.defaultBlockState());
  }
 }
 private static final class World implements InvocationHandler {
  final JsonObject f;final PrintWriter out;final Terrain terrain;final Map<ChunkPos,ProtoChunk> chunks=new HashMap<>();final Map<BlockPos,BeehiveBlockEntity> hives=new HashMap<>();final Map<BlockPos,ChestBlockEntity> chests=new HashMap<>();final Map<BlockPos,SpawnerBlockEntity> spawners=new HashMap<>();
  final RandomSource shapeRandom=RandomSource.create(0);
  World(JsonObject f,PrintWriter out){this.f=f;this.out=out;this.terrain=new Terrain(f);}
  ProtoChunk chunk(int cx,int cz){
   return chunks.computeIfAbsent(new ChunkPos(cx,cz),cp->{
    var chunk=new ProtoChunk(cp,UpgradeData.EMPTY,LevelHeightAccessor.create(MIN_Y,HEIGHT),Chunks.FACTORY,null){
     @Override public void markPosForPostProcessing(BlockPos position){out.println("post "+xyz(position));super.markPosForPostProcessing(position);}
    };
    for(int s=0;s<HEIGHT/16;s++){var section=chunk.getSection(s);for(int ly=0;ly<16;ly++)for(int lz=0;lz<16;lz++)for(int lx=0;lx<16;lx++){
     var state=Block.stateById(terrain.at(cx*16+lx,MIN_Y+s*16+ly,cz*16+lz));if(!state.isAir()||!state.is(Blocks.AIR))section.setBlockState(lx,ly,lz,state,false);}}
    return chunk;
   });
  }
  BlockState get(BlockPos p){
   if(p.getY()<MIN_Y||p.getY()>=MIN_Y+HEIGHT)return Blocks.VOID_AIR.defaultBlockState();
   return chunk(p.getX()>>4,p.getZ()>>4).getBlockState(p);
  }
  public Object invoke(Object proxy,Method method,Object[] args)throws Throwable {
   String name=method.getName();
   switch(name){
    case "getBlockState":return get((BlockPos)args[0]);
    case "getFluidState":return get((BlockPos)args[0]).getFluidState();
    case "isStateAtPosition":return ((java.util.function.Predicate<BlockState>)args[1]).test(get((BlockPos)args[0]));
    case "isFluidAtPosition":return ((java.util.function.Predicate<net.minecraft.world.level.material.FluidState>)args[1]).test(get((BlockPos)args[0]).getFluidState());
    case "getMinY":return MIN_Y;
    case "getSeaLevel":return 63;
    case "getRawBrightness":case "getMaxLocalRawBrightness":return 15;
    case "getBrightness":return 15;
    case "getSeed":return f.get("world_seed").getAsLong();
    case "ensureCanWrite":return true;
    case "getRandom":return shapeRandom;
    case "scheduleTick":return null;  // Edge shape updates also schedule ticks; not compared.
    case "setCurrentlyGenerating":case "playSound":case "levelEvent":case "gameEvent":case "blockEvent":return null;
    case "getEntities":return java.util.List.of();
    case "environmentAttributes":return net.minecraft.world.attribute.EnvironmentAttributeReader.EMPTY;  // attribute defaults
    case "getBlockEntity":{
     if(args.length!=1)break;var p=((BlockPos)args[0]).immutable();var state=get(p);
     if(state.is(Blocks.CHEST))return chests.computeIfAbsent(p,key->new ChestBlockEntity(key,state){@Override public void setLootTable(net.minecraft.resources.ResourceKey<net.minecraft.world.level.storage.loot.LootTable> table,long seed){out.println("loot "+xyz(key)+" "+table.identifier()+" "+seed);super.setLootTable(table,seed);}});
     if(state.is(Blocks.SPAWNER))return spawners.computeIfAbsent(p,key->new SpawnerBlockEntity(key,state){@Override public void setEntityId(net.minecraft.world.entity.EntityType<?> type,RandomSource r){super.setEntityId(type,r);out.println("spawner "+xyz(key)+" "+BuiltInRegistries.ENTITY_TYPE.getKey(type));}});
     if(!state.is(Blocks.BEE_NEST))return null;
     return hives.computeIfAbsent(p,key->new BeehiveBlockEntity(key,state){@Override public void storeBee(BeehiveBlockEntity.Occupant occupant){out.println("bee "+xyz(key)+" "+occupant.ticksInHive());}});
    }
    case "setBlock":{
     var p=((BlockPos)args[0]).immutable();var state=(BlockState)args[1];out.println("set "+xyz(p)+" "+Block.getId(state)+" "+args[2]);
     if(p.getY()<MIN_Y||p.getY()>=MIN_Y+HEIGHT)return false;
     chunk(p.getX()>>4,p.getZ()>>4).getSection((p.getY()-MIN_Y)>>4).setBlockState(p.getX()&15,p.getY()&15,p.getZ()&15,state,false);return true;
    }
    case "getHeight":if(args==null||args.length==0)return HEIGHT;return f.get("height").getAsInt();
    case "getChunk":
     if(args.length==1&&args[0] instanceof BlockPos p)return chunk(p.getX()>>4,p.getZ()>>4);
     if(args.length>=2&&args[0] instanceof Integer x&&args[1] instanceof Integer z)return chunk(x,z);
     break;
    default:break;
   }
   if(method.isDefault())return InvocationHandler.invokeDefault(proxy,method,args);
   throw new AssertionError(method);
  }
 }
 /** Parses named holders on first use from the bundled vanilla resources. */
 static final class Lazy<T> implements HolderGetter<T>, HolderOwner<T> {
  JsonObject json;com.mojang.serialization.Codec<T> codec;RegistryOps<JsonElement> ops;final Map<ResourceKey<T>,Holder.Reference<T>> cache=new HashMap<>();
  public Optional<Holder.Reference<T>> get(ResourceKey<T> key){
   var cached=cache.get(key);if(cached!=null)return Optional.of(cached);
   var text=json.get(key.identifier().toString());if(text==null)return Optional.empty();
   var ref=Holder.Reference.createStandAlone(this,key);cache.put(key,ref);
   T value=codec.parse(ops,JsonParser.parseString(text.getAsString())).getOrThrow();
   try{var bind=Holder.Reference.class.getDeclaredMethod("bindValue",Object.class);bind.setAccessible(true);bind.invoke(ref,value);}catch(ReflectiveOperationException e){throw new IllegalStateException(e);}
   return Optional.of(ref);
  }
  public Optional<HolderSet.Named<T>> get(net.minecraft.tags.TagKey<T> tag){return Optional.empty();}
 }
 static final Lazy<Feature> lazyFeatures=new Lazy<>();
 static final Lazy<net.minecraft.world.level.levelgen.placement.PlacedFeature> lazyPlaced=new Lazy<>();
 @SuppressWarnings("unchecked")
 public static void main(String[] args)throws Exception {
  net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
  VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());
  var bind=VanillaBlockSurvivalOracle.class.getDeclaredMethod("bindFluidTags",JsonObject.class);bind.setAccessible(true);bind.invoke(null,JsonParser.parseString(Files.readString(dir.resolve("fluid-tags.json"))).getAsJsonObject());
  var providers=new MappedRegistry<BlockStateProvider>(Registries.BLOCK_STATE_PROVIDER,com.mojang.serialization.Lifecycle.stable());
  var ops=RegistryOps.create(JsonOps.INSTANCE,new RegistryOps.RegistryInfoLookup(){public <T> Optional<HolderGetter<T>> lookup(ResourceKey<? extends Registry<? extends T>> key){
   if(key.equals(Registries.BLOCK_STATE_PROVIDER))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(providers);
   if(key.equals(Registries.FEATURE))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(lazyFeatures);
   if(key.equals(Registries.PLACED_FEATURE))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(lazyPlaced);
   if(key.equals(Registries.FLUID))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(BuiltInRegistries.FLUID);
   return key.equals(Registries.BLOCK)?(Optional<HolderGetter<T>>)(Optional<?>)Optional.of(BuiltInRegistries.BLOCK):Optional.empty();
  }});
  var bundle=JsonParser.parseString(Files.readString(dir.resolve("vanilla-features.json"))).getAsJsonObject();
  for(var entry:bundle.getAsJsonObject("providers").entrySet())
   providers.register(ResourceKey.create(Registries.BLOCK_STATE_PROVIDER,Identifier.parse(entry.getKey())),BlockStateProvider.DIRECT_CODEC.parse(ops,JsonParser.parseString(entry.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);
  providers.freeze();
  var features=bundle.getAsJsonObject("features");var placedJson=bundle.getAsJsonObject("placed");
  lazyFeatures.json=features;lazyPlaced.json=placedJson;lazyFeatures.codec=Feature.DIRECT_CODEC;lazyPlaced.codec=net.minecraft.world.level.levelgen.placement.PlacedFeature.DIRECT_CODEC;lazyFeatures.ops=ops;lazyPlaced.ops=ops;
  var contextMethod=VanillaPlacementOracle.class.getDeclaredMethod("context",JsonObject.class);contextMethod.setAccessible(true);
  var bounds=new JsonObject();bounds.add("context",JsonParser.parseString("[-64,384,63]"));
  var generator=((net.minecraft.world.level.levelgen.placement.PlacementContext)contextMethod.invoke(null,bounds)).generator();
  try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var inputs=Files.lines(dir.resolve("inputs.txt"))) {
   for(var line:(Iterable<String>)inputs::iterator){
    var f=JsonParser.parseString(line).getAsJsonObject();long seed=f.get("seed").getAsLong();
    RandomSource random=f.get("legacy").getAsBoolean()?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
    out.println("fixture "+f.get("feature").getAsString());
    Optional<Feature> decoded;
    try{decoded=Feature.DIRECT_CODEC.parse(ops,JsonParser.parseString(features.get(f.get("feature").getAsString()).getAsString())).result();}catch(Throwable error){decoded=Optional.empty();}
    if(decoded.isEmpty()){out.println("decode error");continue;}
    var handler=new World(f,out);var world=(WorldGenLevel)Proxy.newProxyInstance(WorldGenLevel.class.getClassLoader(),new Class<?>[]{WorldGenLevel.class},handler);
    try{out.println("place "+decoded.get().place(world,generator,random,pos(f.getAsJsonArray("origin"))));}catch(Throwable error){out.println("place error");System.err.println(f.get("feature").getAsString()+": "+error);}
    out.println("next "+random.nextLong());
    var keys=new ArrayList<>(handler.chunks.keySet());keys.sort(Comparator.<ChunkPos>comparingInt(ChunkPos::x).thenComparingInt(ChunkPos::z));
    var changes=new TreeMap<BlockPos,Integer>(Comparator.<BlockPos>comparingInt(BlockPos::getX).thenComparingInt(BlockPos::getY).thenComparingInt(BlockPos::getZ));
    for(var key:keys){var chunk=handler.chunks.get(key);
     for(int y=MIN_Y;y<MIN_Y+HEIGHT;y++)for(int lz=0;lz<16;lz++)for(int lx=0;lx<16;lx++){
      int x=key.x()*16+lx,z=key.z()*16+lz;int id=Block.getId(chunk.getSection((y-MIN_Y)>>4).getBlockState(lx,y&15,lz));
      if(id!=handler.terrain.at(x,y,z))changes.put(new BlockPos(x,y,z),id);
     }}
    for(var e:changes.entrySet())out.println("final "+xyz(e.getKey())+" "+e.getValue());
    out.println("end");
   }
  }
 }
}
