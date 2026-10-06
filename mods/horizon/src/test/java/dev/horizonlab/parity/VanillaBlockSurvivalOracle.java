package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import net.minecraft.core.*;
import net.minecraft.resources.*;
import net.minecraft.tags.*;
import net.minecraft.world.level.material.Fluid;
import com.mojang.serialization.JsonOps;
import java.util.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.entity.ShulkerBoxBlockEntity;
import net.minecraft.world.level.block.state.*;
/** Native survival calls over changing neighbor states and independent query failures. */
public final class VanillaBlockSurvivalOracle {
 private static final class Stop extends RuntimeException {}
 private static Class<?> declaring(Class<?> type,String name,Class<?>... parameters) {
  while(type!=null){try{type.getDeclaredMethod(name,parameters);return type;}catch(NoSuchMethodException e){type=type.getSuperclass();}}
  return null;
 }
    private static void bindFluidTags(JsonObject resources) {
        var registry=BuiltInRegistries.FLUID;
        var loader=new TagLoader<Holder<Fluid>>((id,required)->registry.get(id).map(value->(Holder<Fluid>)value),"tags/fluid");
        var definitions=new HashMap<Identifier,List<TagLoader.EntryWithSource>>();
        for(var item:resources.entrySet()) {
            var file=TagFile.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(item.getValue().getAsString())).getOrThrow();
            var entries=new ArrayList<TagLoader.EntryWithSource>();
            for(var entry:file.entries())entries.add(new TagLoader.EntryWithSource(entry,"vanilla"));
            definitions.put(Identifier.parse(item.getKey()),entries);
        }
        var tags=new HashMap<TagKey<Fluid>,List<Holder<Fluid>>>();
        loader.build(definitions).forEach((id,values)->tags.put(TagKey.create(Registries.FLUID,id),List.copyOf(values)));
        if(tags.size()!=resources.size())throw new AssertionError("incomplete fluid tags");
        registry.prepareTagReload(new TagLoader.LoadResult<>(Registries.FLUID,tags)).apply();
    }
 public static void main(String[] args)throws Exception {
  net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
  if(args[0].equals("export")||args[0].equals("definitions")) {
   if(args[0].equals("export")) {
   VanillaBlockPropertiesOracle.main(new String[]{"export",args[1]});VanillaBlockSupportOracle.main(new String[]{"export",args[1]});VanillaBlockStateCodecOracle.main(new String[]{"export",dir.resolve("facts.json").toString()});
   }
   var extra=new JsonObject();var fenceGates=new JsonArray();var ignition=new JsonObject();
   for(var block:BuiltInRegistries.BLOCK)if(block instanceof FenceGateBlock)fenceGates.add(BuiltInRegistries.BLOCK.getKey(block).toString());
   var ignitionField=FireBlock.class.getDeclaredField("igniteOdds");ignitionField.setAccessible(true);
   var odds=(it.unimi.dsi.fastutil.objects.Object2IntMap<?>)ignitionField.get(Blocks.FIRE);
   for(var block:BuiltInRegistries.BLOCK)ignition.addProperty(BuiltInRegistries.BLOCK.getKey(block).toString(),odds.getInt(block));
   extra.add("fence_gates",fenceGates);extra.add("ignition_odds",ignition);Files.writeString(dir.resolve("survival-extra-definitions.json"),extra.toString());
   var definitions=new JsonObject();var placement=new JsonObject();var supportTags=new JsonObject();
   for(var block:BuiltInRegistries.BLOCK){
    String name=BuiltInRegistries.BLOCK.getKey(block).toString();
    Class<?> type=declaring(block.getClass(),"canSurvive",BlockState.class,LevelReader.class,BlockPos.class);
    definitions.addProperty(name,type.getSimpleName());
    type=declaring(block.getClass(),"mayPlaceOn",BlockState.class,BlockGetter.class,BlockPos.class);
    if(type!=null)placement.addProperty(name,type.getSimpleName());
    var fields=new JsonObject();
    for(Class<?> c=block.getClass();c!=null;c=c.getSuperclass())for(var field:c.getDeclaredFields())if(field.getType().equals(net.minecraft.tags.TagKey.class)&&!Modifier.isStatic(field.getModifiers())) {
     field.setAccessible(true);var tag=(net.minecraft.tags.TagKey<?>)field.get(block);if(tag!=null)fields.addProperty(field.getName(),tag.location().toString());
    }
    if(fields.size()>0)supportTags.add(name,fields);
   }
   Files.writeString(dir.resolve("survival-classes.json"),definitions.toString());
   Files.writeString(dir.resolve("survival-placement-classes.json"),placement.toString());
   Files.writeString(dir.resolve("survival-support-tags.json"),supportTags.toString());return;
  }
  VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());
  bindFluidTags(JsonParser.parseString(Files.readString(dir.resolve("fluid-tags.json"))).getAsJsonObject());
  var allocator=Class.forName("sun.misc.Unsafe");var access=allocator.getDeclaredField("theUnsafe");access.setAccessible(true);var unsafe=access.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
  var oldProgress=ShulkerBoxBlockEntity.class.getDeclaredField("progressOld");oldProgress.setAccessible(true);var progress=ShulkerBoxBlockEntity.class.getDeclaredField("progress");progress.setAccessible(true);
  var animation=ShulkerBoxBlockEntity.class.getDeclaredField("animationStatus");animation.setAccessible(true);
  try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var inputs=Files.lines(dir.resolve("inputs.txt"))) {
   for(var line:(Iterable<String>)inputs::iterator){
    var f=JsonParser.parseString(line).getAsJsonObject();var candidate=Block.stateById(f.get("state").getAsInt());var xyz=f.getAsJsonArray("position");var p=new BlockPos(xyz.get(0).getAsInt(),xyz.get(1).getAsInt(),xyz.get(2).getAsInt());var neighbors=f.getAsJsonArray("neighbors");int[] reads={0},entities={0},lights={0},fluids={0};
    Object entity=null;String mode=f.get("entity").getAsString();if(!mode.equals("none")){entity=allocate.invoke(unsafe,ShulkerBoxBlockEntity.class);Object status=null;for(var value:animation.getType().getEnumConstants())if(((Enum<?>)value).name().equals(mode.toUpperCase(java.util.Locale.ROOT)))status=value;if(status==null)throw new AssertionError(mode);animation.set(entity,status);}if(entity!=null && f.has("shulker_progress")){var data=f.getAsJsonArray("shulker_progress");oldProgress.setFloat(entity,Float.intBitsToFloat(Integer.parseUnsignedInt(data.get(0).getAsString(),16)));progress.setFloat(entity,Float.intBitsToFloat(Integer.parseUnsignedInt(data.get(1).getAsString(),16)));}Object selected=entity;
    var world=(LevelReader)Proxy.newProxyInstance(LevelReader.class.getClassLoader(),new Class<?>[]{LevelReader.class},(proxy,method,values)->{
     if(method.getName().equals("getFluidState")){var at=(BlockPos)values[0];out.println("fluid "+at.getX()+" "+at.getY()+" "+at.getZ());int count=fluids[0]++;if(count==(f.has("fluid_fail")?f.get("fluid_fail").getAsInt():-1))throw new Stop();var states=f.has("fluid_states")?f.getAsJsonArray("fluid_states"):neighbors;return Block.stateById(states.get(count%states.size()).getAsInt()).getFluidState();}
     if(method.getName().equals("getRawBrightness")){var at=(BlockPos)values[0];out.println("light "+at.getX()+" "+at.getY()+" "+at.getZ()+" "+values[1]);if(lights[0]++==(f.has("light_fail")?f.get("light_fail").getAsInt():-1))throw new Stop();return f.has("brightness")?f.get("brightness").getAsInt():15;}
     if(method.getName().equals("getBlockState")){var at=(BlockPos)values[0];out.println("read "+at.getX()+" "+at.getY()+" "+at.getZ());int count=reads[0]++;if(count==f.get("fail").getAsInt())throw new Stop();return Block.stateById(neighbors.get(count%neighbors.size()).getAsInt());}
     if(method.getName().equals("getBlockEntity")){var at=(BlockPos)values[0];out.println("entity "+at.getX()+" "+at.getY()+" "+at.getZ());if(entities[0]++==f.get("entity_fail").getAsInt())throw new Stop();return selected;}
     if(method.isDefault())return InvocationHandler.invokeDefault(proxy,method,values);
     throw new AssertionError(method);
    });
    try{out.println("survive "+candidate.canSurvive(world,p));}catch(Stop error){out.println("survive error");}
    out.println("counts "+reads[0]+" "+entities[0]+" "+lights[0]+" "+fluids[0]);
   }
  }
 }
}
