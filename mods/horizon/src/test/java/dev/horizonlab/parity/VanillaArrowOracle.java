package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.sounds.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.projectile.arrow.AbstractArrow;
import net.minecraft.world.entity.projectile.arrow.Arrow;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.border.WorldBorder;
import net.minecraft.world.level.gameevent.GameEvent;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;
import static dev.horizonlab.parity.VanillaRegistryCollisionScanOracle.*;

/** Real arrows (`Arrow`, through `AbstractArrow.tick`) over an independently
 * built copy of the development client's resident scene: flight, water inertia
 * and bubbles, fluid currents and splashes, bubble columns, sticking in blocks,
 * falling out when the block goes, and despawning. Burning arrows: lava and fire along
 * the path, water putting them out, and the blocks they light (TNT, campfires,
 * candles). Pickup: the INTANGIBLE_PROJECTILE ammo the constructor sees, `setOwner`, and
 * `playerTouch` by a player (creative or not, with room in the inventory or not).
 * Entities: a box in the way that takes the arrow's damage or refuses it (`onHitEntity`'s
 * REVERSE deflection). No rain; sounds, particles, game events, the damage dealt and
 * dropped pickups are recorded in order. */
public final class VanillaArrowOracle {
    static final class Border extends WorldBorder {
        @Override public boolean isInsideCloseToBorder(Entity e,AABB b){return false;}
    }
    static String bits(Vec3 v){return String.format("%016x,%016x,%016x",Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z));}
    static String sound(SoundEvent s){
        if(s==SoundEvents.ARROW_HIT)return "hit";if(s==SoundEvents.TNT_PRIMED)return "tnt_primed";if(s==SoundEvents.GENERIC_SPLASH)return "splash";if(s==SoundEvents.GENERIC_EXTINGUISH_FIRE)return "extinguish";
        throw new AssertionError("unexpected sound "+s);
    }
    static String particle(net.minecraft.core.particles.ParticleOptions p){
        if(p==net.minecraft.core.particles.ParticleTypes.BUBBLE)return "bubble";if(p==net.minecraft.core.particles.ParticleTypes.CRIT)return "crit";
        if(p==net.minecraft.core.particles.ParticleTypes.SPLASH)return "splash";
        throw new AssertionError("unexpected particle "+p);
    }
    /** Lava, fire and burning hurt arrows not at all (Projectile.hurtServer only
     * marks them hurt): untagged stand-in sources. */
    static final class Sources extends net.minecraft.world.damagesource.DamageSources {
        net.minecraft.world.damagesource.DamageSource any;Sources(){super(null);}
        @Override public net.minecraft.world.damagesource.DamageSource lava(){return any;}
        @Override public net.minecraft.world.damagesource.DamageSource inFire(){return any;}
        @Override public net.minecraft.world.damagesource.DamageSource onFire(){return any;}
        @Override public net.minecraft.world.damagesource.DamageSource arrow(AbstractArrow arrow,Entity owner){return any;}
    }
    /** The entity in the way (allocated without a constructor): a box at a position that
     * takes the arrow's damage or refuses it (a raised shield), recording what it was dealt. */
    static final class Target extends Entity {
        boolean takes;List<String> events;
        Target(){super(null,null);}
        @Override protected void defineSynchedData(net.minecraft.network.syncher.SynchedEntityData.Builder entityData){}
        @Override protected void readAdditionalSaveData(net.minecraft.world.level.storage.ValueInput input){}
        @Override protected void addAdditionalSaveData(net.minecraft.world.level.storage.ValueOutput output){}
        @Override public boolean hurtServer(net.minecraft.server.level.ServerLevel level,net.minecraft.world.damagesource.DamageSource source,float damage){
            events.add(String.format("hurt %08x",Float.floatToIntBits(damage)));return takes;
        }
        @Override public boolean isPickable(){return true;}
        // Untagged: neither DEFLECTS_PROJECTILES nor REDIRECTABLE_PROJECTILE.
        @Override public boolean is(net.minecraft.tags.TagKey<EntityType<?>> tag){return false;}
    }
    static final class Level extends net.minecraft.server.level.ServerLevel {
        Map<Long,Chunk> chunks;WorldBorder border;net.minecraft.util.RandomSource rng;List<String> events;net.minecraft.server.MinecraftServer server;
        static final net.minecraft.world.attribute.EnvironmentAttributeSystem ATTRIBUTES=net.minecraft.world.attribute.EnvironmentAttributeSystem.builder().build();
        Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public net.minecraft.world.attribute.EnvironmentAttributeSystem environmentAttributes(){return ATTRIBUTES;}
        @Override public boolean isClientSide(){return false;}
        @Override public int getNextEntityId(){return 1;}
        @Override public int getMinY(){return -64;}
        @Override public BlockGetter getChunkForCollisions(int x,int z){return chunks.get(key(x,z));}
        @Override public net.minecraft.world.level.chunk.ChunkAccess getChunk(int x,int z,net.minecraft.world.level.chunk.status.ChunkStatus status,boolean load){return chunks.get(key(x,z));}
        @Override public BlockState getBlockState(BlockPos p){var c=chunks.get(key(p.getX()>>4,p.getZ()>>4));return c==null?Blocks.AIR.defaultBlockState():c.getBlockState(p);}
        @Override public FluidState getFluidState(BlockPos p){return getBlockState(p).getFluidState();}
        @Override public List<VoxelShape> getEntityCollisions(Entity e,AABB b){return List.of();}
        Target target;
        @Override public List<Entity> getEntities(Entity e,AABB b,java.util.function.Predicate<? super Entity> p){
            return target!=null&&target.getBoundingBox().intersects(b)&&p.test(target)?List.of(target):List.of();
        }
        @Override public WorldBorder getWorldBorder(){return border;}
        @Override public net.minecraft.util.RandomSource getRandom(){return rng;}
        @Override public boolean isRainingAt(BlockPos p){return false;}
        @Override public net.minecraft.server.MinecraftServer getServer(){return server;}
        @Override public void gameEvent(Holder<GameEvent> event,Vec3 pos,GameEvent.Context context){
            if(event==GameEvent.PROJECTILE_SHOOT)events.add("shoot-event");else if(event==GameEvent.SPLASH)events.add("splash-event");else if(event!=GameEvent.PROJECTILE_LAND)throw new AssertionError(event);
        }
        // Projectile.onHit's PROJECTILE_LAND: the game posts it from the hit it is handed.
        @Override public void gameEvent(Holder<GameEvent> event,BlockPos pos,GameEvent.Context context){
            if(event==GameEvent.PRIME_FUSE)events.add("prime-event");else if(event!=GameEvent.PROJECTILE_LAND)throw new AssertionError(event);
        }
        net.minecraft.world.level.gamerules.GameRules rules;Sources sources;
        @Override public net.minecraft.world.damagesource.DamageSources damageSources(){return sources;}
        @Override public net.minecraft.world.level.gamerules.GameRules getGameRules(){return rules;}
        // onProjectileHit's block changes (campfires, candles, TNT's removal).
        @Override public boolean setBlock(BlockPos p,BlockState state,int flags){
            chunks.get(key(p.getX()>>4,p.getZ()>>4)).write(p,Block.getId(state));events.add("write "+p.getX()+","+p.getY()+","+p.getZ()+" "+Block.getId(state)+" "+flags);return true;
        }
        @Override public boolean addFreshEntity(Entity e){
            // spawnAtLocation's item (its motion comes from the item's own unseeded random).
            if(e instanceof net.minecraft.world.entity.item.ItemEntity item){events.add("drop "+bits(item.position())+" "+item.getItem().getCount());return true;}
            if(!(e instanceof net.minecraft.world.entity.item.PrimedTnt tnt))throw new AssertionError(e);
            events.add("spawn tnt "+bits(tnt.position())+" "+tnt.getFuse());return true;
        }
        @Override public void playSound(Entity except,double x,double y,double z,SoundEvent s,SoundSource source,float volume,float pitch){
            events.add(String.format("sound %s %08x %08x",sound(s),Float.floatToIntBits(volume),Float.floatToIntBits(pitch)));
        }
        @Override public void addParticle(net.minecraft.core.particles.ParticleOptions p,double x,double y,double z,double xd,double yd,double zd){
            events.add("particle "+particle(p)+" "+bits(new Vec3(x,y,z))+" "+bits(new Vec3(xd,yd,zd)));
        }
        @Override public <T extends net.minecraft.core.particles.ParticleOptions> int sendParticles(T p,double x,double y,double z,int count,double dx,double dy,double dz,double speed){
            if(count!=1)throw new AssertionError(count);
            events.add("sent "+particle(p)+" "+bits(new Vec3(x,y,z))+" "+bits(new Vec3(dx,dy,dz))+String.format(" %016x",Double.doubleToLongBits(speed)));return 0;
        }
    }
    /** A player for `setOwner` and `playerTouch`: a real inventory and abilities,
     * and `take` (the pickup packet) recorded. */
    static final class Toucher extends net.minecraft.server.level.ServerPlayer{
        boolean took;
        Toucher(){super(null,null,null,null);}
        @Override public void take(Entity entity,int count){took=true;}
    }
    static Toucher toucher(Object unsafe,Method allocate,boolean creative,String inventory)throws Exception{
        var player=(Toucher)allocate.invoke(unsafe,Toucher.class);
        var abilities=new net.minecraft.world.entity.player.Abilities();abilities.instabuild=creative;
        field(net.minecraft.world.entity.player.Player.class,"abilities").set(player,abilities);
        var items=new net.minecraft.world.entity.player.Inventory(player,new EntityEquipment());
        field(net.minecraft.world.entity.player.Player.class,"inventory").set(player,items);
        int filled=switch(inventory){case "full"->36;case "partial"->35;default->0;};
        for(int i=0;i<filled;i++)items.setItem(i,new ItemStack(Items.STONE));
        return player;
    }
    static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
    static long seed(JsonElement v){return Long.parseLong(v.getAsString());}
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(Path.of(args[2]).resolve("block-tags.json"))).getAsJsonObject());
        // The vanilla data pack's #water and #lava fluid tags.
        net.minecraft.core.registries.BuiltInRegistries.FLUID.prepareTagReload(new net.minecraft.tags.TagLoader.LoadResult<>(net.minecraft.core.registries.Registries.FLUID,Map.of(
            net.minecraft.tags.FluidTags.WATER,List.<Holder<net.minecraft.world.level.material.Fluid>>of(net.minecraft.world.level.material.Fluids.FLOWING_WATER.builtInRegistryHolder(),net.minecraft.world.level.material.Fluids.WATER.builtInRegistryHolder()),
            net.minecraft.tags.FluidTags.LAVA,List.<Holder<net.minecraft.world.level.material.Fluid>>of(net.minecraft.world.level.material.Fluids.FLOWING_LAVA.builtInRegistryHolder(),net.minecraft.world.level.material.Fluids.LAVA.builtInRegistryHolder())))).apply();
        // The arrow's pickup stack: an arrow item without data components.
        Items.ARROW.builtInRegistryHolder().bindComponents(net.minecraft.core.component.DataComponentMap.EMPTY);
        Items.STONE.builtInRegistryHolder().bindComponents(net.minecraft.core.component.DataComponentMap.EMPTY);
        var u=Class.forName("sun.misc.Unsafe");var f=u.getDeclaredField("theUnsafe");f.setAccessible(true);var unsafe=f.get(null);var allocate=u.getMethod("allocateInstance",Class.class);
        var description=JsonParser.parseString(Files.readString(dir.resolve("scene.json"))).getAsJsonObject();
        var trace=new Trace();trace.out=new PrintWriter(OutputStream.nullOutputStream());trace.fail=-1;
        var level=(Level)allocate.invoke(unsafe,Level.class);level.chunks=VanillaSceneCollisionOracle.scene(description,trace);level.border=new Border();
        level.rng=net.minecraft.util.RandomSource.create(0);level.events=new ArrayList<>();
        field(net.minecraft.world.level.Level.class,"random").set(level,level.rng);
        level.rules=new net.minecraft.world.level.gamerules.GameRules(net.minecraft.world.flag.FeatureFlagSet.of());
        level.sources=(Sources)allocate.invoke(unsafe,Sources.class);
        level.sources.any=new net.minecraft.world.damagesource.DamageSource(Holder.direct(new net.minecraft.world.damagesource.DamageType("generic",0.0F)));
        var server=(VanillaBlockLandingOracle.Server)allocate.invoke(unsafe,VanillaBlockLandingOracle.Server.class);
        server.d=new VanillaBlockLandingOracle.Data();server.d.out=new PrintWriter(OutputStream.nullOutputStream());server.d.f=JsonParser.parseString("{\"fail\":-1,\"protected\":false}").getAsJsonObject();level.server=server;
        field(net.minecraft.server.MinecraftServer.class,"debugSubscribers").set(server,new net.minecraft.util.debug.ServerDebugSubscribers(server));
        var random=field(Entity.class,"random");var inGroundTime=field(AbstractArrow.class,"inGroundTime");var life=field(AbstractArrow.class,"life");var lastState=field(AbstractArrow.class,"lastState");
        var touching=field(Entity.class,"wasTouchingWater");var eyes=field(Entity.class,"wasEyeInWater");var first=field(Entity.class,"firstTick");
        var inGround=AbstractArrow.class.getDeclaredMethod("isInGround");inGround.setAccessible(true);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var row=JsonParser.parseString(line).getAsJsonObject();
                // ProjectileWeaponItem.useAmmo's stack: intangible when no ammo was used.
                var ammo=new ItemStack(Items.ARROW);
                if(row.get("intangible").getAsBoolean())ammo.set(net.minecraft.core.component.DataComponents.INTANGIBLE_PROJECTILE,net.minecraft.util.Unit.INSTANCE);
                var a=new Arrow(level,0,0,0,ammo,null);
                random.set(a,net.minecraft.util.RandomSource.create(seed(row.get("seed"))));
                a.setPos(VanillaSceneCollisionOracle.vector(row.get("position")));a.setDeltaMovement(VanillaSceneCollisionOracle.vector(row.get("velocity")));
                // The client's recreateFromPacket snaps a spawned arrow, old position included.
                a.setOldPosAndRot();
                a.setCritArrow(row.get("crit").getAsBoolean());a.setNoPhysics(row.get("no_physics").getAsBoolean());
                String touched="-";level.target=null;
                for(var value:row.getAsJsonArray("ops")){
                    var op=value.getAsJsonObject();
                    switch(op.get("mode").getAsString()){
                        // ServerLevel.tickNonPassenger: old position, tick count, tick.
                        case "tick"->{if(!a.isRemoved()){a.setOldPosAndRot();a.tickCount++;a.tick();}}
                        case "block"->{
                            var v=op.getAsJsonArray("position");var state=VanillaSceneCollisionOracle.state(op.get("name").getAsString());
                            if(op.has("properties"))for(var property:op.getAsJsonObject("properties").entrySet())state=VanillaSceneCollisionOracle.property(state,state.getBlock().getStateDefinition().getProperty(property.getKey()),property.getValue().getAsString());
                            VanillaSceneCollisionOracle.put(level.chunks,v.get(0).getAsInt(),v.get(1).getAsInt(),v.get(2).getAsInt(),state);
                        }
                        case "velocity"->a.setDeltaMovement(VanillaSceneCollisionOracle.vector(op.get("value")));
                        // Flame's projectile_spawned ignite.
                        case "ignite"->a.igniteForSeconds(Float.intBitsToFloat(Integer.parseUnsignedInt(op.get("value").getAsString(),16)));
                        case "level_seed"->level.rng.setSeed(seed(op.get("value")));
                        // setOwner(owner) for the pickup rule, then no owner for the flight.
                        case "owner"->{
                            Entity owner=switch(op.get("value").getAsString()){
                                case "player"->toucher(unsafe,allocate,false,"empty");
                                case "mob"->(Entity)allocate.invoke(unsafe,net.minecraft.world.entity.monster.zombie.Zombie.class);
                                default->null;
                            };
                            a.setOwner(owner);a.setOwner((Entity)null);
                        }
                        // A player-sized box standing at the position.
                        case "target"->{
                            var t=(Target)allocate.invoke(unsafe,Target.class);t.takes=op.get("takes").getAsBoolean();t.events=level.events;
                            var at=VanillaSceneCollisionOracle.vector(op.get("position"));
                            field(Entity.class,"level").set(t,level);field(Entity.class,"position").set(t,at);
                            field(Entity.class,"bb").set(t,EntityDimensions.scalable(0.6F,1.8F).makeBoundingBox(at));
                            level.target=t;
                        }
                        // ArrowItem.asProjectile (dispensers) sets pickup directly.
                        case "pickup"->a.pickup=AbstractArrow.Pickup.byOrdinal(op.get("value").getAsInt());
                        case "touch"->{
                            var player=toucher(unsafe,allocate,op.get("creative").getAsBoolean(),op.get("inventory").getAsString());
                            a.playerTouch(player);
                            int arrows=0;for(int i=0;i<36;i++)if(player.getInventory().getItem(i).getItem()==Items.ARROW)arrows+=player.getInventory().getItem(i).getCount();
                            touched=player.took+" "+arrows;
                        }
                        default->throw new AssertionError(op);
                    }
                    var state=(BlockState)lastState.get(a);
                    out.printf("arrow %s %s %08x %08x %s %d %d %d %s %d %s %s %s %016x %016x %s %s %016x %s%n",bits(a.position()),bits(a.getDeltaMovement()),Float.floatToIntBits(a.getXRot()),Float.floatToIntBits(a.getYRot()),
                        inGround.invoke(a),inGroundTime.getInt(a),life.getInt(a),a.shakeTime,state==null?"none":Block.getId(state),a.getRemainingFireTicks(),touching.getBoolean(a),eyes.getBoolean(a),first.getBoolean(a),
                        Double.doubleToLongBits(a.getFluidHeight(net.minecraft.tags.FluidTags.WATER)),Double.doubleToLongBits(a.getFluidHeight(net.minecraft.tags.FluidTags.LAVA)),a.isRemoved(),a.isCritArrow(),Double.doubleToLongBits(a.fallDistance),bits(new Vec3(a.xo,a.yo,a.zo)));
                    out.println("events "+String.join(" | ",level.events));level.events.clear();
                    out.printf("pickup %d %s %s%n",a.pickup.ordinal(),a.getPickupItemStackOrigin().has(net.minecraft.core.component.DataComponents.INTANGIBLE_PROJECTILE),touched);touched="-";
                }
                out.println("end");
            }
        }
    }
}
