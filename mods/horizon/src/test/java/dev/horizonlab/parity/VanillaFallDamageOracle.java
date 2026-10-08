package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.level.gameevent.*;
import net.minecraft.world.phys.Vec3;
/** Actual base Entity checkFallDamage with traced landing callbacks and mutations. */
public final class VanillaFallDamageOracle {
    static final class Stop extends RuntimeException {}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static Optional<BlockPos> support(JsonElement value){if(value.isJsonNull())return Optional.empty();var a=value.getAsJsonArray();return Optional.of(new BlockPos(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt()));}
    static Field positionField,supportField;
    static final class Data {
        JsonObject input;PrintWriter out;int calls;Actor actor;
        void query(String text){out.println(text);if(calls++==input.get("fail").getAsInt())throw new Stop();}
        void mutate() {if(input.get("mutate").getAsBoolean()){try{actor.fallDistance=17.25;positionField.set(actor,new Vec3(17,18,19));supportField.set(actor,support(input.get("after_support")));}catch(ReflectiveOperationException e){throw new AssertionError(e);}}}
    }
    static final class FallBlock extends Block {
        Data d;FallBlock(){super(BlockBehaviour.Properties.of());}
        @Override public void fallOn(net.minecraft.world.level.Level level,BlockState state,BlockPos p,Entity entity,double distance){d.query(String.format("fall-on %d %d %d %016x",p.getX(),p.getY(),p.getZ(),Double.doubleToLongBits(distance)));d.mutate();}
    }
    static final class State extends BlockState {Block block;State(){super(null,null,null);}@Override public Block getBlock(){return block;}}
    static final class Level extends net.minecraft.server.level.ServerLevel {
        Data d;BlockState queried;
        private Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public BlockState getBlockState(BlockPos p){d.query("state "+p.getX()+" "+p.getY()+" "+p.getZ());return queried;}
        @Override public void gameEvent(Holder<GameEvent> event,Vec3 p,GameEvent.Context context){if(event!=GameEvent.HIT_GROUND)throw new AssertionError(event);d.query(String.format("hit-ground %016x %016x %016x %s",Double.doubleToLongBits(p.x),Double.doubleToLongBits(p.y),Double.doubleToLongBits(p.z),context.affectedState()==queried?"queried":"input"));}
    }
    static final class Actor extends Marker {
        Data d;Actor(){super(null,null);}void configure(Level level){setLevel(level);}
        @Override public boolean isInWater(){d.query("water");return d.input.get("water").getAsBoolean();}
        @Override public void resetFallDistance(){d.query("reset");super.resetFallDistance();}
        void check(double dy,boolean ground,BlockState state,BlockPos p){checkFallDamage(dy,ground,state,p);}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);var unsafe=field.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);positionField=Entity.class.getDeclaredField("position");positionField.setAccessible(true);supportField=Entity.class.getDeclaredField("mainSupportingBlockPos");supportField.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.input=f;d.out=out;var level=(Level)allocate.invoke(unsafe,Level.class);level.d=d;level.queried=Blocks.STONE.defaultBlockState();var entity=(Actor)allocate.invoke(unsafe,Actor.class);entity.d=d;d.actor=entity;entity.configure(level);entity.fallDistance=number(f.get("fall"));positionField.set(entity,new Vec3(.3,1.5,-.3));supportField.set(entity,support(f.get("support")));var block=(FallBlock)allocate.invoke(unsafe,FallBlock.class);block.d=d;var state=(State)allocate.invoke(unsafe,State.class);state.block=block;
                try{entity.check(number(f.get("dy")),f.get("ground").getAsBoolean(),state,new BlockPos(3,4,5));out.println("result ok");}catch(Stop error){out.println("result error");}
                var p=(Vec3)positionField.get(entity);out.printf("final %016x %016x %016x %016x%n",Double.doubleToLongBits(entity.fallDistance),Double.doubleToLongBits(p.x),Double.doubleToLongBits(p.y),Double.doubleToLongBits(p.z));var selected=(Optional<?>)supportField.get(entity);if(selected.isEmpty())out.println("support none");else{var pos=(BlockPos)selected.get();out.println("support "+pos.getX()+" "+pos.getY()+" "+pos.getZ());}out.println("queries "+d.calls);
            }
        }
    }
}
