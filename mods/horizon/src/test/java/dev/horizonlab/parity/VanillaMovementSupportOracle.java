package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.BlockPos;
import net.minecraft.tags.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.phys.Vec3;
/** Native support positions and movement factors with traced world/tag queries. */
public final class VanillaMovementSupportOracle {
    static final class Stop extends RuntimeException {}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static BlockPos position(JsonElement v){var a=v.getAsJsonArray();return new BlockPos(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt());}
    static final class Data {
        JsonObject input;PrintWriter out;int calls,reads;
        void query(String text){out.println(text);if(calls++==input.get("fail").getAsInt())throw new Stop();}
    }
    static class FactorBlock extends Block {
        Data d;int id;FactorBlock(){super(BlockBehaviour.Properties.of());}
        @Override public float getJumpFactor(){d.query("jump "+id);return single(d.input.getAsJsonArray("states").get(id).getAsJsonObject().get("jump"));}
        @Override public float getSpeedFactor(){d.query("speed "+id);return single(d.input.getAsJsonArray("states").get(id).getAsJsonObject().get("speed"));}
    }
    static class State extends BlockState {
        Data d;int id;FactorBlock block;State(){super(null,null,null);}
        boolean flag(String name){return d.input.getAsJsonArray("states").get(id).getAsJsonObject().get(name).getAsBoolean();}
        @Override public Block getBlock(){d.query("block "+id);return flag("gate")?Blocks.OAK_FENCE_GATE:block;}
        @Override public boolean is(TagKey<Block> tag){String name=tag==BlockTags.FENCES?"fence":tag==BlockTags.WALLS?"wall":null;if(name==null)throw new AssertionError(tag);d.query(name+" "+id);return flag(name);}
        @Override public boolean is(Block block){String name=block==Blocks.WATER?"water":block==Blocks.BUBBLE_COLUMN?"bubble":null;if(name==null)throw new AssertionError(block);d.query(name+" "+id);return flag(name);}
    }
    static final class Level extends net.minecraft.server.level.ServerLevel {
        Data d;State[] states;
        private Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public BlockState getBlockState(BlockPos p){d.query("state "+p.getX()+" "+p.getY()+" "+p.getZ());return states[d.reads++ % states.length];}
    }
    static final class Actor extends Marker {
        Actor(){super(null,null);}void configure(Level level){setLevel(level);}
        BlockPos cached;
        @Override public BlockPos blockPosition(){return cached;}
        BlockPos support(float offset){return getOnPos(offset);}
        float jump(){return getBlockJumpFactor();}float speed(){return getBlockSpeedFactor();}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);var unsafe=field.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        var support=Entity.class.getDeclaredField("mainSupportingBlockPos");support.setAccessible(true);var pos=Entity.class.getDeclaredField("position");pos.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.input=f;d.out=out;var level=(Level)allocate.invoke(unsafe,Level.class);level.d=d;level.states=new State[f.getAsJsonArray("states").size()];
                for(int i=0;i<level.states.length;i++){var s=(State)allocate.invoke(unsafe,State.class);s.d=d;s.id=i;var b=(FactorBlock)allocate.invoke(unsafe,FactorBlock.class);b.d=d;b.id=i;s.block=b;level.states[i]=s;}
                var entity=(Actor)allocate.invoke(unsafe,Actor.class);entity.configure(level);entity.cached=position(f.get("cached"));support.set(entity,f.get("support").isJsonNull()?Optional.empty():Optional.of(position(f.get("support"))));var v=f.getAsJsonArray("position");pos.set(entity,new Vec3(number(v.get(0)),number(v.get(1)),number(v.get(2))));
                try{int mode=f.get("mode").getAsInt();if(mode<4){BlockPos p=switch(mode){case 0->entity.support(single(f.get("offset")));case 1->entity.getOnPosLegacy();case 2->entity.getOnPos();case 3->entity.getBlockPosBelowThatAffectsMyMovement();default->throw new AssertionError();};out.printf("position %d %d %d%n",p.getX(),p.getY(),p.getZ());}else{float factor=mode==4?entity.jump():entity.speed();out.printf("factor %08x%n",Float.floatToIntBits(factor));}}
                catch(Stop error){out.println("error");}out.println("queries "+d.calls);
            }
        }
    }
}
