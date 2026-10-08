package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.component.*;import net.minecraft.world.entity.*;import net.minecraft.world.item.*;import net.minecraft.world.InteractionHand;import net.minecraft.world.level.*;import net.minecraft.world.level.gameevent.GameEvent;import net.minecraft.server.level.*;
/** Actual inherited living tick/release/complete/stop lifecycle with custom items. */
public final class VanillaItemUseLifecycleOracle {
    static final class Stop extends RuntimeException {}
    static Object unsafe;static Method allocate;static Field useItem,remaining,kinetic;
    static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
    static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
    static final class Data {JsonObject f;PrintWriter out;int calls;boolean changed,using;Actor actor;ItemStack[] items,hands;Server[] levels;
        JsonObject state(){return f.getAsJsonObject(changed?"after":"initial");}
        void reset(){try{var s=state();useItem.set(actor,items[s.get("item").getAsInt()]);remaining.setInt(actor,s.get("remaining").getAsInt());kinetic.set(actor,s.get("kinetic").getAsBoolean()?new it.unimi.dsi.fastutil.objects.Object2LongOpenHashMap<>():null);using=s.get("using").getAsBoolean();hands=new ItemStack[]{items[s.getAsJsonArray("hands").get(0).getAsInt()],items[s.getAsJsonArray("hands").get(1).getAsInt()]};}catch(Exception e){throw new AssertionError(e);}}
        void event(String name){out.println(name);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt()){changed=true;reset();}}
        int id(ItemStack s){for(int i=0;i<items.length;i++)if(items[i]==s)return i;throw new AssertionError();}
        String snapshot(){try{return id((ItemStack)useItem.get(actor))+" "+remaining.getInt(actor)+" "+(kinetic.get(actor)!=null)+" "+using+" "+id(hands[0])+" "+id(hands[1])+" "+calls+" "+changed;}catch(Exception e){throw new AssertionError(e);}}}
    static final class Server extends ServerLevel {Data d;int id;Server(){super(null,null,null,null,null,null,false,0L,List.of(),false);}@Override public boolean isClientSide(){d.event("client "+id);return d.state().getAsJsonArray("clients").get(id).getAsBoolean();}}
    static final class CustomItem extends Item {Data d;int id;CustomItem(){super(null);}
        @Override public void onUseTick(Level l,LivingEntity a,ItemStack s,int remaining){d.event("tick "+id+" "+d.id(s)+" "+((Server)l).id+" "+remaining);}
        @Override public boolean useOnRelease(ItemStack s){d.event("release-query "+id+" "+d.id(s));return d.state().getAsJsonArray("releases").get(id).getAsBoolean();}
        @Override public boolean releaseUsing(ItemStack s,Level l,LivingEntity a,int remaining){d.event("release "+id+" "+d.id(s)+" "+((Server)l).id+" "+remaining);return false;}
        @Override public ItemStack finishUsingItem(ItemStack s,Level l,LivingEntity a){d.event("finish "+id+" "+d.id(s)+" "+((Server)l).id);return d.items[d.state().getAsJsonArray("finishes").get(id).getAsInt()];}}
    static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {Data d;Actor(){super(null,null);}
        @Override public boolean isUsingItem(){d.event("using");return d.using;}
        @Override public InteractionHand getUsedItemHand(){d.event("hand");return InteractionHand.values()[d.state().get("hand").getAsInt()];}
        @Override public ItemStack getItemInHand(InteractionHand h){d.event("get-hand "+h.ordinal());return d.hands[h.ordinal()];}
        @Override public void setItemInHand(InteractionHand h,ItemStack s){d.event("set-hand "+h.ordinal()+" "+d.id(s));d.hands[h.ordinal()]=s;}
        @Override public ServerLevel level(){d.event("level");return d.levels[d.state().get("level").getAsInt()];}
        @Override public int getUseItemRemainingTicks(){d.event("remaining");return d.state().get("virtual_remaining").getAsInt();}
        @Override protected void setLivingEntityFlag(int flag,boolean value){if(flag!=1)throw new AssertionError();d.event("flag "+value);d.using=value;}
        @Override public void gameEvent(Holder<GameEvent> event){if(event!=GameEvent.ITEM_INTERACT_FINISH)throw new AssertionError();d.event("vibration");}
        void update(ItemStack s){super.updateUsingItem(s);}void complete(){super.completeUsingItem();}}
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);useItem=field(LivingEntity.class,"useItem");remaining=field(LivingEntity.class,"useItemRemaining");kinetic=field(LivingEntity.class,"recentKineticEnemies");var tick=LivingEntity.class.getDeclaredMethod("updatingUsingItem");tick.setAccessible(true);var ctor=ItemStack.class.getDeclaredConstructor(Holder.class,int.class,PatchedDataComponentMap.class);ctor.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var d=new Data();d.f=JsonParser.parseString(line).getAsJsonObject();d.out=out;d.actor=make(Actor.class);d.actor.d=d;d.levels=new Server[]{make(Server.class),make(Server.class)};for(int i=0;i<2;i++){d.levels[i].d=d;d.levels[i].id=i;}var a=make(CustomItem.class);a.d=d;a.id=0;var b=make(CustomItem.class);b.d=d;b.id=1;d.items=new ItemStack[]{(ItemStack)ctor.newInstance(Holder.direct(a),1,new PatchedDataComponentMap(DataComponentMap.EMPTY)),(ItemStack)ctor.newInstance(Holder.direct(d.f.get("shared_type").getAsBoolean()?a:b),1,new PatchedDataComponentMap(DataComponentMap.EMPTY)),ItemStack.EMPTY};for(int i=0;i<2;i++)if(d.f.getAsJsonArray("vibrations").get(i).getAsBoolean())d.items[i].set(DataComponents.USE_EFFECTS,new net.minecraft.world.item.component.UseEffects(false,true,1));d.reset();
            try{switch(d.f.get("mode").getAsString()){case "tick":try{tick.invoke(d.actor);}catch(InvocationTargetException e){if(e.getCause() instanceof Stop)throw new Stop();throw new AssertionError(e.getCause());}break;case "update":d.actor.update(d.items[d.f.get("argument").getAsInt()]);break;case "complete":d.actor.complete();break;case "release":d.actor.releaseUsingItem();break;case "stop":d.actor.stopUsingItem();break;default:throw new AssertionError();}out.println("result ok");}catch(Stop e){out.println("result error");}out.println("state "+d.snapshot());
        }}
    }
}
