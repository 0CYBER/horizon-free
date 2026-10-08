package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.registries.*;import net.minecraft.tags.*;import net.minecraft.world.level.*;import net.minecraft.world.level.block.*;import net.minecraft.world.level.material.*;
/** Registered fluid contents and cached/dynamic support dispatch in actual height/flow calls. */
public final class VanillaRegistryFluidOracle {
    static final class Stop extends RuntimeException {}
    static String pos(BlockPos p){return p.getX()+" "+p.getY()+" "+p.getZ();}
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());
        var natural=new ArrayList<Holder<Block>>();var all=new ArrayList<Holder<Block>>();var names=new JsonArray();var ice=new JsonArray();for(var block:BuiltInRegistries.BLOCK){String name=BuiltInRegistries.BLOCK.getKey(block).toString();all.add(block.builtInRegistryHolder());if(block.defaultBlockState().is(BlockTags.BLOCKS_FLUID_FLOW)){names.add(name);natural.add(block.builtInRegistryHolder());}if(block instanceof IceBlock)ice.add(name);}
        if(args[0].equals("export")){VanillaBlockSupportOracle.main(args);VanillaBlockStateCodecOracle.main(new String[]{"export",dir.resolve("facts.json").toString()});var info=new JsonObject();info.add("blocks_flow",names);info.add("ice",ice);Files.writeString(dir.resolve("fluid-info.json"),info.toString());return;}
        var solid=FlowingFluid.class.getDeclaredMethod("isSolidFace",BlockGetter.class,BlockPos.class,Direction.class);solid.setAccessible(true);int oldTag=-1;
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();int tag=f.get("tag").getAsInt();if(tag!=oldTag){BuiltInRegistries.BLOCK.prepareTagReload(new TagLoader.LoadResult<>(Registries.BLOCK,Map.of(BlockTags.BLOCKS_FLUID_FLOW,tag==0?natural:tag==1?List.<Holder<Block>>of():all))).apply();oldTag=tag;}
            int[] calls={0};var world=(BlockGetter)Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},(proxy,method,values)->{var p=(BlockPos)values[0];String kind=switch(method.getName()){case "getBlockState"->"block";case "getFluidState"->"fluid";case "getBlockEntity"->"entity";default->throw new AssertionError(method);};out.println(kind+" "+pos(p));if(calls[0]++==f.get("fail").getAsInt())throw new Stop();if(kind.equals("entity"))return null;var v=f.getAsJsonObject("cells").get(pos(p));var state=Block.stateById(v==null?f.get("default").getAsInt():v.getAsInt());return kind.equals("block")?state:state.getFluidState();});
            var a=f.getAsJsonArray("position");var p=new BlockPos(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt());var root=Block.stateById(f.get("root").getAsInt()).getFluidState();
            try{switch(f.get("mode").getAsString()){case "flow":{var v=root.getFlow(world,p);out.printf("flow %016x %016x %016x%n",Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z));break;}case "face":out.println("face "+solid.invoke(root.getType(),world,p,Direction.values()[f.get("direction").getAsInt()]));break;case "height":out.printf("height %08x%n",Float.floatToIntBits(root.getHeight(world,p)));break;case "camera":out.printf("height %08x%n",Float.floatToIntBits(root.getHeightForCamera(world,p)));break;default:throw new AssertionError();}}catch(Stop e){out.println("error");}catch(InvocationTargetException e){if(e.getCause() instanceof Stop)out.println("error");else throw e;}
            out.println("queries "+calls[0]);
        }}
    }
}
