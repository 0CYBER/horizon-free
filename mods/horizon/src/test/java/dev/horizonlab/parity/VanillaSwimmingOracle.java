package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.registries.*;import net.minecraft.tags.*;import net.minecraft.world.entity.*;import net.minecraft.world.entity.player.*;import net.minecraft.world.level.*;import net.minecraft.world.level.material.*;
/** Actual Entity/Player swimming transitions and fluid status queries. */
public final class VanillaSwimmingOracle {
    static final class Stop extends RuntimeException {}
    static Field eye,position,abilities,eyeHeight;
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static String pos(BlockPos p){return p.getX()+" "+p.getY()+" "+p.getZ();}
    static final class Data {JsonObject f;PrintWriter out;Entity entity;int calls;boolean swimming,water,sprint,passenger;
        void query(String s){out.println(s);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt()){swimming=!swimming;water=!water;sprint=!sprint;passenger=!passenger;try{eye.setBoolean(entity,!eye.getBoolean(entity));position.set(entity,new BlockPos(31,32,33));}catch(Exception e){throw new AssertionError(e);}}}
        boolean swimming(){query("swimming");return swimming;}boolean water(){query("water");return water;}boolean sprint(){query("sprint");return sprint;}boolean passenger(){query("passenger");return passenger;}
        void set(boolean v){query("set "+v);swimming=v;}
    }
    static final class World extends net.minecraft.server.level.ServerLevel {Data d;FluidState fluid;World(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public FluidState getFluidState(BlockPos p){d.query("fluid "+pos(p));return fluid;}
    }
    static final class Actor extends Marker {Data d;Actor(){super(null,null);}
        @Override public boolean isSwimming(){return d.swimming();}@Override public boolean isInWater(){return d.water();}@Override public boolean isSprinting(){return d.sprint();}@Override public boolean isPassenger(){return d.passenger();}
        @Override public boolean isUnderWater(){d.query("under");return super.isUnderWater();}@Override public void setSwimming(boolean v){d.set(v);}
    }
    static final class Human extends Player {Data d;Human(){super(null,null);}
        @Override public GameType gameMode(){return GameType.SURVIVAL;}
        @Override public boolean isSwimming(){return d.swimming();}@Override public boolean isInWater(){return d.water();}@Override public boolean isSprinting(){return d.sprint();}@Override public boolean isPassenger(){return d.passenger();}
        @Override public boolean isUnderWater(){d.query("under");return super.isUnderWater();}@Override public void setSwimming(boolean v){d.set(v);}
    }
    static Field field(Class<?> c,String name)throws Exception{var f=c.getDeclaredField(name);f.setAccessible(true);return f;}
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");var unsafe=field(u,"theUnsafe").get(null);var allocate=u.getMethod("allocateInstance",Class.class);eye=field(Entity.class,"wasEyeInWater");position=field(Entity.class,"blockPosition");abilities=field(Player.class,"abilities");eyeHeight=field(Entity.class,"eyeHeight");var level=field(Entity.class,"level");Fluid[] fluids={Fluids.EMPTY,Fluids.FLOWING_WATER,Fluids.WATER,Fluids.FLOWING_LAVA,Fluids.LAVA};int oldMask=-1;Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();int mask=f.get("water_mask").getAsInt();if(oldMask!=mask){var members=new ArrayList<Holder<Fluid>>();for(int i=0;i<5;i++)if((mask&(1<<i))!=0)members.add(fluids[i].builtInRegistryHolder());BuiltInRegistries.FLUID.prepareTagReload(new TagLoader.LoadResult<>(Registries.FLUID,Map.of(FluidTags.WATER,members))).apply();oldMask=mask;}
            var d=new Data();d.f=f;d.out=out;d.swimming=f.get("swimming").getAsBoolean();d.water=f.get("water").getAsBoolean();d.sprint=f.get("sprint").getAsBoolean();d.passenger=f.get("passenger").getAsBoolean();if(f.get("player").getAsBoolean()){var h=(Human)allocate.invoke(unsafe,Human.class);h.d=d;var a=new Abilities();a.flying=f.get("flying").getAsBoolean();abilities.set(h,a);d.entity=h;}else{var a=(Actor)allocate.invoke(unsafe,Actor.class);a.d=d;d.entity=a;}
            var w=(World)allocate.invoke(unsafe,World.class);w.d=d;w.fluid=fluids[f.get("fluid").getAsInt()].defaultFluidState();level.set(d.entity,w);position.set(d.entity,new BlockPos(17,-64,-33));eye.setBoolean(d.entity,f.get("eye").getAsBoolean());eyeHeight.setFloat(d.entity,single(f.get("eye_height")));
            try{switch(f.get("mode").getAsString()){case "update":d.entity.updateSwimming();out.println("result ok");break;case "under":out.println("result "+d.entity.isUnderWater());break;case "shallow":out.println("result "+d.entity.isInShallowWater());break;case "threshold":out.printf("threshold %016x%n",Double.doubleToLongBits(d.entity.getFluidJumpThreshold()));break;default:throw new AssertionError();}}catch(Stop e){out.println("error");}
            out.println("final "+d.swimming+" "+eye.getBoolean(d.entity)+" "+pos((BlockPos)position.get(d.entity)));out.println("queries "+d.calls);
        }}
    }
}
