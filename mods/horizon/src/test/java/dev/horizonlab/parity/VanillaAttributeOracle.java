package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.registries.*;import net.minecraft.resources.Identifier;import net.minecraft.world.entity.ai.attributes.*;
/** Actual attribute modifier storage, evaluation order and dirty callbacks. */
public final class VanillaAttributeOracle {
    static final class Stop extends RuntimeException {}
    static Field dirty,cached,operations;
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static String bits(double v){return String.format("%016x",Double.doubleToLongBits(v));}
    static final class Basic extends Attribute {Basic(double value){super("horizon.fixture",value);}}
    static final class Data {JsonObject op;PrintWriter out;int calls;AttributeInstance[] instances;AttributeModifier[] pool;
        int identity(AttributeModifier m){if(m==null)return -1;for(int i=0;i<pool.length;i++)if(pool[i]==m)return i;throw new AssertionError("unknown modifier reference");}
        String refs(Collection<AttributeModifier> values){var result=new StringJoiner(",");for(var m:values)result.add(Integer.toString(identity(m)));return result.length()==0?"-":result.toString();}
        void changed(AttributeInstance a){try{int id=instances[0]==a?0:1;out.println("dirty "+id+" "+bits(a.getBaseValue())+" "+dirty.getBoolean(a)+" "+bits(cached.getDouble(a))+" "+a.getModifiers().size()+" "+a.getPermanentModifiers().size());if(calls++==op.get("fail").getAsInt())throw new Stop();if(op.get("read").getAsBoolean())out.println("observed "+bits(a.getValue()));}catch(IllegalAccessException e){throw new AssertionError(e);}}
        @SuppressWarnings("unchecked") void state(int id)throws Exception{var a=instances[id];out.println("state "+id+" "+bits(a.getBaseValue())+" "+dirty.getBoolean(a)+" "+bits(cached.getDouble(a)));out.println("all "+refs(a.getModifiers()));out.println("permanent "+refs(a.getPermanentModifiers()));var map=(Map<AttributeModifier.Operation,Map<Identifier,AttributeModifier>>)operations.get(a);for(var operation:AttributeModifier.Operation.values()){var bucket=map.get(operation);out.println("operation "+operation.ordinal()+" "+(bucket==null?"-":refs(bucket.values())));}}
    }
    static Field field(Class<?> c,String name)throws Exception{var f=c.getDeclaredField(name);f.setAccessible(true);return f;}
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);if(args[0].equals("export")){var data=new JsonArray();for(var a:BuiltInRegistries.ATTRIBUTE){var f=new JsonObject();f.addProperty("id",BuiltInRegistries.ATTRIBUTE.getKey(a).toString());f.addProperty("default",bits(a.getDefaultValue()));if(a instanceof RangedAttribute r){f.addProperty("min",bits(r.getMinValue()));f.addProperty("max",bits(r.getMaxValue()));}data.add(f);}Files.writeString(dir.resolve("attributes.json"),data.toString());return;}
        dirty=field(AttributeInstance.class,"dirty");cached=field(AttributeInstance.class,"cachedValue");operations=field(AttributeInstance.class,"modifiersByOperation");
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.out=out;var defs=f.getAsJsonArray("definitions");d.instances=new AttributeInstance[2];boolean invalid=false;for(int i=0;i<2;i++){var def=defs.get(i).getAsJsonObject();try{Attribute a=def.has("min")?new RangedAttribute("horizon.fixture",number(def.get("default")),number(def.get("min")),number(def.get("max"))):new Basic(number(def.get("default")));d.instances[i]=new AttributeInstance(Holder.direct(a),d::changed);}catch(IllegalArgumentException e){invalid=true;break;}}if(invalid){out.println("definition error");continue;}
            var mods=f.getAsJsonArray("modifiers");d.pool=new AttributeModifier[mods.size()];for(int i=0;i<mods.size();i++){var m=mods.get(i).getAsJsonObject();d.pool[i]=new AttributeModifier(Identifier.parse(m.get("id").getAsString()),number(m.get("amount")),AttributeModifier.Operation.values()[m.get("operation").getAsInt()]);}
            AttributeInstance.Packed packed=null;
            for(var v:f.getAsJsonArray("ops")){var op=v.getAsJsonObject();d.op=op;d.calls=0;var a=d.instances[op.get("target").getAsInt()];try{switch(op.get("mode").getAsString()){
                case "base":a.setBaseValue(number(op.get("base")));break;
                case "add":a.addTransientModifier(d.pool[op.get("modifier").getAsInt()]);break;
                case "update":a.addOrUpdateTransientModifier(d.pool[op.get("modifier").getAsInt()]);break;
                case "permanent":a.addPermanentModifier(d.pool[op.get("modifier").getAsInt()]);break;
                case "replace":a.addOrReplacePermanentModifier(d.pool[op.get("modifier").getAsInt()]);break;
                case "remove":out.println("removed "+a.removeModifier(d.pool[op.get("modifier").getAsInt()].id()));break;
                case "clear":a.removeModifiers();break;
                case "copy":a.replaceFrom(d.instances[op.get("source").getAsInt()]);break;
                case "pack":packed=a.pack();out.println("packed "+bits(packed.baseValue())+" "+d.refs(packed.modifiers()));break;
                case "unpack":a.apply(Objects.requireNonNull(packed));break;
                case "apply":{var values=new ArrayList<AttributeModifier>();for(var m:op.getAsJsonArray("modifiers"))values.add(d.pool[m.getAsInt()]);a.apply(new AttributeInstance.Packed(d.instances[1-op.get("target").getAsInt()].getAttribute(),number(op.get("base")),values));break;}
                case "many":{var values=new ArrayList<AttributeModifier>();for(var m:op.getAsJsonArray("modifiers"))values.add(d.pool[m.getAsInt()]);a.addPermanentModifiers(values);break;}
                case "value":out.println("value "+bits(a.getValue()));break;
                default:throw new AssertionError();}out.println("result ok");}catch(Stop e){out.println("error observer");}catch(IllegalArgumentException e){out.println("error duplicate");}
                d.state(0);d.state(1);out.println("calls "+d.calls);
            }
        }}
    }
}
