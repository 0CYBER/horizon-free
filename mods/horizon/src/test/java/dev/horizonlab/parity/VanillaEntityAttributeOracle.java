package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.registries.*;
import net.minecraft.resources.Identifier;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.ai.attributes.*;
/** Native lazy entity attributes; set membership is observed in registry order. */
public final class VanillaEntityAttributeOracle {
    static String bits(double v){return String.format("%016x",Double.doubleToLongBits(v));}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
    static List<Holder<Attribute>> attributes=new ArrayList<>();
    static JsonObject export()throws Exception {
        var root=new JsonObject();root.addProperty("data_version",net.minecraft.SharedConstants.getCurrentVersion().dataVersion().version());
        var defs=new JsonArray();
        for(var a:attributes){var v=a.value();var d=new JsonObject();d.addProperty("id",a.getRegisteredName());d.addProperty("default",bits(v.getDefaultValue()));d.addProperty("syncable",v.isClientSyncable());if(v instanceof RangedAttribute r){d.addProperty("min",bits(r.getMinValue()));d.addProperty("max",bits(r.getMaxValue()));}defs.add(d);}
        root.add("attributes",defs);var entities=new JsonObject();
        for(var e:BuiltInRegistries.ENTITY_TYPE){if(!DefaultAttributes.hasSupplier(e))continue;var supplier=supplier(e);var bases=new JsonObject();for(var a:attributes)if(supplier.hasAttribute(a)){var instance=supplier.createInstance(v->{},a);if(!instance.getModifiers().isEmpty())throw new AssertionError("default modifiers require export support");bases.addProperty(a.getRegisteredName(),bits(supplier.getBaseValue(a)));}entities.add(BuiltInRegistries.ENTITY_TYPE.getKey(e).toString(),bases);}
        root.add("entities",entities);return root;
    }
    @SuppressWarnings("unchecked") static AttributeSupplier supplier(EntityType<?> e){return DefaultAttributes.getSupplier((EntityType<? extends LivingEntity>)e);}
    @SuppressWarnings("unchecked") public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        for(var a:BuiltInRegistries.ATTRIBUTE)attributes.add(BuiltInRegistries.ATTRIBUTE.wrapAsHolder(a));
        Path dir=Path.of(args[1]);if(args[0].equals("export")){Files.writeString(dir.resolve("catalog.json"),export().toString());return;}
        var storage=field(AttributeMap.class,"attributes");var dirty=field(AttributeInstance.class,"dirty");var cached=field(AttributeInstance.class,"cachedValue");
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var entity=BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(f.get("entity").getAsString()));var map=new AttributeMap(supplier(entity));var instances=(Map<Holder<Attribute>,AttributeInstance>)storage.get(map);
            for(var ov:f.getAsJsonArray("ops")){var op=ov.getAsJsonObject();var a=attributes.get(op.get("attribute").getAsInt());String mode=op.get("mode").getAsString();var id=Identifier.parse(op.get("id").getAsString());
                try{switch(mode){
                    case "value":out.println("value "+bits(map.getValue(a)));break;
                    case "base":out.println("base "+bits(map.getBaseValue(a)));break;
                    case "has":out.println("has "+map.hasAttribute(a));break;
                    case "modifier":out.println("modifier "+bits(map.getModifierValue(a,id)));break;
                    case "has_modifier":out.println("has_modifier "+map.hasModifier(a,id));break;
                    case "ensure":out.println("ensure "+(map.getInstance(a)!=null));break;
                    case "reset":out.println("reset "+map.resetBaseValue(a));break;
                    case "ack_update":map.getAttributesToUpdate().remove(instances.get(a));break;
                    case "ack_sync":map.getAttributesToSync().remove(instances.get(a));break;
                    default:{var instance=map.getInstance(a);if(instance==null){out.println("absent");break;}var modifier=new AttributeModifier(id,number(op.get("amount")),AttributeModifier.Operation.values()[op.get("operation").getAsInt()]);switch(mode){
                        case "set":instance.setBaseValue(number(op.get("base")));break;
                        case "add":instance.addTransientModifier(modifier);break;
                        case "update":instance.addOrUpdateTransientModifier(modifier);break;
                        case "permanent":instance.addPermanentModifier(modifier);break;
                        case "remove":out.println("removed "+instance.removeModifier(id));break;
                        case "clear":instance.removeModifiers();break;
                        default:throw new AssertionError(mode);
                    }}
                }out.println("ok");}catch(NullPointerException e){out.println("error instance_modifier");}catch(IllegalArgumentException e){out.println("error "+(mode.equals("modifier")?(map.hasAttribute(a)?"modifier":"attribute"):(mode.equals("value")||mode.equals("base")?"attribute":"duplicate")));}
                for(int i=0;i<attributes.size();i++){var key=attributes.get(i);var instance=instances.get(key);if(instance!=null)out.println("state "+i+" "+bits(instance.getBaseValue())+" "+dirty.getBoolean(instance)+" "+bits(cached.getDouble(instance))+" "+map.getAttributesToUpdate().contains(instance)+" "+map.getAttributesToSync().contains(instance)+" "+instance.getModifiers().size()+" "+instance.getPermanentModifiers().size());}out.println("end");
            }
        }}
    }
}
