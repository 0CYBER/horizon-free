package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.tags.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.level.gamerules.*;
import net.minecraft.world.level.material.*;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;
/** Actual BlockGetter.clip with native context modes and controlled shape providers. */
public final class VanillaWorldRayOracle {
    static final class Stop extends RuntimeException {}
    static Object unsafe;static Method allocate;
    static <T>T make(Class<T> type){try{return type.cast(allocate.invoke(unsafe,type));}catch(Exception e){throw new AssertionError(e);}}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static Vec3 vector(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static String pos(BlockPos p){return p.getX()+" "+p.getY()+" "+p.getZ();}
    static VoxelShape shape(JsonElement v){var result=Shapes.empty();for(var value:v.getAsJsonArray()){var a=value.getAsJsonArray();result=Shapes.or(result,Shapes.box(number(a.get(0)),number(a.get(1)),number(a.get(2)),number(a.get(3)),number(a.get(4)),number(a.get(5))));}return result;}
    static final class Data {
        JsonObject f;PrintWriter out;int calls;State[] states;FluidState[] fluids;Rules rules;
        void query(String s){out.println(s);if(calls++==f.get("fail").getAsInt())throw new Stop();}
        int index(BlockPos p){var cells=f.getAsJsonObject("cells");var id=cells.get(pos(p));return id==null?0:id.getAsInt();}
        BlockState block(BlockPos p){query("block "+pos(p));return states[index(p)];}
        FluidState fluid(BlockPos p){query("fluid "+pos(p));return fluids[index(p)];}
    }
    static final class State extends BlockState {
        Data d;int id;VoxelShape[] shapes;State(){super(null,null,null);}
        boolean flag(String n){return d.f.getAsJsonArray("states").get(id).getAsJsonObject().get(n).getAsBoolean();}
        @Override public boolean is(TagKey<Block> tag){if(tag!=BlockTags.FALL_DAMAGE_RESETTING)throw new AssertionError();d.query("reset-tag "+id);return flag("reset");}
        @Override public boolean is(Block block){String n=block==Blocks.END_GATEWAY?"gateway":block==Blocks.END_PORTAL?"end":block==Blocks.NETHER_PORTAL?"nether":null;if(n==null)throw new AssertionError();d.query(n+" "+id);return flag(n);}
        @Override public VoxelShape getCollisionShape(BlockGetter w,BlockPos p,CollisionContext c){d.query("collider "+id+" "+pos(p));return shapes[0];}
        @Override public VoxelShape getShape(BlockGetter w,BlockPos p,CollisionContext c){d.query("outline "+id+" "+pos(p));return shapes[1];}
        @Override public VoxelShape getVisualShape(BlockGetter w,BlockPos p,CollisionContext c){d.query("visual "+id+" "+pos(p));return shapes[2];}
        @Override public VoxelShape getInteractionShape(BlockGetter w,BlockPos p){d.query("interaction "+id+" "+pos(p));return shapes[3];}
    }
    static final class Fluid extends WaterFluid.Source {
        Data d;int id;VoxelShape shape;
        boolean flag(String n){return d.f.getAsJsonArray("states").get(id).getAsJsonObject().get(n).getAsBoolean();}
        @Override public boolean isSource(FluidState state){d.query("source "+id);return flag("source");}
        @Override protected boolean isEmpty(){d.query("empty "+id);return flag("empty");}
        @Override public Holder.Reference<net.minecraft.world.level.material.Fluid> builtInRegistryHolder(){d.query("water "+id);return (flag("water")?Fluids.WATER:Fluids.LAVA).builtInRegistryHolder();}
        @Override public VoxelShape getShape(FluidState state,BlockGetter w,BlockPos p){d.query("fluid-shape "+id+" "+pos(p));return shape;}
    }
    static final class Rules extends GameRules {
        Data d;Rules(){super(net.minecraft.world.flag.FeatureFlagSet.of());}
        @SuppressWarnings("unchecked") @Override public <T>T get(GameRule<T> rule){if(rule!=GameRules.PLAYERS_NETHER_PORTAL_DEFAULT_DELAY)throw new AssertionError();d.query("portal-delay");return (T)Integer.valueOf(d.f.get("delay").getAsInt());}
    }
    static final class ServerWorld extends net.minecraft.server.level.ServerLevel {
        Data d;ServerWorld(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public BlockState getBlockState(BlockPos p){return d.block(p);}
        @Override public FluidState getFluidState(BlockPos p){return d.fluid(p);}
        @Override public GameRules getGameRules(){d.query("rules");return d.rules;}
    }
    static final class Actor extends Marker {
        Data d;Actor(){super(null,null);}
        @Override public boolean is(EntityType<?> type){if(type!=EntityTypes.PLAYER)throw new AssertionError();d.query("player");return d.f.get("player").getAsBoolean();}
        @Override public boolean isDescending(){return false;}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);unsafe=field.get(null);allocate=allocator.getMethod("allocateInstance",Class.class);
        BuiltInRegistries.FLUID.prepareTagReload(new TagLoader.LoadResult<>(Registries.FLUID,Map.of(FluidTags.WATER,List.of(Fluids.WATER.builtInRegistryHolder())))).apply();var position=Entity.class.getDeclaredField("position");position.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){
            var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;var entries=f.getAsJsonArray("states");d.states=new State[entries.size()];d.fluids=new FluidState[entries.size()];for(int i=0;i<entries.size();i++){var data=entries.get(i).getAsJsonObject();var s=make(State.class);s.d=d;s.id=i;s.shapes=new VoxelShape[]{shape(data.get("collider")),shape(data.get("outline")),shape(data.get("visual")),shape(data.get("interaction"))};d.states[i]=s;var fluid=make(Fluid.class);fluid.d=d;fluid.id=i;fluid.shape=shape(data.get("fluid_shape"));d.fluids[i]=new FluidState(fluid,new net.minecraft.world.level.block.state.properties.Property<?>[0],new Comparable<?>[0]);}
            d.rules=make(Rules.class);d.rules.d=d;BlockGetter world;if(f.get("server").getAsBoolean()){var w=make(ServerWorld.class);w.d=d;world=w;}else{world=(BlockGetter)Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},(proxy,method,values)->{if(method.getName().equals("getBlockState"))return d.block((BlockPos)values[0]);if(method.getName().equals("getFluidState"))return d.fluid((BlockPos)values[0]);if(method.isDefault())return InvocationHandler.invokeDefault(proxy,method,values);throw new AssertionError(method);});}
            CollisionContext context=CollisionContext.empty();if(f.get("entity").getAsBoolean()){var actor=make(Actor.class);actor.d=d;position.set(actor,Vec3.ZERO);context=CollisionContext.of(actor);}
            var c=new ClipContext(vector(f.get("from")),vector(f.get("to")),ClipContext.Block.values()[f.get("block_mode").getAsInt()],ClipContext.Fluid.values()[f.get("fluid_mode").getAsInt()],context);
            try{var hit=world.clip(c);var v=hit.getLocation();out.printf("hit %s %016x %016x %016x %s %s %s%n",hit.getType()==HitResult.Type.MISS,Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z),hit.getDirection().getSerializedName(),pos(hit.getBlockPos()),hit.isInside());}catch(Stop e){out.println("error");}out.println("queries "+d.calls);
        }}
    }
}
