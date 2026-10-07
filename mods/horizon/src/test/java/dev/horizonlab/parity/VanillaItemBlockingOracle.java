package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.component.*;import net.minecraft.core.registries.*;import net.minecraft.tags.*;import net.minecraft.world.InteractionHand;import net.minecraft.world.entity.*;import net.minecraft.world.entity.player.*;import net.minecraft.world.damagesource.*;import net.minecraft.world.item.*;import net.minecraft.world.item.component.BlocksAttacks;import net.minecraft.server.level.*;import net.minecraft.stats.*;import net.minecraft.world.phys.Vec3;
/** Actual living item-blocking dispatch with native components and undamageable stacks. */
public final class VanillaItemBlockingOracle {
    static final class Stop extends RuntimeException {}
    static Object unsafe;static Method allocate;
    static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static Vec3 vec(JsonElement v){if(v.isJsonNull())return null;var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static final class Data {JsonObject f;PrintWriter out;int calls;boolean changed;ItemStack[] items;Server[] levels;Entity[] entities;List<Holder<DamageType>> types;
        JsonObject state(){return f.getAsJsonObject(changed?"after":"initial");}void event(String s){out.println(s);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt())changed=true;}}
    static final class Source extends DamageSource {Data d;Source(){super(null);}
        @Override public Holder<DamageType> typeHolder(){d.event("type-holder");return d.types.get(d.state().get("type").getAsInt());}
        @Override public Entity getDirectEntity(){d.event("direct");return d.entities[d.state().get("direct").getAsInt()];}
        @Override public Vec3 getSourcePosition(){d.event("source-position");return vec(d.state().get("source"));}
        @Override public boolean is(TagKey<DamageType> tag){if(tag!=DamageTypeTags.IS_PROJECTILE)throw new AssertionError();d.event("projectile");return d.state().get("projectile").getAsBoolean();}}
    static final class Arrow extends net.minecraft.world.entity.projectile.arrow.Arrow {Data d;Arrow(){super(null,null);}@Override public byte getPierceLevel(){d.event("pierce");return d.state().get("pierce").getAsByte();}}
    static final class Server extends ServerLevel {Data d;int id;Server(){super(null,null,null,null,null,null,false,0L,List.of(),false);}@Override public boolean isClientSide(){d.event("client "+id);return d.state().get("client").getAsBoolean();}}
    static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {Actor(){super(null,null);}
Data d;
        @Override public ItemStack getItemBlockingWith(){d.event("blocking-item");int id=d.state().get("item").getAsInt();return id<0?null:d.items[id];}
        @Override public float getYHeadRot(){d.event("yaw");return single(d.state().get("yaw"));}
        @Override public Vec3 position(){d.event("position");return vec(d.state().get("position"));}
        @Override public ServerLevel level(){d.event("level");return d.levels[d.state().get("level").getAsInt()];}
        @Override public InteractionHand getUsedItemHand(){d.event("hand");return InteractionHand.values()[d.state().get("hand").getAsInt()];}
        @Override protected void blockUsingItem(ServerLevel l,LivingEntity attacker,DamageSource source,float damage,boolean full){d.event("block-using "+(l==d.levels[0]?0:1)+" "+bits(damage)+" "+full);}
    }
    static final class Human extends ServerPlayer {Human(){super(null,null,null,null);}
Data d;
        @Override public ItemStack getItemBlockingWith(){d.event("blocking-item");int id=d.state().get("item").getAsInt();return id<0?null:d.items[id];}
        @Override public float getYHeadRot(){d.event("yaw");return single(d.state().get("yaw"));}
        @Override public Vec3 position(){d.event("position");return vec(d.state().get("position"));}
        @Override public ServerLevel level(){d.event("level");return d.levels[d.state().get("level").getAsInt()];}
        @Override public InteractionHand getUsedItemHand(){d.event("hand");return InteractionHand.values()[d.state().get("hand").getAsInt()];}
        @Override protected void blockUsingItem(ServerLevel l,LivingEntity attacker,DamageSource source,float damage,boolean full){d.event("block-using "+(l==d.levels[0]?0:1)+" "+bits(damage)+" "+full);}
        @Override public void awardStat(Stat<?> s){if(s.getType()!=Stats.ITEM_USED)throw new AssertionError();d.event("stat "+(s.getValue()==Items.SHIELD?0:s.getValue()==Items.STICK?1:-1));}
    }
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);unsafe=uf.get(null);allocate=u.getMethod("allocateInstance",Class.class);var ctor=ItemStack.class.getDeclaredConstructor(Holder.class,int.class,PatchedDataComponentMap.class);ctor.setAccessible(true);var a=Holder.direct(new DamageType("a",0));var b=Holder.direct(new DamageType("b",0));List<Holder<DamageType>> types=List.of(a,b);List<HolderSet<DamageType>> sets=List.of(HolderSet.direct(List.of(a)),HolderSet.direct(List.of(b)),HolderSet.direct(List.of(a,b)),HolderSet.direct(List.of()));
        Path dir=Path.of(args[1]);try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.types=types;d.items=new ItemStack[2];for(int i=0;i<2;i++){var c=f.getAsJsonArray("components").get(i).getAsJsonObject();d.items[i]=(ItemStack)ctor.newInstance(BuiltInRegistries.ITEM.wrapAsHolder(i==0?Items.SHIELD:Items.STICK),1,new PatchedDataComponentMap(DataComponentMap.EMPTY));if(!c.get("present").getAsBoolean())continue;var reductions=new ArrayList<BlocksAttacks.DamageReduction>();for(var rv:c.getAsJsonArray("reductions")){var r=rv.getAsJsonObject();int set=r.get("set").getAsInt();reductions.add(new BlocksAttacks.DamageReduction(single(r.get("angle")),set==0?Optional.empty():Optional.of(sets.get(set-1)),single(r.get("base")),single(r.get("factor"))));}var item=c.getAsJsonObject("item_damage");int bypass=c.get("bypass").getAsInt();d.items[i].set(DataComponents.BLOCKS_ATTACKS,new BlocksAttacks(0,1,reductions,new BlocksAttacks.ItemDamageFunction(single(item.get("threshold")),single(item.get("base")),single(item.get("factor"))),bypass==0?Optional.empty():Optional.of(sets.get(bypass-1)),Optional.empty(),Optional.empty()));}
            d.levels=new Server[]{make(Server.class),make(Server.class)};for(int i=0;i<2;i++){d.levels[i].d=d;d.levels[i].id=i;}var arrow=make(Arrow.class);arrow.d=d;d.entities=new Entity[]{null,make(Actor.class),arrow,make(Marker.class)};var source=new Source();source.d=d;LivingEntity actor;if(f.get("player").getAsBoolean()){var p=make(Human.class);p.d=d;actor=p;}else{var p=make(Actor.class);p.d=d;actor=p;}
            try{out.println("blocked "+bits(actor.applyItemBlocking(d.levels[0],source,single(f.get("damage")))));out.println("result ok");}catch(Stop e){out.println("result error");}out.println("queries "+d.calls+" "+d.changed);out.println("items "+d.items[0].getCount()+" "+d.items[0].getDamageValue()+" "+d.items[1].getCount()+" "+d.items[1].getDamageValue());
        }}
    }
}
