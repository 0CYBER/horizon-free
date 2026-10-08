package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.Holder;import net.minecraft.resources.Identifier;import net.minecraft.world.entity.*;import net.minecraft.world.entity.ai.attributes.*;import net.minecraft.world.damagesource.*;import net.minecraft.server.level.*;import net.minecraft.world.level.gameevent.GameEvent;import net.minecraft.stats.Stats;
/** Actual LivingEntity/Player inner damage application with controlled phase hooks. */
public final class VanillaLivingDamageOracle {
    static final class Stop extends RuntimeException {}
    static Object unsafe;static Method allocate;
    static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static final class Data {JsonObject f;PrintWriter out;int calls;boolean changed;float health,absorption,exhaustion,recorded;LivingEntity actor;Actor other;Human attacker;Tracker tracker;
        JsonObject state(){return f.getAsJsonObject(changed?"after":"initial");}
        void reset(){var s=state();health=single(s.get("health"));absorption=single(s.get("absorption"));exhaustion=single(s.get("exhaustion"));}
        void query(String s){out.println(s);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt()){changed=true;reset();}}
    }
    static final class Source extends DamageSource {Data d;Source(){super(null);}@Override public Entity getEntity(){d.query("attacker");return switch(d.state().get("attacker").getAsInt()){case 0->null;case 1->d.other;case 2->d.attacker;default->throw new AssertionError();};}@Override public float getFoodExhaustion(){d.query("food-exhaustion");return single(d.state().get("food_exhaustion"));}}
    static final class Tracker extends CombatTracker {Data d;Tracker(){super(null);}@Override public void recordDamage(DamageSource s,float v){d.query("record "+bits(v));d.recorded=v;}}
    static final class Level extends ServerLevel {Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}}
    static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {Actor(){super(null,null);}

        Data d;
        @Override public boolean isInvulnerableTo(ServerLevel l,DamageSource s){d.query("invulnerable");return d.state().get("invulnerable").getAsBoolean();}
        @Override protected float getDamageAfterArmorAbsorb(DamageSource s,float v){d.query("armor "+bits(v));return single(d.state().get("armor"));}
        @Override protected float getDamageAfterMagicAbsorb(DamageSource s,float v){d.query("magic "+bits(v));return single(d.state().get("magic"));}
        @Override public float getAbsorptionAmount(){d.query("absorption");return d.absorption;}
        @Override public double getAttributeValue(Holder<Attribute> a){if(a!=Attributes.MAX_ABSORPTION)throw new AssertionError();d.query("max-absorption");return number(d.state().get("max_absorption"));}
        @Override protected void internalSetAbsorptionAmount(float v){d.query("set-absorption "+bits(v));d.absorption=v;}
        @Override public float getHealth(){d.query("health");return d.health;}
        @Override public void setHealth(float v){d.query("set-health "+bits(v));d.health=v;}
        @Override public CombatTracker getCombatTracker(){d.query("tracker");return d.tracker;}
        @Override public void gameEvent(Holder<GameEvent> e){if(e!=GameEvent.ENTITY_DAMAGE)throw new AssertionError();d.query("damage-event");}
        void run(ServerLevel l,DamageSource s,float v){super.actuallyHurt(l,s,v);}
    }
    static final class Human extends ServerPlayer {Human(){super(null,null,null,null);}

        Data d;
        @Override public boolean isInvulnerableTo(ServerLevel l,DamageSource s){d.query("invulnerable");return d.state().get("invulnerable").getAsBoolean();}
        @Override protected float getDamageAfterArmorAbsorb(DamageSource s,float v){d.query("armor "+bits(v));return single(d.state().get("armor"));}
        @Override protected float getDamageAfterMagicAbsorb(DamageSource s,float v){d.query("magic "+bits(v));return single(d.state().get("magic"));}
        @Override public float getAbsorptionAmount(){d.query("absorption");return d.absorption;}
        @Override public double getAttributeValue(Holder<Attribute> a){if(a!=Attributes.MAX_ABSORPTION)throw new AssertionError();d.query("max-absorption");return number(d.state().get("max_absorption"));}
        @Override protected void internalSetAbsorptionAmount(float v){d.query("set-absorption "+bits(v));d.absorption=v;}
        @Override public float getHealth(){d.query("health");return d.health;}
        @Override public void setHealth(float v){d.query("set-health "+bits(v));d.health=v;}
        @Override public CombatTracker getCombatTracker(){d.query("tracker");return d.tracker;}
        @Override public void gameEvent(Holder<GameEvent> e){if(e!=GameEvent.ENTITY_DAMAGE)throw new AssertionError();d.query("damage-event");}
        void run(ServerLevel l,DamageSource s,float v){super.actuallyHurt(l,s,v);}

        @Override public void causeFoodExhaustion(float v){d.query("exhaust "+bits(v));d.exhaustion+=v;}
        @Override public void awardStat(Identifier stat,int value){String name=stat==Stats.DAMAGE_ABSORBED?"absorbed":stat==Stats.DAMAGE_DEALT_ABSORBED?"dealt-absorbed":stat==Stats.DAMAGE_TAKEN?"taken":"unknown";if(name.equals("unknown"))throw new AssertionError();d.query("stat "+(d.actor==this?"self":"attacker")+" "+name+" "+value);}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);unsafe=uf.get(null);allocate=u.getMethod("allocateInstance",Class.class);var level=make(Level.class);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.reset();d.other=make(Actor.class);d.other.d=d;d.attacker=make(Human.class);d.attacker.d=d;d.tracker=make(Tracker.class);d.tracker.d=d;var source=make(Source.class);source.d=d;boolean player=f.get("player").getAsBoolean();if(player){var actor=make(Human.class);actor.d=d;d.actor=actor;}else{var actor=make(Actor.class);actor.d=d;d.actor=actor;}
            try{if(player)((Human)d.actor).run(level,source,single(f.get("damage")));else ((Actor)d.actor).run(level,source,single(f.get("damage")));out.println("result ok");}catch(Stop e){out.println("result error");}
            out.println("final "+bits(d.health)+" "+bits(d.absorption)+" "+bits(d.exhaustion)+" "+bits(d.recorded));out.println("queries "+d.calls);
        }}
    }
}
