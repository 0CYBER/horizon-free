package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import net.minecraft.core.*;
import net.minecraft.tags.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.decoration.ArmorStand;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.level.gameevent.GameEvent;
import net.minecraft.world.phys.Vec3;
/** Actual Entity restitution with traced provider methods and bounce effects. */
public final class VanillaRestitutionOracle {
    static final class Stop extends RuntimeException {}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static Vec3 vector(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static String vectorText(String prefix,Vec3 v){return String.format("%s %016x %016x %016x",prefix,Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z));}
    static final class Data {
        JsonObject input;PrintWriter out;int calls,suppressCalls,gravityCalls;Vec3 velocity;
        void query(String text){out.println(text);if(calls++==input.get("fail").getAsInt())throw new Stop();}
        boolean suppress(){query("suppress");var values=input.getAsJsonArray("suppress");return values.get(Math.min(suppressCalls++,values.size()-1)).getAsBoolean();}
        double gravity(){query("gravity");var values=input.getAsJsonArray("gravity");return number(values.get(Math.min(gravityCalls++,values.size()-1)));}
    }
    static class BounceBlock extends Block {
        Data d;BounceBlock(){super(BlockBehaviour.Properties.of());}
        @Override public float getBounceRestitution(){d.query("block-bounce");return single(d.input.get("block"));}
        @Override public void bounceOn(Level level,BlockState state,BlockPos p,Entity entity,double distance){d.query(String.format("bounce-on %d %d %d %016x",p.getX(),p.getY(),p.getZ(),Double.doubleToLongBits(distance)));if(d.input.get("mutate").getAsBoolean())d.velocity=new Vec3(17,18,19);}
    }
    static class State extends BlockState {
        Data d;BounceBlock block;State(){super(null,null,null);}
        @Override public Block getBlock(){return block;}
        @Override public boolean is(TagKey<Block> tag){if(tag!=BlockTags.SUPPRESSES_BOUNCE)throw new AssertionError(tag);d.query("block-suppresses");return d.input.get("tag").getAsBoolean();}
    }
    static class NonLiving extends Marker {
        NonLiving(){super(null,null);}
        Data d;
        @Override public boolean isSuppressingBounce(){return d.suppress();}
        @Override protected double getEntityBounciness(){d.query("entity-bounce");return number(d.input.get("entity"));}
        @Override public Vec3 getDeltaMovement(){d.query("velocity");return d.velocity;}
        @Override protected double getEffectiveGravity(){return d.gravity();}
        @Override protected float getAirDrag(){d.query("drag");return single(d.input.get("drag"));}
        @Override public BlockPos getBlockPosBelowThatAffectsMyMovement(){d.query("support-position");return new BlockPos(17,-64,-33);}
        @Override public void gameEvent(Holder<GameEvent> event){if(event!=GameEvent.BOUNCE)throw new AssertionError(event);d.query("bounce-event");}
        @Override public void setDeltaMovement(Vec3 velocity){d.query(vectorText("set-velocity",velocity));d.velocity=velocity;}
    }
    static class Living extends ArmorStand {
        Living(){super(null,null);}
        Data d;
        @Override public boolean isSuppressingBounce(){return d.suppress();}
        @Override protected double getEntityBounciness(){d.query("entity-bounce");return number(d.input.get("entity"));}
        @Override public Vec3 getDeltaMovement(){d.query("velocity");return d.velocity;}
        @Override protected double getEffectiveGravity(){return d.gravity();}
        @Override protected float getAirDrag(){d.query("drag");return single(d.input.get("drag"));}
        @Override public BlockPos getBlockPosBelowThatAffectsMyMovement(){d.query("support-position");return new BlockPos(17,-64,-33);}
        @Override public void gameEvent(Holder<GameEvent> event){if(event!=GameEvent.BOUNCE)throw new AssertionError(event);d.query("bounce-event");}
        @Override public void setDeltaMovement(Vec3 velocity){d.query(vectorText("set-velocity",velocity));d.velocity=velocity;}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var allocator=Class.forName("sun.misc.Unsafe");var access=allocator.getDeclaredField("theUnsafe");access.setAccessible(true);var unsafe=access.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        var method=Entity.class.getDeclaredMethod("restituteMovementAfterCollisions",BlockState.class,boolean.class,boolean.class,Vec3.class);method.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.input=f;d.out=out;d.velocity=vector(f.get("velocity"));Entity entity;
                if(f.get("living").getAsBoolean()){var e=(Living)allocate.invoke(unsafe,Living.class);e.d=d;entity=e;}else{var e=(NonLiving)allocate.invoke(unsafe,NonLiving.class);e.d=d;entity=e;}
                entity.verticalCollision=f.get("vertical").getAsBoolean();entity.verticalCollisionBelow=f.get("below").getAsBoolean();entity.fallDistance=number(f.get("fall"));entity.syncPosition=f.get("sync").getAsBoolean();
                var block=(BounceBlock)allocate.invoke(unsafe,BounceBlock.class);block.d=d;var state=(State)allocate.invoke(unsafe,State.class);state.d=d;state.block=block;
                try{method.invoke(entity,state,f.get("x").getAsBoolean(),f.get("z").getAsBoolean(),vector(f.get("resolved")));out.println("result ok");}
                catch(InvocationTargetException error){if(error.getCause() instanceof Stop)out.println("result error");else throw error;}
                out.println(vectorText("final",d.velocity));out.println("sync "+entity.syncPosition);out.println("queries "+d.calls);
            }
        }
    }
}
