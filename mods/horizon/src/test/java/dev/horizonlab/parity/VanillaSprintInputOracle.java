package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.*;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.gui.screens.LevelLoadingScreen;
import net.minecraft.client.multiplayer.MultiPlayerGameMode;
import net.minecraft.client.player.*;
import net.minecraft.client.tutorial.Tutorial;
import net.minecraft.core.Holder;
import net.minecraft.core.component.*;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.player.*;
import net.minecraft.world.food.FoodData;
import net.minecraft.world.item.*;
import net.minecraft.world.item.component.UseEffects;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.tags.TagKey;

/** Real LocalPlayer.aiStep prefix through sprint decisions; stop before riding
 * and parent/world movement. UI/effect/world queries are controlled fixtures. */
public final class VanillaSprintInputOracle {
    static Object unsafe;static Method allocate;static Constructor<ItemStack> stackConstructor;
    static <T>T make(Class<T> type)throws Exception{return type.cast(allocate.invoke(unsafe,type));}
    static Field field(Class<?> type,String name)throws Exception{var f=type.getDeclaredField(name);f.setAccessible(true);return f;}
    static final class Boundary extends RuntimeException {}
    static final class Mode extends MultiPlayerGameMode {
        Mode(){super(null,null);}
        @Override public boolean isSpectator(){return false;}
    }
    static final class Vehicle extends net.minecraft.world.entity.decoration.ArmorStand {
        Actor actor;Vehicle(){super(EntityTypes.ARMOR_STAND,null);}
        @Override public boolean canSprint(){return actor.flag(2);}
        void configure(net.minecraft.world.level.Level level){setLevel(level);}
        @Override protected boolean isLocalClientAuthoritative(){return actor.flag(3);}
    }
    static final class Actor extends LocalPlayer {
        int flags;boolean sprint;List<Boolean> calls;Vehicle vehicle;
        net.minecraft.server.level.ServerPlayer controlled;
        Actor(){super(null,null,null,null,null,Input.EMPTY,false,null,null);}
        boolean flag(int bit){return (flags&(1<<bit))!=0;}
        @Override public boolean isMobilityRestricted(){return flag(0);}
        @Override public boolean isPassenger(){return flag(1);}
        @Override public Entity getVehicle(){return flag(1)?vehicle:null;}
        @Override public boolean isInShallowWater(){return flag(6);}
        @Override public boolean isUsingItem(){return flag(7);}
        @Override public boolean isFallFlying(){return flag(9);}
        @Override public boolean isUnderWater(){return flag(10);}
        @Override public boolean isMovingSlowly(){return controlled==null?flag(11):super.isMovingSlowly();}
        @Override public Pose getPose(){return controlled==null?Pose.STANDING:controlled.getPose();}
        @Override public double getAttributeValue(Holder<net.minecraft.world.entity.ai.attributes.Attribute> attribute){return controlled==null?super.getAttributeValue(attribute):controlled.getAttributeValue(attribute);}
        @Override public boolean isSwimming(){return flag(12);}
        @Override public boolean isInWater(){return flag(13);}
        @Override public boolean onGround(){return flag(14);}
        @Override public boolean isSleeping(){return false;}
        @Override public boolean canPlayerFitWithinBlocksAndEntitiesWhen(Pose pose){
            if(controlled==null)return true;
            try{var method=Player.class.getDeclaredMethod("canPlayerFitWithinBlocksAndEntitiesWhen",Pose.class);method.setAccessible(true);return (Boolean)method.invoke(controlled,pose);}catch(ReflectiveOperationException e){throw new AssertionError(e);}
        }
        @Override public float getEffectBlendFactor(Holder<MobEffect> effect,float partial){return 0;}
        @Override public boolean isEyeInFluid(TagKey<Fluid> tag){return false;}
        @Override public boolean onClimbable(){return false;}
        @Override public boolean tryToStartFallFlying(){return false;}
        // The controlled actor sinks (LivingEntity.goDownInWater on it).
        @Override protected void goDownInWater(){
            if(controlled==null)return;
            try{var method=LivingEntity.class.getDeclaredMethod("goDownInWater");method.setAccessible(true);method.invoke(controlled);}catch(ReflectiveOperationException e){throw new AssertionError(e);}
        }
        @Override protected boolean isControlledCamera(){return false;}
        @Override public void onUpdateAbilities(){}
        @Override public void jumpFromGround(){} // post-sprint ability-flight callback
        @Override public boolean isSprinting(){return controlled==null?sprint:controlled.isSprinting();}
        @Override public void setSprinting(boolean value){calls.add(value);sprint=value;if(controlled!=null)controlled.setSprinting(value);}
        @Override public PlayerRideableJumping jumpableVehicle(){throw new Boundary();}
        void item()throws Exception{useItem=stackConstructor.newInstance(Holder.direct(Items.STONE),1,new PatchedDataComponentMap(DataComponentMap.EMPTY));useItem.set(DataComponents.USE_EFFECTS,new UseEffects(flag(8),false,.2f));}
        int trigger(){return sprintTriggerTime;}
        void trigger(int v){sprintTriggerTime=v;}
    }
    /** Client-only bridge used by the resident scene comparison. Sprint setters
     * dispatch to its real server actor, sharing food and collision conditions. */
    public static final class Controller {
        final Actor p;final KeyMapping[] keys;final Field down;final Method modify;
        public Controller(net.minecraft.server.level.ServerPlayer controlled)throws Exception{
            initialize();
            var options=make(Options.class);String[] names={"keyUp","keyDown","keyLeft","keyRight","keyJump","keyShift","keySprint"};
            keys=new KeyMapping[names.length];down=field(KeyMapping.class,"isDown");
            for(int i=0;i<keys.length;i++){keys[i]=make(KeyMapping.class);field(Options.class,names[i]).set(options,keys[i]);}
            var window=make(OptionInstance.class);field(OptionInstance.class,"value").set(window,7);field(Options.class,"sprintWindow").set(options,window);
            var mc=make(Minecraft.class);field(Minecraft.class,"options").set(mc,options);
            var gui=make(Gui.class);field(Gui.class,"screen").set(gui,make(LevelLoadingScreen.class));field(Minecraft.class,"gui").set(mc,gui);
            field(Minecraft.class,"tutorial").set(mc,make(Tutorial.class));field(Minecraft.class,"gameMode").set(mc,make(Mode.class));
            p=make(Actor.class);p.controlled=controlled;p.calls=new ArrayList<>();p.noPhysics=true;p.input=new KeyboardInput(options);
            field(Player.class,"abilities").set(p,new Abilities());field(Player.class,"foodData").set(p,controlled.getFoodData());field(LocalPlayer.class,"minecraft").set(p,mc);
            modify=LocalPlayer.class.getDeclaredMethod("modifyInput",net.minecraft.world.phys.Vec2.class);modify.setAccessible(true);
        }
        public net.minecraft.world.phys.Vec2 step(int mask)throws Exception{
            p.flags=(p.controlled.onGround()?1<<14:0)|(p.controlled.horizontalCollision?1<<15:0)|(p.controlled.minorHorizontalCollision?1<<16:0)
                |(p.controlled.isInShallowWater()?1<<6:0)|(p.controlled.isUnderWater()?1<<10:0)|(p.controlled.isSwimming()?1<<12:0)|(p.controlled.isInWater()?1<<13:0);
            p.horizontalCollision=p.controlled.horizontalCollision;p.minorHorizontalCollision=p.controlled.minorHorizontalCollision;
            p.calls.clear();for(int i=0;i<keys.length;i++)down.setBoolean(keys[i],(mask&(1<<i))!=0);
            try{p.aiStep();throw new AssertionError("missing sprint boundary");}catch(Boundary expected){}
            p.controlled.setShiftKeyDown((mask&32)!=0);
            return (net.minecraft.world.phys.Vec2)modify.invoke(p,p.input.getMoveVector());
        }
        public String state(){return p.trigger()+" "+p.input.hasForwardImpulse()+" "+p.input.keyPresses.shift();}
        public boolean crouching(){return p.isCrouching();}
    }
    static void initialize()throws Exception{
        if(unsafe!=null)return;
        var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);
        stackConstructor=ItemStack.class.getDeclaredConstructor(Holder.class,int.class,PatchedDataComponentMap.class);stackConstructor.setAccessible(true);
    }
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        initialize();
        var options=make(Options.class);String[] names={"keyUp","keyDown","keyLeft","keyRight","keyJump","keyShift","keySprint"};
        var keys=new KeyMapping[names.length];var down=field(KeyMapping.class,"isDown");
        for(int i=0;i<keys.length;i++){keys[i]=make(KeyMapping.class);field(Options.class,names[i]).set(options,keys[i]);}
        var window=make(OptionInstance.class);field(Options.class,"sprintWindow").set(options,window);
        var mc=make(Minecraft.class);field(Minecraft.class,"options").set(mc,options);
        var gui=make(Gui.class);field(Gui.class,"screen").set(gui,make(LevelLoadingScreen.class));field(Minecraft.class,"gui").set(mc,gui);
        field(Minecraft.class,"tutorial").set(mc,make(Tutorial.class));field(Minecraft.class,"gameMode").set(mc,make(Mode.class));
        try(var out=new PrintWriter(Files.newBufferedWriter(Path.of(args[1])));var lines=Files.lines(Path.of(args[0]))){
            for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();
                var p=make(Actor.class);p.calls=new ArrayList<>();p.noPhysics=true;p.sprint=f.get("sprint").getAsBoolean();p.trigger(f.get("trigger").getAsInt());
                var abilities=new Abilities();field(Player.class,"abilities").set(p,abilities);field(Player.class,"foodData").set(p,new FoodData());
                var level=make(net.minecraft.client.multiplayer.ClientLevel.class);field(net.minecraft.world.level.Level.class,"isClientSide").setBoolean(level,true);
                p.vehicle=make(Vehicle.class);p.vehicle.actor=p;p.vehicle.configure(level);field(LocalPlayer.class,"minecraft").set(p,mc);
                var keyboard=new KeyboardInput(options);p.input=keyboard;
                int initial=f.get("initial_keys").getAsInt();for(int i=0;i<keys.length;i++)down.setBoolean(keys[i],(initial&(1<<i))!=0);keyboard.tick();
                for(var element:f.getAsJsonArray("steps")){var step=element.getAsJsonObject();p.flags=step.get("flags").getAsInt();p.calls.clear();p.item();
                    abilities.mayfly=p.flag(4);abilities.flying=p.flag(5);p.getFoodData().setFoodLevel(step.get("food").getAsInt());
                    p.horizontalCollision=p.flag(15);p.minorHorizontalCollision=p.flag(16);field(OptionInstance.class,"value").set(window,step.get("window").getAsInt());
                    int mask=step.get("keys").getAsInt();for(int i=0;i<keys.length;i++)down.setBoolean(keys[i],(mask&(1<<i))!=0);
                    try{p.aiStep();throw new AssertionError("missing sprint boundary");}catch(Boundary expected){}
                    var vector=keyboard.getMoveVector();var calls=new ArrayList<String>();for(boolean value:p.calls)calls.add(Boolean.toString(value));
                    out.printf("%s %d %08x %08x %s | %s%n",p.sprint,p.trigger(),Float.floatToIntBits(vector.x),Float.floatToIntBits(vector.y),keyboard.keyPresses.shift(),String.join(",",calls));
                }
                out.println("end");
            }
        }
    }
}
