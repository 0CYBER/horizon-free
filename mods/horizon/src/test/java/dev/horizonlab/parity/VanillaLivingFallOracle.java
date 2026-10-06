package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.tags.*;
import net.minecraft.sounds.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.decoration.ArmorStand;
import net.minecraft.world.entity.ai.attributes.*;
import net.minecraft.world.damagesource.*;
import net.minecraft.world.phys.Vec3;
/** Actual living fall calculation/cause methods and impulse context transitions. */
public final class VanillaLivingFallOracle {
    static final class Stop extends RuntimeException {}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static Vec3 vector(JsonElement v){if(v.isJsonNull())return null;var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static Field grace,impact;
    static final class Data {JsonObject f;PrintWriter out;int calls,immuneCalls,hurt;void query(String s){out.println(s);if(calls++==f.get("fail").getAsInt())throw new Stop();}}
    static final class Level extends net.minecraft.server.level.ServerLevel {private Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}}
    static final class Actor extends ArmorStand {
        Data d;Actor(){super(null,null);}void configure(Level level){setLevel(level);}
        @Override public boolean is(TagKey<EntityType<?>> tag){if(tag!=EntityTypeTags.FALL_DAMAGE_IMMUNE)throw new AssertionError(tag);d.query("immune");var values=d.f.getAsJsonArray("immune");return values.get(Math.min(d.immuneCalls++,values.size()-1)).getAsBoolean();}
        @Override public double getAttributeValue(Holder<Attribute> attribute){String name=attribute==Attributes.SAFE_FALL_DISTANCE?"safe":attribute==Attributes.FALL_DAMAGE_MULTIPLIER?"damage-factor":null;if(name==null)throw new AssertionError(attribute);d.query(name);return number(d.f.get(name));}
        @Override public void resetCurrentImpulseContext(){d.query("reset");super.resetCurrentImpulseContext();}
        @Override public void tryResetCurrentImpulseContext(){d.query("try-reset");super.tryResetCurrentImpulseContext();}
        @Override protected void propagateFallToPassengers(double distance,float multiplier,DamageSource source){d.query(String.format("passengers %016x %08x",Double.doubleToLongBits(distance),Float.floatToIntBits(multiplier)));if(d.f.get("mutate").getAsBoolean()){try{grace.setInt(this,17);impact.set(this,new Vec3(17,18,19));}catch(ReflectiveOperationException e){throw new AssertionError(e);}}}
        @Override public Fallsounds getFallSounds(){d.query("sounds");return new Fallsounds(SoundEvents.GENERIC_SMALL_FALL,SoundEvents.GENERIC_BIG_FALL);}
        @Override public void playSound(SoundEvent sound,float volume,float pitch){if(volume!=1||pitch!=1)throw new AssertionError();d.query("sound "+(sound==SoundEvents.GENERIC_BIG_FALL?"big":"small"));}
        @Override protected void playBlockFallSound(){d.query("block-sound");}
        @Override public boolean hurtServer(net.minecraft.server.level.ServerLevel level,DamageSource source,float damage){d.query(String.format("hurt %08x",Float.floatToIntBits(damage)));d.hurt++;return d.f.get("hurt_result").getAsBoolean();}
        int calculate(double distance,float multiplier){return calculateFallDamage(distance,multiplier);}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);var unsafe=field.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        grace=LivingEntity.class.getDeclaredField("currentImpulseContextResetGraceTime");grace.setAccessible(true);impact=LivingEntity.class.getDeclaredField("currentImpulseImpactPos");impact.setAccessible(true);var position=Entity.class.getDeclaredField("position");position.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;var entity=(Actor)allocate.invoke(unsafe,Actor.class);entity.d=d;entity.configure(f.get("server").getAsBoolean()?(Level)allocate.invoke(unsafe,Level.class):null);grace.setInt(entity,f.get("grace").getAsInt());impact.set(entity,vector(f.get("impact")));position.set(entity,new Vec3(0,number(f.get("y")),0));
                try{int mode=f.get("mode").getAsInt();if(mode==0)out.println("damage "+entity.calculate(number(f.get("distance")),single(f.get("multiplier"))));else if(mode==1)out.println("caused "+entity.causeFallDamage(number(f.get("distance")),single(f.get("multiplier")),null));else for(var value:f.getAsJsonArray("ops")){var op=value.getAsJsonObject();switch(op.get("mode").getAsInt()){case 0->entity.setIgnoreFallDamageFromCurrentImpulse(op.get("ignore").getAsBoolean(),vector(op.get("position")));case 1->entity.applyPostImpulseGraceTime(op.get("ticks").getAsInt());case 2->entity.tryResetCurrentImpulseContext();case 3->entity.resetCurrentImpulseContext();default->throw new AssertionError();}emit(out,entity);} }
                catch(Stop error){out.println("error");}emit(out,entity);out.println("hurt-count "+d.hurt);out.println("queries "+d.calls);
            }
        }
    }
    static void emit(PrintWriter out,Actor entity)throws Exception{var p=(Vec3)impact.get(entity);out.println("grace "+grace.getInt(entity)+" "+entity.isInPostImpulseGraceTime()+" "+entity.isIgnoringFallDamageFromCurrentImpulse());if(p==null)out.println("impact none");else out.printf("impact %016x %016x %016x%n",Double.doubleToLongBits(p.x),Double.doubleToLongBits(p.y),Double.doubleToLongBits(p.z));}
}
