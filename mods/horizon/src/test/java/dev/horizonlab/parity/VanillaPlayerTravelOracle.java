package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.world.entity.*;import net.minecraft.world.entity.player.*;import net.minecraft.world.level.material.*;import net.minecraft.world.phys.Vec3;
/** Actual Player travel wrapper and flight speed, with controlled parent fluid travel. */
public final class VanillaPlayerTravelOracle {
    static final class Stop extends RuntimeException {}
    static Field position;
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static Vec3 vec(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static String vector(Vec3 v){return String.format("%016x %016x %016x",Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z));}
    static final class Data {JsonObject f;PrintWriter out;int calls;Vec3 velocity;Actor actor;Ability ability;
        void query(String s){out.println(s);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt()){velocity=vec(f.get("after_velocity"));try{position.set(actor,vec(f.get("after_position")));}catch(ReflectiveOperationException e){throw new AssertionError(e);}ability.flying=flag("after_flying");}}
        boolean flag(String k){return f.get(k).getAsBoolean();}}
    static final class Ability extends Abilities {Data d;@Override public float getFlyingSpeed(){d.query("ability-speed");return single(d.f.get("speed"));}}
    static final class Level extends net.minecraft.server.level.ServerLevel {Data d;Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public FluidState getFluidState(BlockPos p){d.query("fluid "+p.getX()+" "+p.getY()+" "+p.getZ());return (d.flag("empty")?Fluids.EMPTY:Fluids.WATER).defaultFluidState();}}
    static final class Actor extends Player {
        Data d;Actor(){super(null,null);}void configure(Level l){setLevel(l);}
        @Override public net.minecraft.world.level.GameType gameMode(){return net.minecraft.world.level.GameType.SURVIVAL;}
        @Override public boolean isPassenger(){d.query("passenger");return d.flag("passenger");}
        @Override public boolean isSwimming(){d.query("swimming");return d.flag("swimming");}
        @Override public Vec3 getLookAngle(){d.query("look");return vec(d.f.get("look"));}
        @Override public Vec3 getDeltaMovement(){return d.velocity;}
        @Override public void setDeltaMovement(Vec3 v){d.query("set "+vector(v));d.velocity=v;}
        @Override public Abilities getAbilities(){d.query("abilities");return d.ability;}
        @Override public boolean isSprinting(){d.query("sprint");return d.flag("sprint");}
        @Override public BlockPos blockPosition(){d.query("block-position");return new BlockPos(17,-64,-33);}
        @Override protected boolean shouldTravelInFluid(FluidState fluid){d.query("should-fluid");return true;}
        @Override protected void travelInFluid(Vec3 input){d.query("base "+vector(input));if(d.flag("base_mutate")){d.velocity=vec(d.f.get("base_velocity"));d.ability.flying=d.flag("base_flying");}}
        float flightSpeed(){return getFlyingSpeed();}
    }
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);var unsafe=uf.get(null);var allocate=u.getMethod("allocateInstance",Class.class);position=Entity.class.getDeclaredField("position");position.setAccessible(true);var abilities=Player.class.getDeclaredField("abilities");abilities.setAccessible(true);var jumping=LivingEntity.class.getDeclaredField("jumping");jumping.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){
            var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.velocity=vec(f.get("velocity"));var entity=(Actor)allocate.invoke(unsafe,Actor.class);d.actor=entity;entity.d=d;var level=(Level)allocate.invoke(unsafe,Level.class);level.d=d;entity.configure(level);var ability=new Ability();ability.d=d;ability.flying=d.flag("flying");d.ability=ability;abilities.set(entity,ability);position.set(entity,vec(f.get("position")));jumping.setBoolean(entity,d.flag("jumping"));
            try{switch(f.get("mode").getAsString()){case "travel"->entity.travel(vec(f.get("input")));case "speed"->out.printf("float %08x%n",Float.floatToIntBits(entity.flightSpeed()));case "affected"->out.println("bool "+entity.isAffectedByFluids());default->throw new AssertionError();}out.println("result ok");}catch(Stop e){out.println("result error");}
            out.println("final "+vector(d.velocity)+" "+vector((Vec3)position.get(entity))+" "+ability.flying);out.println("queries "+d.calls);
        }}
    }
}
