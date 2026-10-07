package dev.horizonlab.parity;
import com.google.gson.*;import com.mojang.serialization.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.registries.*;import net.minecraft.resources.*;import net.minecraft.tags.*;import net.minecraft.world.entity.*;import net.minecraft.world.entity.player.*;import net.minecraft.world.damagesource.*;import net.minecraft.world.item.*;import net.minecraft.world.phys.Vec3;
/** Native decoded damage definitions, resolved tags and source query behavior. */
public final class VanillaDamageSourceOracle {
    static final class Stop extends RuntimeException {}
    static Object unsafe;static Method allocate;
    static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static Vec3 position(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static String vec(Vec3 v){return v==null?"none":String.format("%016x %016x %016x",Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z));}
    static final class Data {JsonObject f;PrintWriter out;int calls;boolean changed;Entity[] entities;
        JsonObject state(){return f.getAsJsonObject(changed?"after":"initial");}void event(String s){out.println(s);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt())changed=true;}
        int id(Entity e){for(int i=0;i<entities.length;i++)if(entities[i]==e)return i;throw new AssertionError();}}
    static final class Source extends DamageSource {Data d;
        Source(Holder<DamageType> t,Entity direct,Entity causing){super(t,direct,causing);}Source(Holder<DamageType> t,Entity entity){super(t,entity);}Source(Holder<DamageType> t,Vec3 p){super(t,p);}Source(Holder<DamageType> t){super(t);}
        @Override public Entity getEntity(){d.event("entity");return d.f.get("virtual").getAsBoolean()?d.entities[d.state().get("virtual_entity").getAsInt()]:super.getEntity();}
        Entity stored(){return super.getEntity();}}
    static final class Other extends net.minecraft.world.entity.Marker {Other(){super(null,null);}
Data d;int id;
        @Override public ItemStack getWeaponItem(){d.event("weapon "+id);return d.state().getAsJsonArray("weapons").get(id).getAsBoolean()?ItemStack.EMPTY:null;}
        @Override public Vec3 position(){d.event("position "+id);return VanillaDamageSourceOracle.position(d.state().getAsJsonArray("positions").get(id));}
    }
    static final class Living extends net.minecraft.world.entity.decoration.ArmorStand {Living(){super(null,null);}
Data d;int id;
        @Override public ItemStack getWeaponItem(){d.event("weapon "+id);return d.state().getAsJsonArray("weapons").get(id).getAsBoolean()?ItemStack.EMPTY:null;}
        @Override public Vec3 position(){d.event("position "+id);return VanillaDamageSourceOracle.position(d.state().getAsJsonArray("positions").get(id));}
    }
    static final class Human extends Player {Human(){super(null,null);}
Data d;int id;
        @Override public ItemStack getWeaponItem(){d.event("weapon "+id);return d.state().getAsJsonArray("weapons").get(id).getAsBoolean()?ItemStack.EMPTY:null;}
        @Override public Vec3 position(){d.event("position "+id);return VanillaDamageSourceOracle.position(d.state().getAsJsonArray("positions").get(id));}
        @Override public net.minecraft.world.level.GameType gameMode(){return net.minecraft.world.level.GameType.SURVIVAL;}
        @Override public Abilities getAbilities(){d.event("abilities "+id);var a=new Abilities();a.instabuild=d.state().getAsJsonArray("creative").get(id).getAsBoolean();return a;}
    }
    static MappedRegistry<DamageType> registry(Path dir)throws Exception {
        var registry=new MappedRegistry<DamageType>(Registries.DAMAGE_TYPE,Lifecycle.stable());var resources=JsonParser.parseString(Files.readString(dir.resolve("types.json"))).getAsJsonObject();for(var e:resources.entrySet())registry.register(ResourceKey.create(Registries.DAMAGE_TYPE,Identifier.parse(e.getKey())),DamageType.DIRECT_CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(e.getValue().getAsString())).getOrThrow(),RegistrationInfo.BUILT_IN);registry.bindAllTagsToEmpty();registry.freeze();
        var loader=new TagLoader<Holder<DamageType>>((id,required)->registry.get(id).map(v->(Holder<DamageType>)v),"tags/damage_type");var definitions=new HashMap<Identifier,List<TagLoader.EntryWithSource>>();for(var e:JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject().entrySet()){var file=TagFile.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(e.getValue().getAsString())).getOrThrow();var entries=new ArrayList<TagLoader.EntryWithSource>();for(var entry:file.entries())entries.add(new TagLoader.EntryWithSource(entry,"vanilla"));definitions.put(Identifier.parse(e.getKey()),entries);}var tags=new HashMap<TagKey<DamageType>,List<Holder<DamageType>>>();loader.build(definitions).forEach((id,values)->tags.put(TagKey.create(Registries.DAMAGE_TYPE,id),List.copyOf(values)));if(tags.size()!=definitions.size())throw new AssertionError("missing tags");registry.prepareTagReload(new TagLoader.LoadResult<>(Registries.DAMAGE_TYPE,tags)).apply();return registry;
    }
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);var registry=registry(dir);var names=new TreeSet<Identifier>(Comparator.comparing(Identifier::toString));names.addAll(registry.keySet());var tagNames=new TreeSet<String>();tagNames.addAll(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject().keySet());
        if(args[0].equals("export")){var root=new JsonObject();root.addProperty("data_version",net.minecraft.SharedConstants.getCurrentVersion().dataVersion().version());var types=new JsonArray();for(var id:names){var t=registry.getValue(id);var d=new JsonObject();d.addProperty("id",id.toString());d.addProperty("message_id",t.msgId());d.addProperty("exhaustion",bits(t.exhaustion()));d.addProperty("scaling",t.scaling().getSerializedName());d.addProperty("effects",t.effects().getSerializedName());d.addProperty("death_message_type",t.deathMessageType().getSerializedName());types.add(d);}root.add("types",types);Files.writeString(dir.resolve("catalog.json"),root.toString());return;}
        var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);unsafe=uf.get(null);allocate=u.getMethod("allocateInstance",Class.class);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var id:names){var holder=registry.get(id).orElseThrow();var source=new DamageSource(holder);var t=source.type();out.println("definition "+id+" "+source.getMsgId()+" "+bits(source.getFoodExhaustion())+" "+t.scaling().getSerializedName()+" "+t.effects().getSerializedName()+" "+t.deathMessageType().getSerializedName());for(var tag:tagNames)out.println("tag "+tag+" "+source.is(TagKey.create(Registries.DAMAGE_TYPE,Identifier.parse(tag))));}
            for(var line:(Iterable<String>)lines::iterator){var d=new Data();d.f=JsonParser.parseString(line).getAsJsonObject();d.out=out;var other=make(Other.class);other.d=d;other.id=1;var living=make(Living.class);living.d=d;living.id=2;var a=make(Human.class);a.d=d;a.id=3;var b=make(Human.class);b.d=d;b.id=4;d.entities=new Entity[]{null,other,living,a,b};var f=d.f;var holder=registry.get(Identifier.parse(f.get("type").getAsString())).orElseThrow();int direct=f.get("direct").getAsInt(),causing=f.get("causing").getAsInt();var source=switch(f.get("constructor").getAsInt()){case 0->new Source(holder,d.entities[direct],d.entities[causing]);case 1->new Source(holder,d.entities[direct]);case 2->new Source(holder,position(f.get("position")));case 3->new Source(holder);default->throw new AssertionError();};source.d=d;
                try{out.println("direct "+source.isDirect()+" "+d.id(source.getDirectEntity())+" "+d.id(source.stored()));out.println("scales "+source.scalesWithDifficulty());out.println("weapon-result "+(source.getWeaponItem()!=null));out.println("creative "+source.isCreativePlayer());out.println("position-result "+vec(source.getSourcePosition()));out.println("raw "+vec(source.sourcePositionRaw()));out.println("type "+source.is(ResourceKey.create(Registries.DAMAGE_TYPE,Identifier.parse(f.get("check_type").getAsString()))));out.println("result ok");}catch(Stop e){out.println("result error");}out.println("queries "+d.calls+" "+d.changed);
            }
        }
    }
}
