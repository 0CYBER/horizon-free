package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.particles.*;import net.minecraft.resources.Identifier;import net.minecraft.sounds.*;import net.minecraft.util.RandomSource;import net.minecraft.world.entity.*;import net.minecraft.world.level.gameevent.GameEvent;import net.minecraft.world.level.levelgen.*;import net.minecraft.world.phys.Vec3;
/** Actual water-entry splash with native seeded RNG and observable sound/particle requests. */
public final class VanillaWaterSplashOracle {
    static final class Stop extends RuntimeException {}
    static Field position,dimensions;
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static Vec3 vec(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static String vector(Vec3 v){return String.format("%016x %016x %016x",Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z));}
    static final class Data {JsonObject f;PrintWriter out;int calls;Actor actor,other;Vec3 velocity,otherVelocity;SoundEvent low,high;
        void query(String s){out.println(s);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt()){velocity=vec(f.get("after_velocity"));otherVelocity=vec(f.get("after_other_velocity"));try{position.set(actor,vec(f.get("after_position")));dimensions.set(actor,EntityDimensions.fixed(single(f.get("after_width")),2));}catch(Exception e){throw new AssertionError(e);}}}
    }
    static final class World extends net.minecraft.server.level.ServerLevel {Data d;World(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public void addParticle(ParticleOptions type,double x,double y,double z,double vx,double vy,double vz){d.query("particle "+(type==ParticleTypes.BUBBLE?"bubble":type==ParticleTypes.SPLASH?"splash":"unexpected")+" "+vector(new Vec3(x,y,z))+" "+vector(new Vec3(vx,vy,vz)));}
    }
    static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {Data d;boolean other;Actor(){super(null,null);}
        @Override public LivingEntity getControllingPassenger(){d.query("passenger");return switch(d.f.get("passenger").getAsInt()){case 0->null;case 1->this;case 2->d.other;default->throw new AssertionError();};}
        @Override public Vec3 getDeltaMovement(){d.query("velocity "+(other?"other":"self"));return other?d.otherVelocity:d.velocity;}
        @Override protected SoundEvent getSwimSplashSound(){d.query("sound low");return d.low;}
        @Override protected SoundEvent getSwimHighSpeedSplashSound(){d.query("sound high");return d.high;}
        @Override public void playSound(SoundEvent sound,float volume,float pitch){d.query(String.format("play %s %08x %08x",sound==d.low?"low":"high",Float.floatToIntBits(volume),Float.floatToIntBits(pitch)));}
        @Override public void gameEvent(Holder<GameEvent> event){if(event!=GameEvent.SPLASH)throw new AssertionError(event);d.query("event splash");}
    }
    static Field field(Class<?> c,String name)throws Exception{var f=c.getDeclaredField(name);f.setAccessible(true);return f;}
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");var unsafe=field(u,"theUnsafe").get(null);var allocate=u.getMethod("allocateInstance",Class.class);position=field(Entity.class,"position");dimensions=field(Entity.class,"dimensions");var level=field(Entity.class,"level");var random=field(Entity.class,"random");var splash=Entity.class.getDeclaredMethod("doWaterSplashEffect");splash.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.velocity=vec(f.get("velocity"));d.otherVelocity=vec(f.get("other_velocity"));d.low=SoundEvent.createVariableRangeEvent(Identifier.parse("horizon:low"));d.high=SoundEvent.createVariableRangeEvent(Identifier.parse("horizon:high"));d.actor=(Actor)allocate.invoke(unsafe,Actor.class);d.actor.d=d;d.other=(Actor)allocate.invoke(unsafe,Actor.class);d.other.d=d;d.other.other=true;var w=(World)allocate.invoke(unsafe,World.class);w.d=d;level.set(d.actor,w);position.set(d.actor,vec(f.get("position")));dimensions.set(d.actor,EntityDimensions.fixed(single(f.get("width")),2));long seed=f.get("seed").getAsLong();RandomSource rng=f.get("legacy").getAsBoolean()?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
            var source=(RandomSource)Proxy.newProxyInstance(RandomSource.class.getClassLoader(),new Class<?>[]{RandomSource.class},(proxy,method,values)->{switch(method.getName()){case "nextFloat":d.query("float");return rng.nextFloat();case "nextDouble":d.query("double");return rng.nextDouble();default:throw new AssertionError(method);}});random.set(d.actor,source);
            try{splash.invoke(d.actor);out.println("result ok");}catch(InvocationTargetException e){if(e.getCause() instanceof Stop)out.println("error");else throw e;}
            out.println("final "+vector((Vec3)position.get(d.actor))+" "+String.format("%08x",Float.floatToIntBits(d.actor.getBbWidth())));out.println("next "+rng.nextLong());out.println("queries "+d.calls);
        }}
    }
}
