package dev.horizonlab.parity;
import com.google.gson.*;import com.mojang.serialization.JsonOps;
import java.io.*;import java.lang.reflect.*;import java.nio.file.*;
import net.minecraft.core.*;import net.minecraft.core.registries.*;import net.minecraft.tags.*;import net.minecraft.world.level.*;import net.minecraft.world.level.block.*;import net.minecraft.world.level.block.state.*;import net.minecraft.world.level.material.*;import net.minecraft.world.phys.*;
/** Native registered fluid current calculation with independently controlled block/fluid neighbors. */
public final class VanillaFluidFlowOracle {
    static final class Stop extends RuntimeException {}
    static String pos(BlockPos p){return p.getX()+" "+p.getY()+" "+p.getZ();}
    static final class Data {JsonObject f;PrintWriter out;int calls;State[] blocks;FluidState[] fluids;
        void query(String s){out.println(s);if(calls++==f.get("fail").getAsInt())throw new Stop();}
        int cell(BlockPos p){var v=f.getAsJsonObject("cells").get(pos(p));return v==null?f.get("default").getAsInt():v.getAsInt();}
    }
    static final class State extends BlockState {Data d;int id;
        State(Block block){super(block,new net.minecraft.world.level.block.state.properties.Property<?>[0],new Comparable<?>[0]);}
        @Override public boolean is(TagKey<Block> tag){if(tag!=BlockTags.BLOCKS_FLUID_FLOW)throw new AssertionError(tag);return d.f.getAsJsonArray("states").get(id).getAsJsonObject().get("blocks_flow").getAsBoolean();}
        @Override public boolean isFaceSturdy(BlockGetter w,BlockPos p,Direction face){d.query("sturdy "+pos(p)+" "+face.getSerializedName());return (d.f.getAsJsonArray("states").get(id).getAsJsonObject().get("faces").getAsInt()&(1<<face.ordinal()))!=0;}
    }
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
        if(args[0].equals("export")){var states=new JsonArray();for(var fluid:BuiltInRegistries.FLUID)for(var state:fluid.getStateDefinition().getPossibleStates())states.add(FluidState.CODEC.encodeStart(JsonOps.INSTANCE,state).getOrThrow());Files.writeString(dir.resolve("fluid-states.json"),states.toString());return;}
        var solid=FlowingFluid.class.getDeclaredMethod("isSolidFace",BlockGetter.class,BlockPos.class,Direction.class);solid.setAccessible(true);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;var states=f.getAsJsonArray("states");d.blocks=new State[states.size()];d.fluids=new FluidState[states.size()];for(int i=0;i<states.size();i++){var s=states.get(i).getAsJsonObject();var b=new State(s.get("ice").getAsBoolean()?Blocks.ICE:Blocks.STONE);b.d=d;b.id=i;d.blocks[i]=b;d.fluids[i]=FluidState.CODEC.parse(JsonOps.INSTANCE,s.get("fluid")).getOrThrow();}
            var world=(BlockGetter)Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},(proxy,method,values)->{var p=(BlockPos)values[0];switch(method.getName()){case "getBlockState":d.query("block "+pos(p));return d.blocks[d.cell(p)];case "getFluidState":d.query("fluid "+pos(p));return d.fluids[d.cell(p)];default:throw new AssertionError(method);}});
            var a=f.getAsJsonArray("position");var p=new BlockPos(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt());var root=FluidState.CODEC.parse(JsonOps.INSTANCE,f.get("root")).getOrThrow();
            try{if(f.get("mode").getAsString().equals("flow")){var v=root.getFlow(world,p);out.printf("flow %016x %016x %016x%n",Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z));}else out.println("face "+solid.invoke(root.getType(),world,p,Direction.values()[f.get("direction").getAsInt()]));}catch(Stop e){out.println("error");}catch(InvocationTargetException e){if(e.getCause() instanceof Stop)out.println("error");else throw e;}
            out.println("queries "+d.calls);
        }}
    }
}
