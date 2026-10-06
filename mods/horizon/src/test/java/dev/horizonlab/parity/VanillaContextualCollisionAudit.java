package dev.horizonlab.parity;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import net.minecraft.core.BlockPos;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.item.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.phys.shapes.*;
/** Verifies that the pinned cached families used by the contextual dispatcher
 * return the exact cache object without reading world or context providers. */
public final class VanillaContextualCollisionAudit {
    static final class Query extends RuntimeException { Query(String name){super(name);} }
    static class RejectContext implements CollisionContext {
        public boolean isDescending(){throw new Query("descending");}
        public boolean isAbove(VoxelShape s,BlockPos p,boolean d){throw new Query("above");}
        public boolean isHoldingItem(Item i){throw new Query("item");}
        public boolean alwaysCollideWithFluid(){throw new Query("fluid");}
        public boolean canStandOnFluid(FluidState a,FluidState b){throw new Query("stand");}
        public boolean isPlacement(){throw new Query("placement");}
        public VoxelShape getCollisionShape(BlockState s,CollisionGetter w,BlockPos p){throw new Query("shape");}
    }
    static class RejectEntityContext extends EntityCollisionContext {
        RejectEntityContext(){super(false,false,0.0,ItemStack.EMPTY,false,null);}
        public boolean isDescending(){throw new Query("descending");}
        public boolean isAbove(VoxelShape s,BlockPos p,boolean d){throw new Query("above");}
        public boolean isHoldingItem(Item i){throw new Query("item");}
        public boolean alwaysCollideWithFluid(){throw new Query("fluid");}
        public boolean canStandOnFluid(FluidState a,FluidState b){throw new Query("stand");}
        public boolean isPlacement(){throw new Query("placement");}
        public Entity getEntity(){throw new Query("entity");}
        public VoxelShape getCollisionShape(BlockState s,CollisionGetter w,BlockPos p){throw new Query("shape");}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var cache=BlockBehaviour.BlockStateBase.class.getDeclaredField("cache");cache.setAccessible(true);
        var collision=cache.getType().getDeclaredField("collisionShape");collision.setAccessible(true);
        var world=(BlockGetter)Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},(p,m,v)->{throw new AssertionError("unexpected world read: "+m);});
        var contexts=new CollisionContext[]{new RejectContext(),new RejectEntityContext()};
        var positions=new BlockPos[]{BlockPos.ZERO,new BlockPos(17,-64,-33),new BlockPos(-17,320,33),new BlockPos(30000000,63,-30000000),new BlockPos(Integer.MIN_VALUE,Integer.MIN_VALUE,Integer.MIN_VALUE),new BlockPos(Integer.MAX_VALUE,Integer.MAX_VALUE,Integer.MAX_VALUE),new BlockPos(-1,-1,-1)};
        int independent=0,contextual=0,uncached=0;
        try(var out=new PrintWriter(Files.newBufferedWriter(Path.of(args[1]).resolve(args[0]+"-audit.txt")))) {
            for(var state:Block.BLOCK_STATE_REGISTRY) {
                var c=cache.get(state);String label="uncached";
                if(c==null)uncached++;
                else {
                    String expected=state.getBlock() instanceof LiquidBlock?"fluid":"independent";
                    for(var p:positions)for(var context:contexts){
                        String result;
                        try{var shape=state.getCollisionShape(world,p,context);if(shape!=collision.get(c))throw new AssertionError("different shape identity: "+state);result="independent";}
                        catch(Query query){result=query.getMessage();}
                        if(!result.equals(expected))throw new AssertionError(state+" unexpected query "+result);
                    }
                    label=expected;if(label.equals("independent"))independent++;else contextual++;
                }
                out.println("audit "+Block.getId(state)+" "+label+" "+state.hasLargeCollisionShape()+" "+state.is(Blocks.MOVING_PISTON));
            }
        }
        if(independent!=35492||contextual!=32||uncached!=199)throw new AssertionError("registry classification changed");
    }
}
