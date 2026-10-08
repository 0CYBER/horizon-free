package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.sounds.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.damagesource.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.level.gamerules.*;
import net.minecraft.world.level.gameevent.*;
import net.minecraft.world.phys.*;
/** Runs registered native landing/bounce methods with controlled observable providers. */
public final class VanillaBlockLandingOracle {
    static final class Stop extends RuntimeException {}
    static Object unsafe;static Method allocate;
    static <T>T make(Class<T> type){try{return type.cast(allocate.invoke(unsafe,type));}catch(Exception e){throw new AssertionError(e);}}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static String pos(BlockPos p){return p.getX()+" "+p.getY()+" "+p.getZ();}
    static String bounds(String label,AABB b){return String.format("%s %016x %016x %016x %016x %016x %016x",label,Double.doubleToLongBits(b.minX),Double.doubleToLongBits(b.minY),Double.doubleToLongBits(b.minZ),Double.doubleToLongBits(b.maxX),Double.doubleToLongBits(b.maxY),Double.doubleToLongBits(b.maxZ));}
    static AABB box(JsonElement v){var a=v.getAsJsonArray();return new AABB(number(a.get(0)),number(a.get(1)),number(a.get(2)),number(a.get(3)),number(a.get(4)),number(a.get(5)));}
    static String sound(SoundEvent s){if(s==SoundEvents.HONEY_BLOCK_SLIDE)return "honey-slide";if(s==SoundEvents.HONEY_BLOCK_FALL)return "honey-fall";if(s==SoundEvents.TURTLE_EGG_BREAK)return "egg-break";if(s==SoundEvents.SHELF_MUSHROOM_BOUNCE)return "mushroom";if(s==SoundEvents.GENERIC_SMALL_FALL)return "small";if(s==SoundEvents.GENERIC_BIG_FALL)return "big";throw new AssertionError(s);}
    static final class Data {
        JsonObject f;PrintWriter out;int calls;BlockState state;Sources sources;Rules rules;Server server;RandomSource random;List<Occupant> occupants;
        boolean flag(String name){return f.get(name).getAsBoolean();}
        void query(String s){out.println(s);if(calls++==f.get("fail").getAsInt())throw new Stop();}
        boolean damage(double distance,float multiplier,DamageSource source){query(String.format("damage %016x %08x %s",Double.doubleToLongBits(distance),Float.floatToIntBits(multiplier),source==sources.fall?"fall":source==sources.stalagmite?"stalagmite":"unknown"));return flag("damage");}
        void entitySound(SoundEvent s,float volume,float pitch){query(String.format("entity-sound %s %08x %08x",sound(s),Float.floatToIntBits(volume),Float.floatToIntBits(pitch)));}
        boolean set(BlockPos p,BlockState next,int flags){query("set "+pos(p)+" "+Block.getId(next)+" "+flags);if(flag("write"))state=next;return flag("write");}
        boolean destroy(BlockPos p,boolean drops){query("destroy "+pos(p)+" "+drops);if(flag("write"))state=Blocks.AIR.defaultBlockState();return flag("write");}
        void event(Holder<GameEvent> event,BlockPos p,GameEvent.Context c){query("event "+(event==GameEvent.BLOCK_CHANGE?"change":event==GameEvent.BLOCK_DESTROY?"destroy":"unknown")+" "+pos(p)+" "+Block.getId(c.affectedState())+" "+(c.sourceEntity()!=null));}
        void blockSound(Entity entity,BlockPos p,SoundEvent sound,SoundSource source,float volume,float pitch){if(entity!=null||source!=SoundSource.BLOCKS)throw new AssertionError();query(String.format("block-sound %s %s %08x %08x",pos(p),sound(sound),Float.floatToIntBits(volume),Float.floatToIntBits(pitch)));}
        List<Entity> entities(Entity excluded,AABB bounds){if(excluded!=null)throw new AssertionError();query(bounds("entities",bounds));return new ArrayList<Entity>(occupants);}
    }
    static final class Sources extends DamageSources {
        Data d;DamageSource fall,stalagmite;Sources(){super(null);}
        @Override public DamageSource fall(){d.query("fall-source");return fall;}
        @Override public DamageSource stalagmite(){d.query("stalagmite-source");return stalagmite;}
    }
    static final class Rules extends GameRules {
        Data d;Rules(){super(net.minecraft.world.flag.FeatureFlagSet.of());}
        @SuppressWarnings("unchecked") @Override public <T>T get(GameRule<T> rule){if(rule!=GameRules.MOB_GRIEFING)throw new AssertionError();d.query("mob-griefing");return (T)Boolean.valueOf(d.flag("griefing"));}
    }
    static final class Server extends net.minecraft.server.MinecraftServer {
        @Override public int getMaxPlayers(){throw new UnsupportedOperationException();}
        Data d;Server(){super(null,null,null,null,Optional.empty(),null,null,null,null,false,null);}
        @Override public boolean isUnderSpawnProtection(net.minecraft.server.level.ServerLevel level,BlockPos p,Player player){d.query("protection "+pos(p));return d.flag("protected");}
        @Override public boolean initServer(){throw new UnsupportedOperationException();}
        @Override public net.minecraft.server.permissions.LevelBasedPermissionSet operatorUserPermissions(){throw new UnsupportedOperationException();}
        @Override public net.minecraft.server.permissions.PermissionSet getFunctionCompilationPermissions(){throw new UnsupportedOperationException();}
        @Override public boolean shouldRconBroadcast(){throw new UnsupportedOperationException();}
        @Override public net.minecraft.util.debugchart.SampleLogger getTickTimeLogger(){throw new UnsupportedOperationException();}
        @Override public boolean isTickTimeLoggingEnabled(){throw new UnsupportedOperationException();}
        @Override public net.minecraft.SystemReport fillServerSystemReport(net.minecraft.SystemReport p0){throw new UnsupportedOperationException();}
        @Override public boolean isDedicatedServer(){throw new UnsupportedOperationException();}
        @Override public int getRateLimitPacketsPerSecond(){throw new UnsupportedOperationException();}
        @Override public int getCommandSpamThresholdSeconds(){throw new UnsupportedOperationException();}
        @Override public int getChatSpamThresholdSeconds(){throw new UnsupportedOperationException();}
        @Override public boolean useNativeTransport(){throw new UnsupportedOperationException();}
        @Override public boolean isPublished(){throw new UnsupportedOperationException();}
        @Override public boolean shouldInformAdmins(){throw new UnsupportedOperationException();}
        @Override public boolean isSingleplayerOwner(net.minecraft.server.players.NameAndId p0){throw new UnsupportedOperationException();}
    }
    static final class ServerWorld extends net.minecraft.server.level.ServerLevel {
        Data d;ServerWorld(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public GameRules getGameRules(){d.query("rules");return d.rules;}

        @Override public boolean isClientSide(){d.query("client");return d.flag("client");}
        @Override public RandomSource getRandom(){d.query("random");return d.random;}
        @Override public DamageSources damageSources(){d.query("level-sources");return d.sources;}
        @Override public net.minecraft.server.MinecraftServer getServer(){d.query("server");return d.server;}
        @Override public void playSound(Entity e,BlockPos p,SoundEvent s,SoundSource source,float volume,float pitch){d.blockSound(e,p,s,source,volume,pitch);}
        @Override public void broadcastEntityEvent(Entity e,byte event){d.query("broadcast "+event);}
        @Override public boolean setBlock(BlockPos p,BlockState state,int flags){return d.set(p,state,flags);}
        @Override public boolean destroyBlock(BlockPos p,boolean drops){return d.destroy(p,drops);}
        @Override public void gameEvent(Holder<GameEvent> e,BlockPos p,GameEvent.Context c){d.event(e,p,c);}
        @Override public void levelEvent(int event,BlockPos p,int data){d.query("level-event "+event+" "+pos(p)+" "+data);}
        @Override public List<Entity> getEntities(Entity excluded,AABB bounds){return d.entities(excluded,bounds);}
    }
    static final class ClientWorld extends Level {
        Data d;ClientWorld(){super(null,null,null,null,false,false,0L,0);}

        @Override public boolean isClientSide(){d.query("client");return d.flag("client");}
        @Override public RandomSource getRandom(){d.query("random");return d.random;}
        @Override public DamageSources damageSources(){d.query("level-sources");return d.sources;}
        @Override public net.minecraft.server.MinecraftServer getServer(){d.query("server");return d.server;}
        @Override public void playSound(Entity e,BlockPos p,SoundEvent s,SoundSource source,float volume,float pitch){d.blockSound(e,p,s,source,volume,pitch);}
        @Override public void broadcastEntityEvent(Entity e,byte event){d.query("broadcast "+event);}
        @Override public boolean setBlock(BlockPos p,BlockState state,int flags){return d.set(p,state,flags);}
        @Override public boolean destroyBlock(BlockPos p,boolean drops){return d.destroy(p,drops);}
        @Override public void gameEvent(Holder<GameEvent> e,BlockPos p,GameEvent.Context c){d.event(e,p,c);}
        @Override public void levelEvent(int event,BlockPos p,int data){d.query("level-event "+event+" "+pos(p)+" "+data);}
        @Override public List<Entity> getEntities(Entity excluded,AABB bounds){return d.entities(excluded,bounds);}
@Override public net.minecraft.world.clock.ClockManager clockManager(){throw new UnsupportedOperationException();}
@Override public void destroyBlockProgress(int p0,net.minecraft.core.BlockPos p1,int p2){throw new UnsupportedOperationException();}
@Override public java.util.Collection dragonParts(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.attribute.EnvironmentAttributeSystem environmentAttributes(){throw new UnsupportedOperationException();}
@Override public void explode(net.minecraft.world.entity.Entity p0,net.minecraft.world.damagesource.DamageSource p1,net.minecraft.world.level.ExplosionDamageCalculator p2,double p3,double p4,double p5,float p6,boolean p7,net.minecraft.world.level.Level.ExplosionInteraction p8,net.minecraft.core.particles.ParticleOptions p9,net.minecraft.core.particles.ParticleOptions p10,net.minecraft.util.random.WeightedList p11,net.minecraft.core.Holder p12){throw new UnsupportedOperationException();}
@Override public void gameEvent(net.minecraft.core.Holder p0,net.minecraft.world.phys.Vec3 p1,net.minecraft.world.level.gameevent.GameEvent.Context p2){throw new UnsupportedOperationException();}
@Override public java.lang.String gatherChunkSourceStats(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.ticks.LevelTickAccess getBlockTicks(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.level.chunk.ChunkSource getChunkSource(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.level.entity.LevelEntityGetter getEntities(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.entity.Entity getEntity(int p0){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.ticks.LevelTickAccess getFluidTicks(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.level.saveddata.maps.MapItemSavedData getMapData(net.minecraft.world.level.saveddata.maps.MapId p0){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.level.storage.LevelData.RespawnData getRespawnData(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.scores.Scoreboard getScoreboard(){throw new UnsupportedOperationException();}
@Override public int getSeaLevel(){throw new UnsupportedOperationException();}
@Override public net.minecraft.core.Holder getUncachedNoiseBiome(int p0,int p1,int p2){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.level.border.WorldBorder getWorldBorder(){throw new UnsupportedOperationException();}
@Override public void levelEvent(net.minecraft.world.entity.Entity p0,int p1,net.minecraft.core.BlockPos p2,int p3){throw new UnsupportedOperationException();}
@Override public void playSeededSound(net.minecraft.world.entity.Entity p0,net.minecraft.world.entity.Entity p1,net.minecraft.core.Holder p2,net.minecraft.sounds.SoundSource p3,float p4,float p5,long p6){throw new UnsupportedOperationException();}
@Override public void playSeededSound(net.minecraft.world.entity.Entity p0,double p1,double p2,double p3,net.minecraft.core.Holder p4,net.minecraft.sounds.SoundSource p5,float p6,float p7,long p8){throw new UnsupportedOperationException();}
@Override public java.util.List players(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.item.crafting.RecipeAccess recipeAccess(){throw new UnsupportedOperationException();}
@Override public void sendBlockUpdated(net.minecraft.core.BlockPos p0,net.minecraft.world.level.block.state.BlockState p1,net.minecraft.world.level.block.state.BlockState p2,int p3){throw new UnsupportedOperationException();}
@Override public void setRespawnData(net.minecraft.world.level.storage.LevelData.RespawnData p0){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.TickRateManager tickRateManager(){throw new UnsupportedOperationException();}
    }
    static final class Other extends Marker {
        Data d;Other(){super(null,null);}
        @Override public boolean causeFallDamage(double distance,float multiplier,DamageSource source){return d.damage(distance,multiplier,source);}
        @Override public boolean isSuppressingBounce(){d.query("suppress");return d.flag("suppress");}
        @Override public DamageSources damageSources(){d.query("entity-sources");return d.sources;}
        @Override public void playSound(SoundEvent s,float volume,float pitch){d.entitySound(s,volume,pitch);}
    }
    static final class Living extends net.minecraft.world.entity.decoration.ArmorStand {
        Data d;Living(){super(null,null);}
        @Override public boolean causeFallDamage(double distance,float multiplier,DamageSource source){return d.damage(distance,multiplier,source);}
        @Override public boolean isSuppressingBounce(){d.query("suppress");return d.flag("suppress");}
        @Override public DamageSources damageSources(){d.query("entity-sources");return d.sources;}
        @Override public void playSound(SoundEvent s,float volume,float pitch){d.entitySound(s,volume,pitch);}
        @Override public Fallsounds getFallSounds(){d.query("fall-sounds");return new Fallsounds(SoundEvents.GENERIC_SMALL_FALL,SoundEvents.GENERIC_BIG_FALL);}
    }
    static final class Human extends Player {
        Data d;Human(){super(null,null);}
        @Override public boolean causeFallDamage(double distance,float multiplier,DamageSource source){return d.damage(distance,multiplier,source);}
        @Override public boolean isSuppressingBounce(){d.query("suppress");return d.flag("suppress");}
        @Override public DamageSources damageSources(){d.query("entity-sources");return d.sources;}
        @Override public void playSound(SoundEvent s,float volume,float pitch){d.entitySound(s,volume,pitch);}
        @Override public Fallsounds getFallSounds(){d.query("fall-sounds");return new Fallsounds(SoundEvents.GENERIC_SMALL_FALL,SoundEvents.GENERIC_BIG_FALL);}
        @Override public GameType gameMode(){return GameType.SURVIVAL;}
    }
    static final class Undead extends net.minecraft.world.entity.monster.zombie.Zombie {
        Data d;Undead(){super(null,null);}
        @Override public boolean causeFallDamage(double distance,float multiplier,DamageSource source){return d.damage(distance,multiplier,source);}
        @Override public boolean isSuppressingBounce(){d.query("suppress");return d.flag("suppress");}
        @Override public DamageSources damageSources(){d.query("entity-sources");return d.sources;}
        @Override public void playSound(SoundEvent s,float volume,float pitch){d.entitySound(s,volume,pitch);}
        @Override public Fallsounds getFallSounds(){d.query("fall-sounds");return new Fallsounds(SoundEvents.GENERIC_SMALL_FALL,SoundEvents.GENERIC_BIG_FALL);}
    }
    static final class Reptile extends net.minecraft.world.entity.animal.turtle.Turtle {
        Data d;Reptile(){super(null,null);}
        @Override public boolean causeFallDamage(double distance,float multiplier,DamageSource source){return d.damage(distance,multiplier,source);}
        @Override public boolean isSuppressingBounce(){d.query("suppress");return d.flag("suppress");}
        @Override public DamageSources damageSources(){d.query("entity-sources");return d.sources;}
        @Override public void playSound(SoundEvent s,float volume,float pitch){d.entitySound(s,volume,pitch);}
        @Override public Fallsounds getFallSounds(){d.query("fall-sounds");return new Fallsounds(SoundEvents.GENERIC_SMALL_FALL,SoundEvents.GENERIC_BIG_FALL);}
    }
    static final class Flying extends net.minecraft.world.entity.ambient.Bat {
        Data d;Flying(){super(null,null);}
        @Override public boolean causeFallDamage(double distance,float multiplier,DamageSource source){return d.damage(distance,multiplier,source);}
        @Override public boolean isSuppressingBounce(){d.query("suppress");return d.flag("suppress");}
        @Override public DamageSources damageSources(){d.query("entity-sources");return d.sources;}
        @Override public void playSound(SoundEvent s,float volume,float pitch){d.entitySound(s,volume,pitch);}
        @Override public Fallsounds getFallSounds(){d.query("fall-sounds");return new Fallsounds(SoundEvents.GENERIC_SMALL_FALL,SoundEvents.GENERIC_BIG_FALL);}
    }
    static final class Item extends net.minecraft.world.entity.item.ItemEntity {
        Data d;Item(){super(null,null);}
        @Override public boolean causeFallDamage(double distance,float multiplier,DamageSource source){return d.damage(distance,multiplier,source);}
        @Override public boolean isSuppressingBounce(){d.query("suppress");return d.flag("suppress");}
        @Override public DamageSources damageSources(){d.query("entity-sources");return d.sources;}
        @Override public void playSound(SoundEvent s,float volume,float pitch){d.entitySound(s,volume,pitch);}
    }
    static final class Tnt extends net.minecraft.world.entity.item.PrimedTnt {
        Data d;Tnt(){super(null,null);}
        @Override public boolean causeFallDamage(double distance,float multiplier,DamageSource source){return d.damage(distance,multiplier,source);}
        @Override public boolean isSuppressingBounce(){d.query("suppress");return d.flag("suppress");}
        @Override public DamageSources damageSources(){d.query("entity-sources");return d.sources;}
        @Override public void playSound(SoundEvent s,float volume,float pitch){d.entitySound(s,volume,pitch);}
    }
    static final class Occupant extends Marker {
        Data d;int id;Occupant(){super(null,null);}
        @Override public void teleportRelative(double x,double y,double z){d.query(String.format("teleport %d %016x %016x %016x",id,Double.doubleToLongBits(x),Double.doubleToLongBits(y),Double.doubleToLongBits(z)));setBoundingBox(getBoundingBox().move(x,y,z));if(d.flag("mutate")&&id+1<d.occupants.size()){var next=d.occupants.get(id+1);next.setBoundingBox(next.getBoundingBox().move(0,.125,0));}}
    }
    static void export(Path dir)throws Exception {
        VanillaCollisionOracle.main(new String[]{"generate",dir.toString()});VanillaMovementRegistryOracle.export(dir);
        var definitions=new JsonObject();var metadata=new JsonArray();var base=FarmlandBlock.class.getDeclaredField("baseBlock");base.setAccessible(true);var sound=BlockBehaviour.class.getDeclaredField("soundType");sound.setAccessible(true);
        var owners=Set.of(Block.class,HayBlock.class,HoneyBlock.class,SlimeBlock.class,PowderSnowBlock.class,PointedDripstoneBlock.class,FarmlandBlock.class,TurtleEggBlock.class);
        for(var block:BuiltInRegistries.BLOCK){var owner=block.getClass().getMethod("fallOn",Level.class,BlockState.class,BlockPos.class,Entity.class,double.class).getDeclaringClass();if(!owners.contains(owner))throw new AssertionError(owner);var bounce=block.getClass().getMethod("bounceOn",Level.class,BlockState.class,BlockPos.class,Entity.class,double.class).getDeclaringClass();if(bounce!=Block.class&&bounce!=ShelfMushroomBlock.class)throw new AssertionError(bounce);var entry=new JsonObject();entry.addProperty("fall",owner.getSimpleName());entry.addProperty("bounce",bounce==ShelfMushroomBlock.class);if(owner==FarmlandBlock.class)entry.addProperty("base",BuiltInRegistries.BLOCK.getKey((Block)base.get(block)).toString());if(owner==HoneyBlock.class){var type=(SoundType)sound.get(block);entry.addProperty("volume",String.format("%08x",Float.floatToIntBits(type.getVolume())));entry.addProperty("pitch",String.format("%08x",Float.floatToIntBits(type.getPitch())));if(type.getFallSound()!=SoundEvents.HONEY_BLOCK_FALL)throw new AssertionError();}definitions.add(BuiltInRegistries.BLOCK.getKey(block).toString(),entry);}
        for(var state:Block.BLOCK_STATE_REGISTRY){var row=new JsonObject();row.addProperty("id",Block.getId(state));row.addProperty("name",BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString());metadata.add(row);}
        Files.writeString(dir.resolve("landing-definitions.json"),definitions.toString());Files.writeString(dir.resolve("landing-states.json"),metadata.toString());
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);if(args[0].equals("export")){export(dir);return;}
        var allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);unsafe=field.get(null);allocate=allocator.getMethod("allocateInstance",Class.class);var dimensions=Entity.class.getDeclaredField("dimensions");dimensions.setAccessible(true);
        var kinds=new Class<?>[]{Other.class,Living.class,Human.class,Undead.class,Reptile.class,Flying.class,Item.class,Tnt.class};
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){
            var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.state=Block.stateById(f.get("state").getAsInt());d.sources=make(Sources.class);d.sources.d=d;d.sources.fall=make(DamageSource.class);d.sources.stalagmite=make(DamageSource.class);d.rules=make(Rules.class);d.rules.d=d;d.server=make(Server.class);d.server.d=d;
            d.random=(RandomSource)Proxy.newProxyInstance(RandomSource.class.getClassLoader(),new Class<?>[]{RandomSource.class},(proxy,method,values)->{if(method.getName().equals("nextFloat")){d.query("float");return single(f.get("random_float"));}if(method.getName().equals("nextInt")&&values.length==1){d.query("int "+values[0]);return f.get("random_int").getAsInt();}throw new AssertionError(method);});
            Level world;if(d.flag("server")){var w=make(ServerWorld.class);w.d=d;world=w;}else{var w=make(ClientWorld.class);w.d=d;world=w;}
            var kind=kinds[f.get("kind").getAsInt()];var entity=(Entity)make(kind);var dataField=kind.getDeclaredField("d");dataField.set(entity,d);dimensions.set(entity,EntityDimensions.scalable(single(f.get("width")),single(f.get("height"))));d.occupants=new ArrayList<>();for(var box:f.getAsJsonArray("entities")){var e=make(Occupant.class);e.d=d;e.id=d.occupants.size();e.setBoundingBox(box(box));d.occupants.add(e);}
            var p=f.getAsJsonArray("position");var position=new BlockPos(p.get(0).getAsInt(),p.get(1).getAsInt(),p.get(2).getAsInt());var state=d.state;
            try{if(d.flag("bounce"))state.getBlock().bounceOn(world,state,position,entity,number(f.get("distance")));else state.getBlock().fallOn(world,state,position,entity,number(f.get("distance")));out.println("ok");}catch(Stop e){out.println("error");}
            out.println("state "+Block.getId(d.state));for(var e:d.occupants)out.println(bounds("final "+e.id,e.getBoundingBox()));out.println("queries "+d.calls);
        }}
    }
}
