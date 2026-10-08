package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.Holder;import net.minecraft.resources.Identifier;import net.minecraft.tags.*;import net.minecraft.world.entity.*;import net.minecraft.world.entity.ai.attributes.*;import net.minecraft.world.damagesource.*;import net.minecraft.world.effect.*;import net.minecraft.world.item.ItemStack;import net.minecraft.world.level.Level;import net.minecraft.server.level.*;import net.minecraft.stats.Stats;
/** Actual living armor/resistance/protection dispatch, with empty equipment. */
public final class VanillaDamageReductionOracle {
    static final class Stop extends RuntimeException {}
    static Object unsafe;static Method allocate;static Field amplifier;
    static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static final class Data {JsonObject f;PrintWriter out;int calls;boolean changed;LivingEntity actor;Actor other;Human attacker;Server level;
        JsonObject state(){return f.getAsJsonObject(changed?"after":"initial");}
        void query(String s){out.println(s);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt())changed=true;}}
    static final class Source extends DamageSource {Data d;Source(){super(null);}
        @Override public boolean is(TagKey<DamageType> tag){String n=tag==DamageTypeTags.BYPASSES_ARMOR?"armor":tag==DamageTypeTags.BYPASSES_EFFECTS?"effects":tag==DamageTypeTags.BYPASSES_RESISTANCE?"resistance":tag==DamageTypeTags.BYPASSES_ENCHANTMENTS?"enchantments":null;if(n==null)throw new AssertionError();d.query("bypass "+n);return d.state().get("bypass_"+n).getAsBoolean();}
        @Override public ItemStack getWeaponItem(){d.query("weapon");return d.state().get("weapon").getAsBoolean()?ItemStack.EMPTY:null;}
        @Override public Entity getEntity(){d.query("attacker");return switch(d.f.get("attacker").getAsInt()){case 0->null;case 1->d.other;case 2->d.attacker;default->throw new AssertionError();};}}
    static final class Server extends ServerLevel {Server(){super(null,null,null,null,null,null,false,0L,List.of(),false);}}
    static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {Actor(){super(null,null);}

        Data d;
        @Override public double getAttributeValue(Holder<Attribute> a){String k=a==Attributes.ARMOR?"armor":a==Attributes.ARMOR_TOUGHNESS?"toughness":null;if(k==null)throw new AssertionError();d.query(k);return number(d.state().get(k));}
        @Override protected void hurtArmor(DamageSource s,float v){d.query("hurt-armor "+bits(v));}
        @Override public boolean hasEffect(Holder<MobEffect> e){if(e!=MobEffects.RESISTANCE)throw new AssertionError();d.query("has-resistance");return d.state().get("resistance").getAsBoolean();}
        @Override public MobEffectInstance getEffect(Holder<MobEffect> e){if(e!=MobEffects.RESISTANCE)throw new AssertionError();d.query("effect");var result=new MobEffectInstance(e,100,0);try{amplifier.setInt(result,d.state().get("amplifier").getAsInt());}catch(Exception ex){throw new AssertionError(ex);}return result;}
        @Override public Level level(){d.query("level");return d.state().get("server").getAsBoolean()?d.level:null;}
        @Override public ItemStack getItemBySlot(EquipmentSlot slot){d.query("slot "+slot.ordinal());return ItemStack.EMPTY;}
        float armor(DamageSource s,float v){return super.getDamageAfterArmorAbsorb(s,v);}
        float magic(DamageSource s,float v){return super.getDamageAfterMagicAbsorb(s,v);}
    }
    static final class Human extends ServerPlayer {Human(){super(null,null,null,null);}

        Data d;
        @Override public double getAttributeValue(Holder<Attribute> a){String k=a==Attributes.ARMOR?"armor":a==Attributes.ARMOR_TOUGHNESS?"toughness":null;if(k==null)throw new AssertionError();d.query(k);return number(d.state().get(k));}
        @Override protected void hurtArmor(DamageSource s,float v){d.query("hurt-armor "+bits(v));}
        @Override public boolean hasEffect(Holder<MobEffect> e){if(e!=MobEffects.RESISTANCE)throw new AssertionError();d.query("has-resistance");return d.state().get("resistance").getAsBoolean();}
        @Override public MobEffectInstance getEffect(Holder<MobEffect> e){if(e!=MobEffects.RESISTANCE)throw new AssertionError();d.query("effect");var result=new MobEffectInstance(e,100,0);try{amplifier.setInt(result,d.state().get("amplifier").getAsInt());}catch(Exception ex){throw new AssertionError(ex);}return result;}
        @Override public ServerLevel level(){d.query("level");return d.state().get("server").getAsBoolean()?d.level:null;}
        @Override public ItemStack getItemBySlot(EquipmentSlot slot){d.query("slot "+slot.ordinal());return ItemStack.EMPTY;}
        float armor(DamageSource s,float v){return super.getDamageAfterArmorAbsorb(s,v);}
        float magic(DamageSource s,float v){return super.getDamageAfterMagicAbsorb(s,v);}

        @Override public void awardStat(Identifier stat,int v){String n=stat==Stats.DAMAGE_RESISTED?"resisted":stat==Stats.DAMAGE_DEALT_RESISTED?"dealt-resisted":null;if(n==null)throw new AssertionError();d.query("stat "+(d.actor==this?"self":"attacker")+" "+n+" "+v);}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);if(args[0].equals("export")){var slots=new JsonArray();for(var slot:EquipmentSlot.VALUES)slots.add(slot.ordinal());Files.writeString(dir.resolve("slots.json"),slots.toString());return;}
        var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);unsafe=uf.get(null);allocate=u.getMethod("allocateInstance",Class.class);amplifier=MobEffectInstance.class.getDeclaredField("amplifier");amplifier.setAccessible(true);var level=make(Server.class);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.level=level;d.other=make(Actor.class);d.other.d=d;d.attacker=make(Human.class);d.attacker.d=d;var source=make(Source.class);source.d=d;boolean player=f.get("player").getAsBoolean();if(player){var actor=make(Human.class);actor.d=d;d.actor=actor;}else{var actor=make(Actor.class);actor.d=d;d.actor=actor;}
            try{String mode=f.get("mode").getAsString();float damage=single(f.get("damage"));if(mode.equals("armor-value"))out.println("int "+d.actor.getArmorValue());else{float result=player?(mode.equals("armor")?((Human)d.actor).armor(source,damage):((Human)d.actor).magic(source,damage)):(mode.equals("armor")?((Actor)d.actor).armor(source,damage):((Actor)d.actor).magic(source,damage));out.println("float "+bits(result));}out.println("result ok");}catch(Stop e){out.println("result error");}out.println("queries "+d.calls);
        }}
    }
}
