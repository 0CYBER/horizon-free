package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.nio.file.*;
import net.minecraft.core.BlockPos;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.entity.item.FallingBlockEntity;
import net.minecraft.tags.*;
import net.minecraft.world.item.*;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.CollisionGetter;
import net.minecraft.world.level.block.*;
import java.lang.reflect.Proxy;
import net.minecraft.world.level.material.*;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.*;
/** Executes native collision-context factories and queries over controlled snapshots. */
public final class VanillaCollisionContextOracle {
    static { net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap(); }
    private static final Item[] ITEMS={Items.AIR,Items.STONE,Items.SCAFFOLDING,Items.POWDER_SNOW_BUCKET,Items.LEATHER_BOOTS};
    private static final Fluid[] FLUIDS={Fluids.EMPTY,Fluids.FLOWING_WATER,Fluids.WATER,Fluids.FLOWING_LAVA,Fluids.LAVA};
    private static final class Stop extends RuntimeException {}
    private static final class Trace {
        PrintWriter out;int calls,fail;
        void query(String value){out.println(value);if(calls++==fail)throw new Stop();}
    }
    public static class PlayerSnapshot extends Player {
        Trace trace;VoxelShape liquidShape;boolean powderTag;ItemStack feet;
        @Override public boolean is(TagKey<EntityType<?>> tag){if(tag!=EntityTypeTags.POWDER_SNOW_WALKABLE_MOBS)throw new AssertionError(tag);trace.query("walkable_tag 1");return powderTag;}
        @Override public ItemStack getItemBySlot(EquipmentSlot slot){if(slot!=EquipmentSlot.FEET)throw new AssertionError(slot);trace.query("feet 1");return feet;}

        @Override public VoxelShape getLiquidCollisionShape(){trace.query("liquid_shape 1");return liquidShape;}

        boolean descending,stand,fail;
        ItemStack held;
        int calls,lastFluid;
        public PlayerSnapshot(){super(null,null);}
        @Override public GameType gameMode(){return GameType.SURVIVAL;}
        @Override public boolean isDescending(){return descending;}
        @Override public ItemStack getMainHandItem(){return held;}
        @Override public boolean canStandOnFluid(FluidState state){
            if(trace!=null)trace.query("stand 1 "+fluidIndex(state)+" "+state.getAmount()+" "+state.getValueOrElse(FlowingFluid.FALLING,false));
            calls++;lastFluid=-1;for(int i=0;i<FLUIDS.length;i++)if(state.getType()==FLUIDS[i])lastFluid=i;
            if(fail)throw new Stop();return stand;
        }
    }
    public static class MarkerSnapshot extends Marker {
        boolean descending,powderTag;Trace trace;
        @Override public boolean is(TagKey<EntityType<?>> tag){if(tag!=EntityTypeTags.POWDER_SNOW_WALKABLE_MOBS)throw new AssertionError(tag);trace.query("walkable_tag 2");return powderTag;}
        public MarkerSnapshot(){super(null,null);}
        @Override public boolean isDescending(){return descending;}
    }
    public static class FallingSnapshot extends FallingBlockEntity {
        boolean descending;
        public FallingSnapshot(){super(null,null);}
        @Override public boolean isDescending(){return descending;}
    }
    private static int fluidIndex(FluidState state){for(int i=0;i<FLUIDS.length;i++)if(FLUIDS[i]==state.getType())return i;throw new AssertionError();}
    private static FluidState liquidAbove(int index){return index<0?Fluids.EMPTY.defaultFluidState():(index<16?Blocks.WATER:Blocks.LAVA).defaultBlockState().setValue(LiquidBlock.LEVEL,index%16).getFluidState();}
    private static VoxelShape liquidShape(int index){return switch(index){case 0->Shapes.empty();case 1->Shapes.block();case 2->Shapes.box(0,0,0,1,0.5,1);case 3->Shapes.box(0.25,-0.5,0.25,0.75,1.5,0.75);default->throw new AssertionError();};}
    private static double number(JsonElement value){return Double.longBitsToDouble(Long.parseUnsignedLong(value.getAsString(),16));}
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var allocator=Class.forName("sun.misc.Unsafe");var access=allocator.getDeclaredField("theUnsafe");access.setAccessible(true);var unsafe=access.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        var position=Entity.class.getDeclaredField("position");position.setAccessible(true);
        var stackItem=ItemStack.class.getDeclaredField("item");stackItem.setAccessible(true);
        var stackCount=ItemStack.class.getDeclaredField("count");stackCount.setAccessible(true);
        Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var inputs=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)inputs::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();
                var player=(PlayerSnapshot)allocate.invoke(unsafe,PlayerSnapshot.class);
                var marker=(MarkerSnapshot)allocate.invoke(unsafe,MarkerSnapshot.class);
                var falling=(FallingSnapshot)allocate.invoke(unsafe,FallingSnapshot.class);
                player.descending=marker.descending=falling.descending=f.get("descending").getAsBoolean();
                // Query only item identity/count; component loading and inventory lifecycle are outside this fixture.
                player.held=(ItemStack)allocate.invoke(unsafe,ItemStack.class);
                stackItem.set(player.held,ITEMS[f.get("item").getAsInt()].builtInRegistryHolder());
                stackCount.setInt(player.held,f.get("count").getAsInt());
                player.stand=f.get("stand").getAsBoolean();player.fail=f.get("fail").getAsBoolean();
                double y=number(f.get("y")),other=number(f.get("other"));
                position.set(player,new Vec3(0,y,0));position.set(marker,new Vec3(0,y,0));position.set(falling,new Vec3(0,y,0));
                boolean always=f.get("always").getAsBoolean();
                var context=switch(f.get("mode").getAsString()){
                    case "empty"->always?CollisionContext.emptyWithFluidCollisions():CollisionContext.empty();
                    case "position"->CollisionContext.positionContext(y);
                    case "player"->CollisionContext.of(player,always);
                    case "marker"->CollisionContext.of(marker,always);
                    case "falling"->CollisionContext.of(falling,always);
                    case "with_falling"->CollisionContext.withPosition(falling,other);
                    case "factory_player"->CollisionContext.of(player);
                    case "factory_marker"->CollisionContext.of(marker);
                    case "placement"->CollisionContext.placementContext(player);
                    case "placement_null"->CollisionContext.placementContext(null);
                    case "with_player"->CollisionContext.withPosition(player,other);
                    case "with_marker"->CollisionContext.withPosition(marker,other);
                    case "with_null"->CollisionContext.withPosition(null,other);
                    default->throw new AssertionError();
                };
                int entity=context instanceof EntityCollisionContext ec?(ec.getEntity()==player?1:ec.getEntity()==marker?2:ec.getEntity()==falling?3:0):0;
                int held=0;for(int i=0;i<ITEMS.length;i++)if(context.isHoldingItem(ITEMS[i]))held|=1<<i;
                out.printf("context %b %b %b %d %d%n",context.isDescending(),context.isPlacement(),context.alwaysCollideWithFluid(),entity,held);
                if(f.has("registry")) {
                    var config=f.getAsJsonArray("registry");var state=Block.stateById(config.get(0).getAsInt());
                    var pos=new BlockPos(config.get(1).getAsInt(),config.get(2).getAsInt(),config.get(3).getAsInt());
                    var trace=new Trace();trace.out=out;trace.fail=-1;player.trace=marker.trace=trace;player.liquidShape=liquidShape(2);
                    player.fallDistance=marker.fallDistance=falling.fallDistance=always?3.0:0.0;player.powderTag=marker.powderTag=false;
                    player.feet=(ItemStack)allocate.invoke(unsafe,ItemStack.class);stackItem.set(player.feet,Items.LEATHER_BOOTS.builtInRegistryHolder());stackCount.setInt(player.feet,1);
                    var world=(CollisionGetter)Proxy.newProxyInstance(CollisionGetter.class.getClassLoader(),new Class<?>[]{CollisionGetter.class},(proxy,method,values)->{
                        var at=(BlockPos)values[0];
                        if(method.getName().equals("getFluidState")){trace.query("fluid_at "+at.getX()+" "+at.getY()+" "+at.getZ());return Fluids.EMPTY.defaultFluidState();}
                        if(method.getName().equals("getBlockEntity")){trace.query("block_entity "+at.getX()+" "+at.getY()+" "+at.getZ());return null;}
                        throw new AssertionError(method);
                    });
                    var shape=context.getCollisionShape(state,world,pos);
                    var cacheField=net.minecraft.world.level.block.state.BlockBehaviour.BlockStateBase.class.getDeclaredField("cache");cacheField.setAccessible(true);var cache=cacheField.get(state);
                    if(cache!=null){var shapeField=cache.getClass().getDeclaredField("collisionShape");shapeField.setAccessible(true);out.println("cache_equal "+!Shapes.joinIsNotEmpty(shape,(VoxelShape)shapeField.get(cache),BooleanOp.NOT_SAME));}
                    else{var emit=VanillaFaceSupportOracle.class.getDeclaredMethod("emit",PrintWriter.class,VoxelShape.class);emit.setAccessible(true);emit.invoke(null,out,shape);}
                    out.println("queries "+trace.calls);
                    continue;
                }
                if(f.has("powder")) {
                    var config=f.getAsJsonArray("powder");
                    var pos=new BlockPos(17,config.get(0).getAsInt(),-33);
                    var trace=new Trace();trace.out=out;trace.fail=config.get(5).getAsInt();player.trace=marker.trace=trace;
                    player.fallDistance=marker.fallDistance=falling.fallDistance=number(config.get(1));
                    player.powderTag=marker.powderTag=config.get(2).getAsBoolean();
                    player.feet=(ItemStack)allocate.invoke(unsafe,ItemStack.class);stackItem.set(player.feet,ITEMS[config.get(3).getAsInt()].builtInRegistryHolder());stackCount.setInt(player.feet,config.get(4).getAsInt());
                    var world=(CollisionGetter)Proxy.newProxyInstance(CollisionGetter.class.getClassLoader(),new Class<?>[]{CollisionGetter.class},(proxy,method,values)->{throw new AssertionError("unexpected powder world query: "+method);});
                    var emit=VanillaFaceSupportOracle.class.getDeclaredMethod("emit",PrintWriter.class,VoxelShape.class);emit.setAccessible(true);
                    try{emit.invoke(null,out,context.getCollisionShape(Blocks.POWDER_SNOW.defaultBlockState(),world,pos));}catch(Stop error){out.println("shape error");}
                    out.println("queries "+trace.calls);
                    continue;
                }
                if(f.has("liquid")) {
                    var config=f.getAsJsonArray("liquid");
                    var state=(config.get(1).getAsBoolean()?Blocks.LAVA:Blocks.WATER).defaultBlockState().setValue(LiquidBlock.LEVEL,config.get(0).getAsInt());
                    var pos=new BlockPos(17,config.get(2).getAsInt(),-33);
                    var trace=new Trace();trace.out=out;trace.fail=config.get(5).getAsInt();player.trace=trace;player.liquidShape=liquidShape(config.get(4).getAsInt());
                    var world=(CollisionGetter)Proxy.newProxyInstance(CollisionGetter.class.getClassLoader(),new Class<?>[]{CollisionGetter.class},(proxy,method,values)->{
                        if(!method.getName().equals("getFluidState"))throw new AssertionError(method);
                        var at=(BlockPos)values[0];trace.query("fluid_at "+at.getX()+" "+at.getY()+" "+at.getZ());return liquidAbove(config.get(3).getAsInt());
                    });
                    var emit=VanillaFaceSupportOracle.class.getDeclaredMethod("emit",PrintWriter.class,VoxelShape.class);emit.setAccessible(true);
                    try{emit.invoke(null,out,context.getCollisionShape(state,world,pos));}catch(Stop error){out.println("shape error");}
                    out.println("queries "+trace.calls);
                    continue;
                }
                if(f.has("scaffolding")) {
                    var config=f.getAsJsonArray("scaffolding");
                    var state=Blocks.SCAFFOLDING.defaultBlockState().setValue(ScaffoldingBlock.DISTANCE,config.get(0).getAsInt()).setValue(ScaffoldingBlock.BOTTOM,config.get(1).getAsBoolean()).setValue(ScaffoldingBlock.WATERLOGGED,config.get(2).getAsBoolean());
                    var pos=new BlockPos(17,config.get(3).getAsInt(),-33);
                    var world=(CollisionGetter)Proxy.newProxyInstance(CollisionGetter.class.getClassLoader(),new Class<?>[]{CollisionGetter.class},(proxy,method,values)->{throw new AssertionError("unexpected world query: "+method);});
                    var emit=VanillaFaceSupportOracle.class.getDeclaredMethod("emit",PrintWriter.class,VoxelShape.class);emit.setAccessible(true);
                    emit.invoke(null,out,context.getCollisionShape(state,world,pos));
                    continue;
                }
                var above=new StringBuilder();
                for(var query:f.getAsJsonArray("above")){
                    var q=query.getAsJsonArray();int blockY=q.get(0).getAsInt();double top=number(q.get(1));
                    var shape=q.get(2).getAsBoolean()?Shapes.empty():Shapes.box(0,top-0.25,0,1,top,1);
                    for(boolean fallback:new boolean[]{false,true})above.append(context.isAbove(shape,new BlockPos(0,blockY,0),fallback)?'1':'0');
                }
                out.println("above "+above);
                for(int a=0;a<FLUIDS.length;a++)for(int b=0;b<FLUIDS.length;b++){
                    player.calls=0;player.lastFluid=-1;String result;
                    try{result=Boolean.toString(context.canStandOnFluid(FLUIDS[a].defaultFluidState(),FLUIDS[b].defaultFluidState()));}catch(Stop error){result="error";}
                    out.printf("fluid %s %d %d%n",result,player.calls,player.lastFluid);
                }
            }
        }
    }
}
