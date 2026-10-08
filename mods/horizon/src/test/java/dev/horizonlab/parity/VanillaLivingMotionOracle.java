package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.world.entity.*;import net.minecraft.world.entity.ai.attributes.*;import net.minecraft.world.effect.*;import net.minecraft.world.phys.Vec3;
/** Native default living jump, gravity and flight parameter methods. */
public final class VanillaLivingMotionOracle {
    static final class Stop extends RuntimeException {}
    static Field amplifier;
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static Vec3 vec(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static String vector(Vec3 v){return String.format("%016x %016x %016x",Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z));}
    static final class Data {JsonObject f;PrintWriter out;int calls;Vec3 velocity;LivingEntity passenger;
        void query(String s){out.println(s);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt())velocity=vec(f.get("after_velocity"));}
        boolean flag(String k){return f.get(k).getAsBoolean();}}
    static final class Human extends net.minecraft.world.entity.player.Player {Human(){super(null,null);}@Override public net.minecraft.world.level.GameType gameMode(){return net.minecraft.world.level.GameType.SURVIVAL;}}
    static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {
        Data d;Actor(){super(null,null);}
        @Override public Vec3 getDeltaMovement(){return d.velocity;}
        @Override public void setDeltaMovement(Vec3 v){d.query("set "+vector(v));d.velocity=v;}
        @Override public double getAttributeValue(Holder<Attribute> attribute){String name=attribute==Attributes.JUMP_STRENGTH?"strength":attribute==Attributes.GRAVITY?"gravity":attribute==Attributes.AIR_DRAG_MODIFIER?"drag-modifier":null;if(name==null)throw new AssertionError(attribute);d.query(name);return number(d.f.get(name.replace('-','_')));}
        @Override protected float getBlockJumpFactor(){d.query("block-factor");return single(d.f.get("block_factor"));}
        @Override public boolean hasEffect(Holder<MobEffect> effect){if(effect==MobEffects.JUMP_BOOST){d.query("has-boost");return d.flag("boost");}if(effect==MobEffects.SLOW_FALLING){d.query("slow-falling");return d.flag("slow_falling");}throw new AssertionError(effect);}
        @Override public MobEffectInstance getEffect(Holder<MobEffect> effect){if(effect!=MobEffects.JUMP_BOOST)throw new AssertionError(effect);d.query("get-boost");var e=new MobEffectInstance(effect,100,0);try{amplifier.setInt(e,d.f.get("amplifier").getAsInt());}catch(ReflectiveOperationException ex){throw new AssertionError(ex);}return e;}
        @Override public boolean isSprinting(){d.query("sprint");return d.flag("sprint");}
        @Override public float getYRot(){d.query("yaw");return single(d.f.get("yaw"));}
        @Override public boolean isNoGravity(){d.query("no-gravity");return d.flag("no_gravity");}
        @Override public LivingEntity getControllingPassenger(){d.query("passenger");return d.passenger;}
        @Override public float getSpeed(){d.query("speed");return single(d.f.get("speed"));}
        @Override protected boolean omnidirectionalAirMover(){d.query("omni");return d.flag("omni");}
    }
    static Method method(Class<?> c,String name,Class<?>... args)throws Exception{var m=c.getDeclaredMethod(name,args);m.setAccessible(true);return m;}
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);var unsafe=uf.get(null);var allocate=u.getMethod("allocateInstance",Class.class);amplifier=MobEffectInstance.class.getDeclaredField("amplifier");amplifier.setAccessible(true);
        var power=method(LivingEntity.class,"getJumpPower",float.class);var effective=method(LivingEntity.class,"getEffectiveGravity");var apply=method(Entity.class,"applyGravity");var up=method(LivingEntity.class,"jumpInLiquid",net.minecraft.tags.TagKey.class);var down=method(LivingEntity.class,"goDownInWater");var speed=method(LivingEntity.class,"getFlyingSpeed");var drag=method(LivingEntity.class,"getAirDrag");Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){
            var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.velocity=vec(f.get("velocity"));int p=f.get("passenger").getAsInt();d.passenger=p==0?null:(LivingEntity)allocate.invoke(unsafe,p==2?Human.class:Actor.class);var entity=(Actor)allocate.invoke(unsafe,Actor.class);entity.d=d;entity.needsSync=d.flag("sync");
            try{Object result=switch(f.get("mode").getAsString()){case "boost"->entity.getJumpBoostPower();case "power"->power.invoke(entity,single(f.get("scale")));case "jump"->{entity.jumpFromGround();yield null;}case "gravity"->entity.getGravity();case "effective"->effective.invoke(entity);case "apply"->apply.invoke(entity);case "up"->up.invoke(entity,net.minecraft.tags.FluidTags.WATER);case "down"->down.invoke(entity);case "speed"->speed.invoke(entity);case "drag"->drag.invoke(entity);case "add"->{entity.addDeltaMovement(vec(f.get("delta")));yield null;}default->throw new AssertionError();};if(result instanceof Float v)out.printf("float %08x%n",Float.floatToIntBits(v));else if(result instanceof Double v)out.printf("double %016x%n",Double.doubleToLongBits(v));out.println("result ok");}catch(Stop e){out.println("result error");}catch(InvocationTargetException e){if(e.getCause() instanceof Stop)out.println("result error");else throw e;}
            out.println("final "+vector(d.velocity)+" "+entity.needsSync);out.println("queries "+d.calls);
        }}
    }
}
