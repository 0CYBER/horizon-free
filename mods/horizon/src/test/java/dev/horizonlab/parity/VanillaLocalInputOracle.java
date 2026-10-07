package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import net.minecraft.client.*;
import net.minecraft.client.player.*;
import net.minecraft.core.Holder;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.component.DataComponentMap;
import net.minecraft.core.component.PatchedDataComponentMap;
import net.minecraft.world.entity.ai.attributes.*;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.item.*;
import net.minecraft.world.item.component.UseEffects;
import net.minecraft.world.phys.Vec2;

/** Calls real KeyboardInput.tick and private LocalPlayer.modifyInput. */
public final class VanillaLocalInputOracle {
    static Object unsafe;static Method allocate;static Constructor<ItemStack> stackConstructor;
    static <T>T make(Class<T> type)throws Exception{return type.cast(allocate.invoke(unsafe,type));}
    static Field field(Class<?> type,String name)throws Exception{var f=type.getDeclaredField(name);f.setAccessible(true);return f;}
    static float number(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static final class Actor extends LocalPlayer {
        boolean using,riding,slow;float sneak;
        Actor(){super(null,null,null,null,null,Input.EMPTY,false,null,null);}
        @Override public boolean isUsingItem(){return using;}
        @Override public boolean isPassenger(){return riding;}
        @Override public boolean isMovingSlowly(){return slow;}
        @Override public double getAttributeValue(Holder<Attribute> attribute){if(attribute!=Attributes.SNEAKING_SPEED)throw new AssertionError(attribute);return sneak;}
        void item(float speed)throws Exception{useItem=stackConstructor.newInstance(Holder.direct(Items.STONE),1,new PatchedDataComponentMap(DataComponentMap.EMPTY));useItem.set(DataComponents.USE_EFFECTS,new UseEffects(false,false,speed));}
    }
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);
        stackConstructor=ItemStack.class.getDeclaredConstructor(Holder.class,int.class,PatchedDataComponentMap.class);stackConstructor.setAccessible(true);
        var options=make(Options.class);String[] names={"keyUp","keyDown","keyLeft","keyRight","keyJump","keyShift","keySprint"};
        var keys=new KeyMapping[names.length];var down=field(KeyMapping.class,"isDown");
        for(int i=0;i<names.length;i++){keys[i]=make(KeyMapping.class);field(Options.class,names[i]).set(options,keys[i]);}
        var keyboard=new KeyboardInput(options);var actor=make(Actor.class);
        var modify=LocalPlayer.class.getDeclaredMethod("modifyInput",Vec2.class);modify.setAccessible(true);
        try(var out=new PrintWriter(Files.newBufferedWriter(Path.of(args[1])));var lines=Files.lines(Path.of(args[0]))){
            for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();int mask=f.get("keys").getAsInt();
                for(int i=0;i<keys.length;i++)down.setBoolean(keys[i],(mask&(1<<i))!=0);
                keyboard.tick();var direction=keyboard.getMoveVector();
                Vec2 vector=direction;if(f.has("vector")){var v=f.getAsJsonArray("vector");vector=new Vec2(number(v.get(0)),number(v.get(1)));}
                actor.using=f.get("using").getAsBoolean();actor.riding=f.get("riding").getAsBoolean();actor.slow=f.get("slow").getAsBoolean();actor.sneak=number(f.get("sneak"));actor.item(number(f.get("item")));
                var result=(Vec2)modify.invoke(actor,vector);
                out.printf("%08x %08x %08x %08x%n",Float.floatToIntBits(direction.x),Float.floatToIntBits(direction.y),Float.floatToIntBits(result.x),Float.floatToIntBits(result.y));
            }
        }
    }
}
