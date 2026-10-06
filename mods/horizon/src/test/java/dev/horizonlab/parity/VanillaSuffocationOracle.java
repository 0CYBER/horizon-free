package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.tags.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.entity.*;
import net.minecraft.world.level.block.piston.PistonMovingBlockEntity;
import net.minecraft.world.level.block.state.*;
/** Native suffocation predicates, tag rebinding, cursor flags and traced block entities. */
public final class VanillaSuffocationOracle {
    static final class Stop extends RuntimeException {}
    static Object field(Class<?> type,String name,Object instance)throws Exception{var f=type.getDeclaredField(name);f.setAccessible(true);return f.get(instance);}
    static void definitions(Path dir)throws Exception {
        var defaults=field(BlockBehaviour.Properties.class,"isSuffocating",BlockBehaviour.Properties.of());
        var piston=field(Blocks.class,"NOT_EXTENDED_PISTON",null);var shulker=field(Blocks.class,"NOT_CLOSED_SHULKER",null);
        var predicate=BlockBehaviour.BlockStateBase.class.getDeclaredField("isSuffocating");predicate.setAccessible(true);
        var rejecting=(BlockGetter)Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},(p,m,v)->{throw new AssertionError("unexpected constant predicate query: "+m);});
        var result=new TreeMap<String,String>();
        for(var state:Block.BLOCK_STATE_REGISTRY){
            var selected=predicate.get(state);String rule;
            if(selected==defaults)rule="tagged_full";else if(selected==piston)rule="not_extended";else if(selected==shulker)rule="closed_shulker";
            else {
                Boolean constant=null;
                for(var p:new BlockPos[]{BlockPos.ZERO,new BlockPos(17,-64,-33),new BlockPos(Integer.MIN_VALUE,Integer.MAX_VALUE,-1)}){
                    boolean value=state.isSuffocating(rejecting,p);if(constant!=null&&value!=constant)throw new AssertionError("position-dependent suffocation override");constant=value;
                }
                rule=constant?"always":"never";
            }
            String name=BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString();var previous=result.put(name,rule);if(previous!=null&&!previous.equals(rule))throw new AssertionError("unclassified state-dependent predicate: "+name);
        }
        Files.writeString(dir.resolve("definitions.json"),new Gson().toJson(result));
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());
        if(args[0].equals("export")){
            VanillaCollisionOracle.main(new String[]{"generate",args[1]});VanillaBlockStateCodecOracle.main(new String[]{"export",dir.resolve("facts.json").toString()});definitions(dir);return;
        }
        var registry=BuiltInRegistries.BLOCK;var natural=new ArrayList<Holder<Block>>();var all=new ArrayList<Holder<Block>>();
        for(var block:registry){var holder=block.builtInRegistryHolder();all.add(holder);if(block.defaultBlockState().is(BlockTags.CAUSES_SUFFOCATION))natural.add(holder);}
        var allocator=Class.forName("sun.misc.Unsafe");var access=allocator.getDeclaredField("theUnsafe");access.setAccessible(true);var unsafe=access.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        var animation=ShulkerBoxBlockEntity.class.getDeclaredField("animationStatus");animation.setAccessible(true);int activeTag=-1;
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();int tag=f.get("tags").getAsInt();
                if(activeTag!=tag){List<Holder<Block>> members=tag==0?natural:tag==1?List.of():all;registry.prepareTagReload(new TagLoader.LoadResult<>(Registries.BLOCK,Map.of(BlockTags.CAUSES_SUFFOCATION,members))).apply();activeTag=tag;}
                var state=Block.stateById(f.get("state").getAsInt());var xyz=f.getAsJsonArray("position");var p=new BlockPos(xyz.get(0).getAsInt(),xyz.get(1).getAsInt(),xyz.get(2).getAsInt());String mode=f.get("entity").getAsString();Object entity=null;
                if(mode.equals("other"))entity=allocate.invoke(unsafe,PistonMovingBlockEntity.class);
                else if(!mode.equals("none")){
                    entity=allocate.invoke(unsafe,ShulkerBoxBlockEntity.class);Object status=null;for(var value:animation.getType().getEnumConstants())if(((Enum<?>)value).name().equals(mode.toUpperCase(Locale.ROOT)))status=value;if(status==null)throw new AssertionError(mode);animation.set(entity,status);
                }
                Object selected=entity;int[] calls={0};
                var world=(BlockGetter)Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},(proxy,method,values)->{
                    if(!method.getName().equals("getBlockEntity"))throw new AssertionError(method);var at=(BlockPos)values[0];out.println("entity "+at.getX()+" "+at.getY()+" "+at.getZ());if(calls[0]++==f.get("fail").getAsInt())throw new Stop();return selected;
                });
                out.println("metadata "+Block.getId(state)+" "+state.hasLargeCollisionShape()+" "+state.is(Blocks.MOVING_PISTON));
                try{out.println("suffocating "+state.isSuffocating(world,p));}catch(Stop error){out.println("suffocating error");}
                out.println("queries "+calls[0]);
            }
        }
    }
}
