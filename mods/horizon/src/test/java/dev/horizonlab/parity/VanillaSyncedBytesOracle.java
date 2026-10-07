package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.nio.file.*;import java.util.*;
import net.minecraft.network.syncher.*;
/** Native multi-byte dirty/default lists, assignment and reentrant callbacks. */
public final class VanillaSyncedBytesOracle {
    static final class Stop extends RuntimeException {}
    static byte single(JsonElement v){return (byte)Integer.parseUnsignedInt(v.getAsString(),16);}
    static String bits(byte v){return String.format("%02x",Byte.toUnsignedInt(v));}
    static String values(List<SynchedEntityData.DataValue<?>> list){if(list==null)return "none";var s=new StringJoiner(",");for(var v:list)s.add(v.id()+":"+bits((Byte)v.value()));return s.length()==0?"empty":s.toString();}
    static EntityDataAccessor<Byte> key(int id){return new EntityDataAccessor<Byte>(id,EntityDataSerializers.BYTE);}
    static List<SynchedEntityData.DataValue<?>> values(JsonArray values){var result=new ArrayList<SynchedEntityData.DataValue<?>>();for(var x:values){var v=x.getAsJsonObject();result.add(new SynchedEntityData.DataValue<Byte>(v.get("id").getAsInt(),EntityDataSerializers.BYTE,single(v.get("value"))));}return result;}
    static final class Holder extends net.minecraft.world.entity.decoration.ArmorStand {
        Holder(){super(null,null);}
        void flag(int mask,boolean value){super.setLivingEntityFlag(mask,value);}

        SynchedEntityData data;SynchedEntityData.DataItem<?>[] items;JsonObject op;PrintWriter out;int calls;boolean nested;
        void event(String s){out.println(s);if(calls++==op.get("fail").getAsInt())throw new Stop();}
        @Override public void onSyncedDataUpdated(EntityDataAccessor<?> key){int id=key.id();event("updated "+id+" "+bits((Byte)items[id].getValue())+" "+data.isDirty()+" "+items[id].isDirty());if(nested)return;nested=true;try{switch(op.get("hook").getAsString()){
            case "flag":flag(op.get("mask").getAsInt(),op.get("flag").getAsBoolean());break;case "none":break;case "pack":out.println("callback-pack "+values(data.packDirty()));break;
            case "set":data.set(key(op.get("other").getAsInt()),single(op.get("other_value")),op.get("other_force").getAsBoolean());break;
            case "assign":data.assignValues(List.of(new SynchedEntityData.DataValue<Byte>(op.get("other").getAsInt(),EntityDataSerializers.BYTE,single(op.get("other_value")))));break;
            default:throw new AssertionError();}}finally{nested=false;}}
        @Override public void onSyncedDataUpdated(List<SynchedEntityData.DataValue<?>> values){event("assigned "+values(values));}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var ctor=SynchedEntityData.class.getDeclaredConstructor(SyncedDataHolder.class,SynchedEntityData.DataItem[].class);ctor.setAccessible(true);var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);var unsafe=uf.get(null);var allocate=u.getMethod("allocateInstance",Class.class);var storage=net.minecraft.world.entity.Entity.class.getDeclaredField("entityData");storage.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var holder=(Holder)allocate.invoke(unsafe,Holder.class);holder.out=out;var initial=f.getAsJsonArray("initial");var ids=new ArrayList<Integer>();int max=-1;for(var v:initial){int id=v.getAsJsonObject().get("id").getAsInt();ids.add(id);max=Math.max(max,id);}Collections.sort(ids);holder.items=new SynchedEntityData.DataItem<?>[max+1];for(int i=0;i<=max;i++)holder.items[i]=new SynchedEntityData.DataItem<Byte>(key(i),(byte)0);for(var v:initial){var e=v.getAsJsonObject();int id=e.get("id").getAsInt();holder.items[id]=new SynchedEntityData.DataItem<Byte>(key(id),single(e.get("value")));}holder.data=ctor.newInstance(holder,holder.items);storage.set(holder,holder.data);
            for(var v:f.getAsJsonArray("ops")){var op=v.getAsJsonObject();holder.op=op;holder.calls=0;try{switch(op.get("mode").getAsString()){
                case "flag":holder.flag(op.get("mask").getAsInt(),op.get("flag").getAsBoolean());break;
                case "set":holder.data.set(key(op.get("id").getAsInt()),single(op.get("value")),op.get("force").getAsBoolean());break;
                case "get":out.println("get "+bits(holder.data.get(key(op.get("id").getAsInt()))));break;
                case "assign":holder.data.assignValues(values(op.getAsJsonArray("values")));break;
                case "pack":out.println("packed "+values(holder.data.packDirty()));break;
                case "defaults":out.println("defaults "+values(holder.data.getNonDefaultValues()));break;
                default:throw new AssertionError();}out.println("result ok");}catch(Stop e){out.println("error observer");}catch(IndexOutOfBoundsException e){out.println("error id");}
                out.println("dirty "+holder.data.isDirty());for(int id:ids){var e=holder.items[id];out.println("state "+id+" "+bits((Byte)e.getValue())+" "+e.isDirty()+" "+e.isSetToDefault());}out.println("calls "+holder.calls);out.println("flags "+holder.isUsingItem()+" "+holder.getUsedItemHand().ordinal());
            }
        }}
    }
}
