package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.tags.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.level.gameevent.*;
import net.minecraft.world.phys.Vec3;
/** Invokes native movement emission, including its private block effect dispatch. */
public final class VanillaMovementEmissionOracle {
    static final class Stop extends RuntimeException {}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static Vec3 vector(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static BlockPos position(JsonElement v){var a=v.getAsJsonArray();return new BlockPos(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt());}
    static String pos(BlockPos p){return p.getX()+" "+p.getY()+" "+p.getZ();}
    static Field moveDist,flyDist,nextStep;
    static final class Data {
        JsonObject f;PrintWriter out;int calls;Actor actor;State[] states;
        boolean flag(String n){return f.get(n).getAsBoolean();}
        void query(String text){out.println(text);if(calls++==f.get("fail").getAsInt())throw new Stop();}
        void mutate(){if(flag("mutate")){try{moveDist.setFloat(actor,single(f.get("mutated_move")));flyDist.setFloat(actor,single(f.get("mutated_fly")));nextStep.setFloat(actor,single(f.get("mutated_next")));}catch(Exception e){throw new AssertionError(e);}}}
    }
    static final class State extends BlockState {
        Data d;int id;State(){super(null,null,null);}
        boolean flag(String n){return d.f.getAsJsonArray("states").get(id).getAsJsonObject().get(n).getAsBoolean();}
        @Override public boolean isAir(){d.query("air "+id);return flag("air");}
        @Override public boolean is(TagKey<Block> tag){if(tag==BlockTags.CRYSTAL_SOUND_BLOCKS){d.query("crystal "+id);return false;}if(tag!=BlockTags.CLIMBABLE)throw new AssertionError(tag);d.query("climbable "+id);return flag("climbable");}
        @Override public boolean is(Block block){if(block!=Blocks.POWDER_SNOW)throw new AssertionError(block);d.query("powder "+id);return flag("powder");}
    }
    static final class Level extends net.minecraft.server.level.ServerLevel {
        Data d;Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public BlockState getBlockState(BlockPos p){d.query("state "+pos(p));return d.states[1];}
        @Override public void gameEvent(Holder<GameEvent> e,Vec3 p,GameEvent.Context c){if(e!=GameEvent.STEP||c.sourceEntity()!=d.actor)throw new AssertionError();d.query(String.format("step %d %016x %016x %016x",((State)c.affectedState()).id,Double.doubleToLongBits(p.x),Double.doubleToLongBits(p.y),Double.doubleToLongBits(p.z)));d.mutate();}
    }
    static final class Actor extends Marker {
        Data d;Actor(){super(null,null);}void configure(Level w){setLevel(w);}
        @Override public BlockPos getOnPos(){d.query("on-pos");return VanillaMovementEmissionOracle.position(d.f.get("on_pos"));}
        @Override public boolean onGround(){d.query("ground");return d.flag("ground");}
        @Override public boolean isCrouching(){d.query("crouch");return d.flag("crouch");}
        @Override public boolean isOnRails(){d.query("rails");return d.flag("rails");}
        @Override public boolean isSwimming(){d.query("swimming");return d.flag("swimming");}
        @Override public boolean isInWater(){d.query("water");return d.flag("water");}
        @Override protected void playStepSound(BlockPos p,BlockState s){d.query("walking "+pos(p)+" "+((State)s).id);d.mutate();}
        @Override protected float nextStep(){d.query("next-step");return super.nextStep();}
        @Override protected void waterSwimSound(){d.query("water-sound");d.mutate();}
        @Override public void gameEvent(Holder<GameEvent> e){d.query("event "+(e==GameEvent.SWIM?"swim":e==GameEvent.FLAP?"flap":"unknown"));d.mutate();}
        @Override protected boolean isFlapping(){d.query("flapping");return d.flag("flapping");}
        @Override protected void onFlap(){d.query("on-flap");d.mutate();}
        @Override protected MovementEmission getMovementEmission(){d.query("emission");return MovementEmission.values()[d.f.get("flap_emission").getAsInt()];}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);var unsafe=field.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        moveDist=Entity.class.getDeclaredField("moveDist");moveDist.setAccessible(true);flyDist=Entity.class.getDeclaredField("flyDist");flyDist.setAccessible(true);nextStep=Entity.class.getDeclaredField("nextStep");nextStep.setAccessible(true);var position=Entity.class.getDeclaredField("position");position.setAccessible(true);
        var apply=Entity.class.getDeclaredMethod("applyMovementEmissionAndPlaySound",Entity.MovementEmission.class,Vec3.class,BlockPos.class,BlockState.class);apply.setAccessible(true);var block=Entity.class.getDeclaredMethod("vibrationAndSoundEffectsFromBlock",BlockPos.class,BlockState.class,boolean.class,boolean.class,Vec3.class);block.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){
            var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;var actor=(Actor)allocate.invoke(unsafe,Actor.class);actor.d=d;d.actor=actor;var level=(Level)allocate.invoke(unsafe,Level.class);level.d=d;actor.configure(level);d.states=new State[2];for(int i=0;i<2;i++){var s=(State)allocate.invoke(unsafe,State.class);s.d=d;s.id=i;d.states[i]=s;}
            moveDist.setFloat(actor,single(f.get("move")));flyDist.setFloat(actor,single(f.get("fly")));nextStep.setFloat(actor,single(f.get("next")));position.set(actor,vector(f.get("position")));
            try{if(f.get("block").getAsBoolean())out.println("result "+block.invoke(actor,position(f.get("legacy")),d.states[0],d.flag("sounds"),d.flag("events"),vector(f.get("movement"))));else{apply.invoke(actor,Entity.MovementEmission.values()[f.get("emission").getAsInt()],vector(f.get("movement")),position(f.get("legacy")),d.states[0]);out.println("ok");}}catch(InvocationTargetException e){if(e.getCause() instanceof Stop)out.println("error");else throw e;}
            out.printf("distances %08x %08x %08x%n",Float.floatToIntBits(moveDist.getFloat(actor)),Float.floatToIntBits(flyDist.getFloat(actor)),Float.floatToIntBits(nextStep.getFloat(actor)));out.println("queries "+d.calls);
        }}
    }
}
