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
  final int[][] layers;final int holeMod,holeState,obstacleMod,obstacleState,obstacleMin;final Map<BlockPos,Integer> overrides=new HashMap<>();
  Terrain(JsonObject f){
   var raw=f.getAsJsonArray("layers");layers=new int[raw.size()][];for(int i=0;i<raw.size();i++){var l=raw.get(i).getAsJsonArray();layers[i]=new int[]{l.get(0).getAsInt(),l.get(1).getAsInt()};}
   holeMod=f.get("hole_mod").getAsInt();holeState=f.get("hole_state").getAsInt();obstacleMod=f.get("obstacle_mod").getAsInt();obstacleState=f.get("obstacle_state").getAsInt();obstacleMin=f.get("obstacle_min").getAsInt();
   for(var e:f.getAsJsonArray("states")){var v=e.getAsJsonArray();overrides.put(pos(v),v.get(3).getAsInt());}
  }
  static int hash(int x,int y,int z){int h=x*73856093^y*19349663^z*83492791;h^=h>>>13;h*=0x5bd1e995;h^=h>>>15;return h&0x7fffffff;}
  int at(int x,int y,int z){
   var o=overrides.get(new BlockPos(x,y,z));if(o!=null)return o;
   int base=-1;for(var l:layers)if(y<=l[0]){base=l[1];break;}
   int h=hash(x,y,z);
   if(base>=0){if(holeMod>0&&h%holeMod==0)return holeState;return base;}
   if(obstacleMod>0&&y>=obstacleMin&&h%obstacleMod==0)return obstacleState;
   return Block.getId(Blocks.AIR.defaultBlockState());
  }
 }
 private static final class World implements InvocationHandler {
  final JsonObject f;final PrintWriter out;final Terrain terrain;final Map<ChunkPos,ProtoChunk> chunks=new HashMap<>();final Map<BlockPos,BeehiveBlockEntity> hives=new HashMap<>();
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
    case "ensureCanWrite":return true;
    case "getRandom":return shapeRandom;
    case "scheduleTick":return null;
    case "setCurrentlyGenerating":return null;
    case "getBlockEntity":{
     if(args.length!=1)break;var p=((BlockPos)args[0]).immutable();var state=get(p);
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
 @SuppressWarnings("unchecked")
 public static void main(String[] args)throws Exception {
  net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
  VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());
  var bind=VanillaBlockSurvivalOracle.class.getDeclaredMethod("bindFluidTags",JsonObject.class);bind.setAccessible(true);bind.invoke(null,JsonParser.parseString(Files.readString(dir.resolve("fluid-tags.json"))).getAsJsonObject());
  var providers=new MappedRegistry<BlockStateProvider>(Registries.BLOCK_STATE_PROVIDER,com.mojang.serialization.Lifecycle.stable());
  var ops=RegistryOps.create(JsonOps.INSTANCE,new RegistryOps.RegistryInfoLookup(){public <T> Optional<HolderGetter<T>> lookup(ResourceKey<? extends Registry<? extends T>> key){
   if(key.equals(Registries.BLOCK_STATE_PROVIDER))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(providers);
   if(key.equals(Registries.FLUID))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(BuiltInRegistries.FLUID);
   return key.equals(Registries.BLOCK)?(Optional<HolderGetter<T>>)(Optional<?>)Optional.of(BuiltInRegistries.BLOCK):Optional.empty();
  }});
  var bundle=JsonParser.parseString(Files.readString(dir.resolve("vanilla-features.json"))).getAsJsonObject();
  for(var entry:bundle.getAsJsonObject("providers").entrySet())
   providers.register(ResourceKey.create(Registries.BLOCK_STATE_PROVIDER,Identifier.parse(entry.getKey())),BlockStateProvider.DIRECT_CODEC.parse(ops,JsonParser.parseString(entry.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);
  providers.freeze();
  var features=bundle.getAsJsonObject("features");
  try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var inputs=Files.lines(dir.resolve("inputs.txt"))) {
   for(var line:(Iterable<String>)inputs::iterator){
    var f=JsonParser.parseString(line).getAsJsonObject();long seed=f.get("seed").getAsLong();
    RandomSource random=f.get("legacy").getAsBoolean()?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
    out.println("fixture "+f.get("feature").getAsString());
    Optional<Feature> decoded;
    try{decoded=Feature.DIRECT_CODEC.parse(ops,JsonParser.parseString(features.get(f.get("feature").getAsString()).getAsString())).result();}catch(RuntimeException error){decoded=Optional.empty();}
    if(decoded.isEmpty()){out.println("decode error");continue;}
    var handler=new World(f,out);var world=(WorldGenLevel)Proxy.newProxyInstance(WorldGenLevel.class.getClassLoader(),new Class<?>[]{WorldGenLevel.class},handler);
    try{out.println("place "+decoded.get().place(world,null,random,pos(f.getAsJsonArray("origin"))));}catch(RuntimeException error){out.println("place error "+error);}
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
