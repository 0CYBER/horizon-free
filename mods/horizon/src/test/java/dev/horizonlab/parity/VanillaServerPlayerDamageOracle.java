package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.registries.*;import net.minecraft.resources.Identifier;import net.minecraft.tags.*;import net.minecraft.world.entity.*;import net.minecraft.world.entity.player.*;import net.minecraft.world.entity.ai.attributes.*;import net.minecraft.network.syncher.*;import net.minecraft.world.damagesource.*;import net.minecraft.world.effect.*;import net.minecraft.world.item.ItemStack;import net.minecraft.world.food.FoodData;import net.minecraft.server.level.*;import net.minecraft.stats.Stats;import net.minecraft.world.level.gameevent.GameEvent;
/** Native player inner damage with actual attributes, synchronized floats and food storage. */
public final class VanillaServerPlayerDamageOracle {
    static final class Stop extends RuntimeException {}
    static Object unsafe;static Method allocate;static EntityDataAccessor<Float> healthKey,absorptionKey;static Field amplifier;
    static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
    static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static String values(List<SynchedEntityData.DataValue<?>> values){if(values==null)return "none";var s=new StringJoiner(",");for(var v:values)s.add(v.id()+":"+bits((Float)v.value()));return s.length()==0?"empty":s.toString();}
    static final class Data {JsonObject op;PrintWriter out;int calls;float recorded;AttributeMap attributes;SynchedEntityData data;Server level;Tracker tracker;
        void event(String s){out.println(s);if(calls++==op.get("fail").getAsInt())throw new Stop();}}
    static final class Source extends DamageSource {Data d;Source(){super(null);}
        @Override public boolean is(TagKey<DamageType> tag){String n=tag==DamageTypeTags.BYPASSES_ARMOR?"armor":tag==DamageTypeTags.BYPASSES_EFFECTS?"effects":tag==DamageTypeTags.BYPASSES_RESISTANCE?"resistance":tag==DamageTypeTags.BYPASSES_ENCHANTMENTS?"enchantments":null;if(n==null)throw new AssertionError();d.event("bypass "+n);return d.op.get("bypass_"+n).getAsBoolean();}
        @Override public ItemStack getWeaponItem(){d.event("weapon");return d.op.get("weapon").getAsBoolean()?ItemStack.EMPTY:null;}
        @Override public float getFoodExhaustion(){d.event("food-exhaustion");return single(d.op.get("food_exhaustion"));}}
    static final class Server extends ServerLevel {Data d;Server(){super(null,null,null,null,null,null,false,0L,List.of(),false);}@Override public boolean isClientSide(){d.event("client");return d.op.get("client").getAsBoolean();}}
    static final class Tracker extends CombatTracker {Data d;Tracker(){super(null);}@Override public void recordDamage(DamageSource s,float v){d.event("record "+bits(v));d.recorded=v;}}
    static final class Actor extends ServerPlayer {Data d;Actor(){super(null,null,null,null);}
        @Override public boolean isInvulnerableTo(ServerLevel l,DamageSource s){d.event("invulnerable");return d.op.get("invulnerable").getAsBoolean();}
        @Override public double getAttributeValue(Holder<Attribute> a){return d.attributes.getValue(a);}
        @Override protected void hurtArmor(DamageSource s,float v){d.event("hurt-armor "+bits(v));if(d.op.get("break_armor").getAsBoolean())d.attributes.getInstance(Attributes.ARMOR).setBaseValue(number(d.op.get("armor_after")));}
        @Override public boolean hasEffect(Holder<MobEffect> e){if(e!=MobEffects.RESISTANCE)throw new AssertionError();d.event("has-resistance");return d.op.get("resistance").getAsBoolean();}
        @Override public MobEffectInstance getEffect(Holder<MobEffect> e){if(e!=MobEffects.RESISTANCE)throw new AssertionError();d.event("effect");var v=new MobEffectInstance(e,100,0);try{amplifier.setInt(v,d.op.get("amplifier").getAsInt());}catch(Exception ex){throw new AssertionError(ex);}return v;}
        @Override public ServerLevel level(){d.event("level");return d.level;}
        @Override public ItemStack getItemBySlot(EquipmentSlot s){d.event("slot "+s.ordinal());return ItemStack.EMPTY;}
        @Override public CombatTracker getCombatTracker(){d.event("tracker");return d.tracker;}
        @Override public void gameEvent(Holder<GameEvent> e){if(e!=GameEvent.ENTITY_DAMAGE)throw new AssertionError();d.event("damage-event");}
        @Override public void awardStat(Identifier s,int v){String n=s==Stats.DAMAGE_RESISTED?"resisted":s==Stats.DAMAGE_ABSORBED?"absorbed":s==Stats.DAMAGE_TAKEN?"taken":null;if(n==null)throw new AssertionError();d.event("stat "+n+" "+v);}
        @Override public void onSyncedDataUpdated(EntityDataAccessor<?> a){var k=a.id()==healthKey.id()?healthKey:absorptionKey;d.event("updated "+k.id()+" "+bits(d.data.get(k))+" "+d.data.isDirty());if(d.op.get("pack_callback").getAsBoolean())d.out.println("callback-pack "+values(d.data.packDirty()));}
        void run(DamageSource s,float v){super.actuallyHurt(d.level,s,v);}
    }
    @SuppressWarnings("unchecked") public static void main(String[] args)throws Exception{
        Path dir=Path.of(args[1]);if(args[0].equals("export"))VanillaPlayerHealthOracle.main(args);else{net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();}
        healthKey=(EntityDataAccessor<Float>)field(LivingEntity.class,"DATA_HEALTH_ID").get(null);absorptionKey=(EntityDataAccessor<Float>)field(Player.class,"DATA_PLAYER_ABSORPTION_ID").get(null);
        if(args[0].equals("export")){var slots=new JsonArray();for(var s:EquipmentSlot.VALUES)slots.add(s.ordinal());Files.writeString(dir.resolve("slots.json"),slots.toString());return;}
        var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);amplifier=field(MobEffectInstance.class,"amplifier");var ctor=SynchedEntityData.class.getDeclaredConstructor(SyncedDataHolder.class,SynchedEntityData.DataItem[].class);ctor.setAccessible(true);var storage=field(Entity.class,"entityData");var foodField=field(Player.class,"foodData");var abilityField=field(Player.class,"abilities");var exhaustion=field(FoodData.class,"exhaustionLevel");var keys=List.of(Attributes.ARMOR,Attributes.ARMOR_TOUGHNESS,Attributes.MAX_HEALTH,Attributes.MAX_ABSORPTION);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.out=out;d.attributes=new AttributeMap(VanillaEntityAttributeOracle.supplier(BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse("minecraft:player"))));
            for(int i=0;i<keys.size();i++){var a=f.getAsJsonArray("attributes").get(i).getAsJsonObject();var instance=d.attributes.getInstance(keys.get(i));instance.setBaseValue(number(a.get("base")));instance.addTransientModifier(new AttributeModifier(Identifier.parse("horizon:test"),number(a.get("amount")),AttributeModifier.Operation.values()[a.get("operation").getAsInt()]));}
            var actor=make(Actor.class);actor.d=d;d.level=make(Server.class);d.level.d=d;d.tracker=make(Tracker.class);d.tracker.d=d;var source=make(Source.class);source.d=d;var food=new FoodData();exhaustion.setFloat(food,single(f.get("exhaustion")));foodField.set(actor,food);var abilities=new Abilities();abilityField.set(actor,abilities);
            var items=new SynchedEntityData.DataItem<?>[Math.max(healthKey.id(),absorptionKey.id())+1];for(int i=0;i<items.length;i++)items[i]=new SynchedEntityData.DataItem<Float>(new EntityDataAccessor<Float>(i,EntityDataSerializers.FLOAT),0f);var health=new SynchedEntityData.DataItem<Float>(healthKey,single(f.get("health")));var absorption=new SynchedEntityData.DataItem<Float>(absorptionKey,single(f.get("absorption")));items[healthKey.id()]=health;items[absorptionKey.id()]=absorption;d.data=ctor.newInstance(actor,items);storage.set(actor,d.data);
            for(var ov:f.getAsJsonArray("ops")){d.op=ov.getAsJsonObject();d.calls=0;abilities.invulnerable=d.op.get("ability_invulnerable").getAsBoolean();try{actor.run(source,single(d.op.get("damage")));out.println("result ok");}catch(Stop e){out.println("result error");}
                out.println("state "+bits(health.getValue())+" "+bits(absorption.getValue())+" "+bits(exhaustion.getFloat(food))+" "+bits(d.recorded)+" "+d.data.isDirty()+" "+health.isDirty()+" "+absorption.isDirty()+" "+d.calls);
                if(d.op.get("pack").getAsBoolean())out.println("packed "+values(d.data.packDirty()));out.println("defaults "+values(d.data.getNonDefaultValues()));
            }
        }}
    }
}
