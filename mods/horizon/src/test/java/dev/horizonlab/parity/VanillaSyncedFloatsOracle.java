package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.nio.file.*;import java.util.*;
import net.minecraft.network.syncher.*;
/** Native multi-float dirty/default lists, assignment and reentrant callbacks. */
public final class VanillaSyncedFloatsOracle {
    static final class Stop extends RuntimeException {}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static String values(List<SynchedEntityData.DataValue<?>> list){if(list==null)return "none";var s=new StringJoiner(",");for(var v:list)s.add(v.id()+":"+bits((Float)v.value()));return s.length()==0?"empty":s.toString();}
    static EntityDataAccessor<Float> key(int id){return new EntityDataAccessor<Float>(id,EntityDataSerializers.FLOAT);}
    static List<SynchedEntityData.DataValue<?>> values(JsonArray values){var result=new ArrayList<SynchedEntityData.DataValue<?>>();for(var x:values){var v=x.getAsJsonObject();result.add(new SynchedEntityData.DataValue<Float>(v.get("id").getAsInt(),EntityDataSerializers.FLOAT,single(v.get("value"))));}return result;}
    static final class Holder implements SyncedDataHolder {
        SynchedEntityData data;SynchedEntityData.DataItem<?>[] items;JsonObject op;PrintWriter out;int calls;boolean nested;
        void event(String s){out.println(s);if(calls++==op.get("fail").getAsInt())throw new Stop();}
        @Override public void onSyncedDataUpdated(EntityDataAccessor<?> key){int id=key.id();event("updated "+id+" "+bits((Float)items[id].getValue())+" "+data.isDirty()+" "+items[id].isDirty());if(nested)return;nested=true;try{switch(op.get("hook").getAsString()){
            case "none":break;case "pack":out.println("callback-pack "+values(data.packDirty()));break;
            case "set":data.set(key(op.get("other").getAsInt()),single(op.get("other_value")),op.get("other_force").getAsBoolean());break;
            case "assign":data.assignValues(List.of(new SynchedEntityData.DataValue<Float>(op.get("other").getAsInt(),EntityDataSerializers.FLOAT,single(op.get("other_value")))));break;
            default:throw new AssertionError();}}finally{nested=false;}}
        @Override public void onSyncedDataUpdated(List<SynchedEntityData.DataValue<?>> values){event("assigned "+values(values));}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var ctor=SynchedEntityData.class.getDeclaredConstructor(SyncedDataHolder.class,SynchedEntityData.DataItem[].class);ctor.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var holder=new Holder();holder.out=out;var initial=f.getAsJsonArray("initial");var ids=new ArrayList<Integer>();int max=-1;for(var v:initial){int id=v.getAsJsonObject().get("id").getAsInt();ids.add(id);max=Math.max(max,id);}Collections.sort(ids);holder.items=new SynchedEntityData.DataItem<?>[max+1];for(int i=0;i<=max;i++)holder.items[i]=new SynchedEntityData.DataItem<Float>(key(i),0f);for(var v:initial){var e=v.getAsJsonObject();int id=e.get("id").getAsInt();holder.items[id]=new SynchedEntityData.DataItem<Float>(key(id),single(e.get("value")));}holder.data=ctor.newInstance(holder,holder.items);
            for(var v:f.getAsJsonArray("ops")){var op=v.getAsJsonObject();holder.op=op;holder.calls=0;try{switch(op.get("mode").getAsString()){
                case "set":holder.data.set(key(op.get("id").getAsInt()),single(op.get("value")),op.get("force").getAsBoolean());break;
                case "get":out.println("get "+bits(holder.data.get(key(op.get("id").getAsInt()))));break;
                case "assign":holder.data.assignValues(values(op.getAsJsonArray("values")));break;
                case "pack":out.println("packed "+values(holder.data.packDirty()));break;
                case "defaults":out.println("defaults "+values(holder.data.getNonDefaultValues()));break;
                default:throw new AssertionError();}out.println("result ok");}catch(Stop e){out.println("error observer");}catch(IndexOutOfBoundsException e){out.println("error id");}
                out.println("dirty "+holder.data.isDirty());for(int id:ids){var e=holder.items[id];out.println("state "+id+" "+bits((Float)e.getValue())+" "+e.isDirty()+" "+e.isSetToDefault());}out.println("calls "+holder.calls);
            }
        }}
    }
}
