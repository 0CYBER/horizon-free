package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.component.*;import net.minecraft.core.registries.*;import net.minecraft.world.item.*;import net.minecraft.world.item.component.*;import net.minecraft.resources.*;import com.mojang.serialization.JsonOps;
/** Bound built-in prototypes, queried through actual registered item methods. */
public final class VanillaItemUseCatalogOracle {
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static KineticWeapon kinetic(){return new KineticWeapon(0,0,Optional.empty(),Optional.empty(),Optional.empty(),0,0,Optional.empty(),Optional.empty());}
    static BlocksAttacks blocking(float delay){return new BlocksAttacks(delay,1,List.of(),new BlocksAttacks.ItemDamageFunction(0,0,0),Optional.empty(),Optional.empty(),Optional.empty());}
    static void snapshot(PrintWriter out,ItemStack s){out.println("stack "+BuiltInRegistries.ITEM.getId(s.getItem())+" "+s.getCount()+" "+s.getMaxStackSize()+" "+s.getMaxDamage()+" "+s.getDamageValue()+" "+s.isEmpty()+" "+s.isDamageableItem()+" "+s.isDamaged()+" "+s.isBroken()+" "+s.nextDamageWillBreak()+" "+s.isStackable()+" "+s.getUseDuration(null)+" "+s.useOnRelease());}
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var lookup=net.minecraft.data.registries.VanillaRegistries.createWorldLookup();for(var p:BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(lookup))p.apply();Path dir=Path.of(args[1]);
        if(args[0].equals("export")){var root=new JsonObject();root.addProperty("schema",1);root.addProperty("DataVersion",5023);var instruments=new JsonObject();lookup.lookupOrThrow(Registries.INSTRUMENT).listElements().forEach(h->instruments.addProperty(h.key().identifier().toString(),bits(h.value().useDuration())));root.add("instruments",instruments);var rows=new JsonArray();var ops=RegistryOps.create(JsonOps.INSTANCE,lookup);for(var item:BuiltInRegistries.ITEM){var row=new JsonObject();row.addProperty("name",BuiltInRegistries.ITEM.getKey(item).toString());row.addProperty("id",BuiltInRegistries.ITEM.getId(item));row.addProperty("duration_impl",item.getClass().getMethod("getUseDuration",ItemStack.class,net.minecraft.world.entity.LivingEntity.class).getDeclaringClass().getSimpleName());row.addProperty("release_impl",item.getClass().getMethod("useOnRelease",ItemStack.class).getDeclaringClass().getSimpleName());row.add("components",DataComponentMap.CODEC.encodeStart(ops,item.builtInRegistryHolder().components()).getOrThrow());rows.add(row);}root.add("items",rows);Files.writeString(dir.resolve("catalog.json"),root.toString());return;}
        var sound=net.minecraft.sounds.SoundEvents.GENERIC_EAT;
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var item=BuiltInRegistries.ITEM.getValue(Identifier.parse(f.get("name").getAsString()));var stack=new ItemStack(item,f.get("count").getAsInt());snapshot(out,stack);for(var ov:f.getAsJsonArray("ops")){var op=ov.getAsJsonObject();var v=op.get("value");switch(op.get("kind").getAsString()){
        case "count":stack.setCount(v.getAsInt());break;
        case "set-damage":stack.setDamageValue(v.getAsInt());break;
        case "damage":stack.set(DataComponents.DAMAGE,v.isJsonNull()?null:v.getAsInt());break;
        case "max-damage":stack.set(DataComponents.MAX_DAMAGE,v.isJsonNull()?null:v.getAsInt());break;
        case "max-stack":stack.set(DataComponents.MAX_STACK_SIZE,v.isJsonNull()?null:v.getAsInt());break;
        case "unbreakable":stack.set(DataComponents.UNBREAKABLE,v.getAsBoolean()?net.minecraft.util.Unit.INSTANCE:null);break;
        case "consume":stack.set(DataComponents.CONSUMABLE,v.isJsonNull()?null:new Consumable(single(v),ItemUseAnimation.EAT,sound,false,List.of()));break;
        case "blocking":stack.set(DataComponents.BLOCKS_ATTACKS,v.isJsonNull()?null:blocking(single(v)));break;
        case "kinetic":stack.set(DataComponents.KINETIC_WEAPON,v.getAsBoolean()?kinetic():null);break;
        case "instrument":stack.set(DataComponents.INSTRUMENT,v.isJsonNull()?null:new InstrumentComponent(Holder.direct(new Instrument(sound,single(v),1,0,net.minecraft.network.chat.Component.empty()))));break;
        default:throw new AssertionError();}snapshot(out,stack);}}}
    }
}
