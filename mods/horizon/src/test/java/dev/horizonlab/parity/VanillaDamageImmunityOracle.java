package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.lang.invoke.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.resources.*;import net.minecraft.tags.*;import net.minecraft.world.entity.*;import net.minecraft.world.entity.player.*;import net.minecraft.world.damagesource.*;import net.minecraft.world.item.*;import net.minecraft.world.level.gamerules.*;import net.minecraft.world.flag.FeatureFlagSet;import net.minecraft.server.level.*;import net.minecraft.server.network.ServerGamePacketListenerImpl;
/** Native immunity layers with empty equipment and controlled source/rule queries. */
public final class VanillaDamageImmunityOracle {
    static final class Stop extends RuntimeException {}
    static Object unsafe;static Method allocate;
    static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
    static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
    static final class Data {JsonObject f;PrintWriter out;int calls;boolean changed;Rules rules;
        JsonObject state(){return f.getAsJsonObject(changed?"after":"initial");}boolean flag(String n){return state().get(n).getAsBoolean();}void event(String s){out.println(s);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt())changed=true;}}
    static final class Source extends DamageSource {Data d;Source(){super(null);}
        @Override public boolean is(TagKey<DamageType> tag){String n=tag==DamageTypeTags.BYPASSES_INVULNERABILITY?"bypass":tag==DamageTypeTags.IS_FIRE?"fire":tag==DamageTypeTags.IS_FALL?"fall":tag==DamageTypeTags.IS_DROWNING?"drowning":tag==DamageTypeTags.IS_FREEZING?"freezing":null;if(n==null)throw new AssertionError();d.event("tag "+n);return d.flag(n);}
        @Override public boolean is(ResourceKey<DamageType> k){if(k!=DamageTypes.ENDER_PEARL)throw new AssertionError();d.event("pearl");return d.flag("pearl");}
        @Override public boolean isCreativePlayer(){d.event("creative");return d.flag("creative");}}
    static final class Rules extends GameRules {Data d;Rules(){super(FeatureFlagSet.of());}@SuppressWarnings("unchecked") @Override public <T>T get(GameRule<T> rule){String n=rule==GameRules.DROWNING_DAMAGE?"drowning":rule==GameRules.FALL_DAMAGE?"fall":rule==GameRules.FIRE_DAMAGE?"fire":rule==GameRules.FREEZE_DAMAGE?"freeze":null;if(n==null)throw new AssertionError();d.event("rule "+n);return (T)Boolean.valueOf(d.flag("rule_"+n));}}
    static final class Server extends ServerLevel {Data d;Server(){super(null,null,null,null,null,null,false,0L,List.of(),false);}@Override public GameRules getGameRules(){d.event("rules");return d.rules;}}
    static final class Connection extends ServerGamePacketListenerImpl {Data d;Connection(){super(null,null,null,null);}@Override public boolean hasClientLoaded(){d.event("loaded");return d.flag("loaded");}}
    static final class Human extends ServerPlayer {Data d;Human(){super(null,null,null,null);}
        @Override public boolean isPermanentlyInvulnerable(){d.event("permanent");return d.flag("permanent");}
        @Override public boolean isTemporarilyInvulnerable(){d.event("temporary");return d.flag("temporary");}
        @Override public boolean fireImmune(){d.event("fire-immune");return d.flag("fire_immune");}
        @Override public boolean is(TagKey<EntityType<?>> tag){if(tag!=EntityTypeTags.FALL_DAMAGE_IMMUNE)throw new AssertionError();d.event("fall-immune");return d.flag("fall_immune");}
        @Override public ItemStack getItemBySlot(EquipmentSlot s){d.event("slot "+s.ordinal());return ItemStack.EMPTY;}
        @Override public boolean isChangingDimension(){d.event("changing");return d.flag("changing");}
        boolean base(DamageSource s){return super.isInvulnerableToBase(s);}
    }
    public static void main(String[] args)throws Throwable{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);if(args[0].equals("export")){var slots=new JsonArray();for(var s:EquipmentSlot.VALUES)slots.add(s.ordinal());Files.writeString(dir.resolve("slots.json"),slots.toString());return;}
        var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);var handles=new HashMap<String,MethodHandle>();for(var entry:Map.of("living",LivingEntity.class,"player",Player.class,"server",ServerPlayer.class).entrySet()){var type=entry.getValue();var lookup=MethodHandles.privateLookupIn(type,MethodHandles.lookup());handles.put(entry.getKey(),lookup.findSpecial(type,"isInvulnerableTo",MethodType.methodType(boolean.class,ServerLevel.class,DamageSource.class),type).asType(MethodType.methodType(boolean.class,Human.class,ServerLevel.class,DamageSource.class)));}
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var d=new Data();d.f=JsonParser.parseString(line).getAsJsonObject();d.out=out;d.rules=make(Rules.class);d.rules.d=d;var actor=make(Human.class);actor.d=d;var connection=make(Connection.class);connection.d=d;actor.connection=connection;var level=make(Server.class);level.d=d;var source=make(Source.class);source.d=d;if(d.flag("removed"))field(Entity.class,"removalReason").set(actor,Entity.RemovalReason.DISCARDED);
            try{String mode=d.f.get("mode").getAsString();boolean v=switch(mode){case "entity"->actor.isInvulnerable();case "base"->actor.base(source);default->(boolean)handles.get(mode).invokeExact(actor,(ServerLevel)level,(DamageSource)source);};out.println("bool "+v);out.println("result ok");}catch(Stop e){out.println("result error");}out.println("queries "+d.calls+" "+d.changed);
        }}
    }
}
