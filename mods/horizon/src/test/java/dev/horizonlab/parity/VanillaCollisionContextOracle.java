package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.nio.file.*;
import net.minecraft.core.BlockPos;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.*;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.material.*;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.*;
/** Executes native collision-context factories and queries over controlled snapshots. */
public final class VanillaCollisionContextOracle {
    static { net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap(); }
    private static final Item[] ITEMS={Items.AIR,Items.STONE,Items.SCAFFOLDING,Items.POWDER_SNOW_BUCKET,Items.LEATHER_BOOTS};
    private static final Fluid[] FLUIDS={Fluids.EMPTY,Fluids.FLOWING_WATER,Fluids.WATER,Fluids.FLOWING_LAVA,Fluids.LAVA};
    private static final class Stop extends RuntimeException {}
    public static class PlayerSnapshot extends Player {
        boolean descending,stand,fail;
        ItemStack held;
        int calls,lastFluid;
        public PlayerSnapshot(){super(null,null);}
        @Override public GameType gameMode(){return GameType.SURVIVAL;}
        @Override public boolean isDescending(){return descending;}
        @Override public ItemStack getMainHandItem(){return held;}
        @Override public boolean canStandOnFluid(FluidState state){
            calls++;lastFluid=-1;for(int i=0;i<FLUIDS.length;i++)if(state.getType()==FLUIDS[i])lastFluid=i;
            if(fail)throw new Stop();return stand;
        }
    }
    public static class MarkerSnapshot extends Marker {
        boolean descending;
        public MarkerSnapshot(){super(null,null);}
        @Override public boolean isDescending(){return descending;}
    }
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
                player.descending=marker.descending=f.get("descending").getAsBoolean();
                // Query only item identity/count; component loading and inventory lifecycle are outside this fixture.
                player.held=(ItemStack)allocate.invoke(unsafe,ItemStack.class);
                stackItem.set(player.held,ITEMS[f.get("item").getAsInt()].builtInRegistryHolder());
                stackCount.setInt(player.held,f.get("count").getAsInt());
                player.stand=f.get("stand").getAsBoolean();player.fail=f.get("fail").getAsBoolean();
                double y=number(f.get("y")),other=number(f.get("other"));
                position.set(player,new Vec3(0,y,0));position.set(marker,new Vec3(0,y,0));
                boolean always=f.get("always").getAsBoolean();
                var context=switch(f.get("mode").getAsString()){
                    case "empty"->always?CollisionContext.emptyWithFluidCollisions():CollisionContext.empty();
                    case "position"->CollisionContext.positionContext(y);
                    case "player"->CollisionContext.of(player,always);
                    case "marker"->CollisionContext.of(marker,always);
                    case "factory_player"->CollisionContext.of(player);
                    case "factory_marker"->CollisionContext.of(marker);
                    case "placement"->CollisionContext.placementContext(player);
                    case "placement_null"->CollisionContext.placementContext(null);
                    case "with_player"->CollisionContext.withPosition(player,other);
                    case "with_marker"->CollisionContext.withPosition(marker,other);
                    case "with_null"->CollisionContext.withPosition(null,other);
                    default->throw new AssertionError();
                };
                int entity=context instanceof EntityCollisionContext ec?(ec.getEntity()==player?1:ec.getEntity()==marker?2:0):0;
                int held=0;for(int i=0;i<ITEMS.length;i++)if(context.isHoldingItem(ITEMS[i]))held|=1<<i;
                out.printf("context %b %b %b %d %d%n",context.isDescending(),context.isPlacement(),context.alwaysCollideWithFluid(),entity,held);
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
