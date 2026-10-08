package dev.horizonlab.parity;

import com.google.gson.*;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.lang.reflect.RecordComponent;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.Optional;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.sounds.SoundEvent;
import net.minecraft.tags.TagKey;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.SupportType;
import net.minecraft.world.level.material.FlowingFluid;
import net.minecraft.world.level.material.Fluid;

/**
 * Original exporter of the block-behaviour inputs Horizon's level simulation needs:
 * every block's class chain and immutable instance configuration (read reflectively),
 * per-state support/redstone/fluid facts from vanilla's accessors over an empty level,
 * fire odds and block-item placement bindings. No behaviour is evaluated here.
 */
public final class VanillaBlockBehaviourExporter {
    private static JsonElement value(Object v, int depth) {
        if (v == null) {
            return JsonNull.INSTANCE;
        } else if (v instanceof Boolean b) {
            return new JsonPrimitive(b);
        } else if (v instanceof Number n) {
            return new JsonPrimitive(n);
        } else if (v instanceof String || v instanceof Character) {
            return new JsonPrimitive(v.toString());
        } else if (v instanceof Enum<?> e) {
            return new JsonPrimitive(e.name());
        } else if (v instanceof Block b) {
            return new JsonPrimitive(BuiltInRegistries.BLOCK.getKey(b).toString());
        } else if (v instanceof BlockState s) {
            var o = new JsonObject();
            o.addProperty("state", Block.getId(s));
            return o;
        } else if (v instanceof Item i) {
            return new JsonPrimitive(BuiltInRegistries.ITEM.getKey(i).toString());
        } else if (v instanceof Fluid f) {
            return new JsonPrimitive(BuiltInRegistries.FLUID.getKey(f).toString());
        } else if (v instanceof SoundEvent s) {
            return new JsonPrimitive(s.location().toString());
        } else if (v instanceof Identifier id) {
            return new JsonPrimitive(id.toString());
        } else if (v instanceof ResourceKey<?> key) {
            return new JsonPrimitive(key.identifier().toString());
        } else if (v instanceof TagKey<?> tag) {
            return new JsonPrimitive("#" + tag.location());
        } else if (v instanceof Holder<?> holder) {
            if (holder.isBound() && depth > 0) {
                return value(holder.value(), depth - 1);
            }
            return holder.unwrapKey().<JsonElement>map(k -> new JsonPrimitive(k.identifier().toString())).orElse(JsonNull.INSTANCE);
        } else if (v instanceof Optional<?> optional) {
            return optional.isPresent() ? value(optional.get(), depth) : JsonNull.INSTANCE;
        } else if (depth > 0 && v instanceof java.util.Map<?, ?> map) {
            var o = new JsonObject();
            for (var entry : map.entrySet()) {
                var key = value(entry.getKey(), 0);
                o.add(key.isJsonPrimitive() ? key.getAsString() : key.toString(), value(entry.getValue(), depth - 1));
            }
            return o;
        } else if (depth > 0 && v.getClass().isRecord()) {
            var o = new JsonObject();
            for (RecordComponent component : v.getClass().getRecordComponents()) {
                try {
                    component.getAccessor().setAccessible(true);
                    o.add(component.getName(), value(component.getAccessor().invoke(v), depth - 1));
                } catch (Exception e) {
                    o.addProperty(component.getName(), "<error>");
                }
            }
            return o;
        } else if (depth > 0 && v instanceof SoundType sound) {
            var o = new JsonObject();
            o.addProperty("volume", sound.getVolume());
            o.addProperty("pitch", sound.getPitch());
            o.addProperty("break", sound.getBreakSound().location().toString());
            o.addProperty("place", sound.getPlaceSound().location().toString());
            return o;
        } else if (v instanceof net.minecraft.core.particles.ParticleType<?> particle) {
            return new JsonPrimitive(BuiltInRegistries.PARTICLE_TYPE.getKey(particle).toString());
        } else if (depth > 0 && v.getClass().getName().startsWith("net.minecraft.world.level.block.grower.")) {
            return fields(v, Object.class);
        }
        return new JsonPrimitive("<" + v.getClass().getSimpleName() + ">");
    }

    private static JsonObject fields(Object instance, Class<?> top) {
        var o = new JsonObject();
        for (Class<?> c = instance.getClass(); c != null && c != top; c = c.getSuperclass()) {
            for (Field field : c.getDeclaredFields()) {
                if (Modifier.isStatic(field.getModifiers()) || o.has(field.getName())) {
                    continue;
                }
                try {
                    field.setAccessible(true);
                    o.add(field.getName(), value(field.get(instance), 3));
                } catch (Exception e) {
                    o.addProperty(field.getName(), "<error>");
                }
            }
        }
        return o;
    }

    private static JsonArray chain(Class<?> c, Class<?> top) {
        var result = new JsonArray();
        for (; c != null && c != top; c = c.getSuperclass()) {
            result.add(c.getSimpleName());
        }
        return result;
    }

    private static JsonArray blocks() {
        var blocks = new JsonArray();
        for (Block block : BuiltInRegistries.BLOCK) {
            var o = new JsonObject();
            o.addProperty("name", BuiltInRegistries.BLOCK.getKey(block).toString());
            o.add("class", chain(block.getClass(), BlockBehaviour.class));
            var interfaces = new JsonArray();
            for (Class<?> c = block.getClass(); c != null; c = c.getSuperclass()) {
                for (Class<?> i : c.getInterfaces()) {
                    interfaces.add(i.getSimpleName());
                }
            }
            o.add("interfaces", interfaces);
            o.add("fields", fields(block, Block.class));
            o.addProperty("dynamic_shape", block.hasDynamicShape());
            o.addProperty("item", BuiltInRegistries.ITEM.getKey(block.asItem()).toString());
            blocks.add(o);
        }
        return blocks;
    }

    private static final SupportType[] SUPPORTS = {SupportType.FULL, SupportType.CENTER, SupportType.RIGID};

    private static JsonArray states() {
        var states = new JsonArray();
        var level = EmptyBlockGetter.INSTANCE;
        for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
            var o = new JsonArray();
            int sturdy = 0;
            for (int s = 0; s < SUPPORTS.length; s++) {
                for (Direction d : Direction.values()) {
                    try {
                        if (state.isFaceSturdy(level, BlockPos.ZERO, d, SUPPORTS[s])) {
                            sturdy |= 1 << (s * 6 + d.ordinal());
                        }
                    } catch (Exception e) {
                        sturdy |= 1 << 30;
                    }
                }
            }
            int flags = 0;
            if (state.isSolid()) flags |= 1;
            if (state.isAir()) flags |= 2;
            try {
                if (state.isRedstoneConductor(level, BlockPos.ZERO)) flags |= 4;
            } catch (Exception e) {
                flags |= 1 << 20;
            }
            if (state.isSignalSource()) flags |= 8;
            if (state.hasAnalogOutputSignal()) flags |= 16;
            try {
                if (state.getCollisionShape(level, BlockPos.ZERO).isEmpty()) flags |= 32;
            } catch (Exception e) {
                flags |= 1 << 21;
            }
            try {
                if (state.isSuffocating(level, BlockPos.ZERO)) flags |= 64;
            } catch (Exception e) {
                flags |= 1 << 22;
            }
            var fluid = state.getFluidState();
            int fluidId = BuiltInRegistries.FLUID.getId(fluid.getType());
            int fluidAmount = fluid.getAmount();
            boolean falling = fluid.hasProperty(FlowingFluid.FALLING) && fluid.getValue(FlowingFluid.FALLING);
            // Weak/strong signals over an empty level, packed 4 bits per direction.
            long weak = 0;
            long strong = 0;
            boolean signalError = false;
            if (state.isSignalSource()) {
                for (Direction d : Direction.values()) {
                    try {
                        weak |= ((long) state.getSignal(level, BlockPos.ZERO, d) & 15) << (4 * d.ordinal());
                        strong |= ((long) state.getDirectSignal(level, BlockPos.ZERO, d) & 15) << (4 * d.ordinal());
                    } catch (Exception e) {
                        signalError = true;
                    }
                }
            }
            o.add(sturdy);
            o.add(flags);
            o.add(fluidId);
            o.add(fluidAmount);
            o.add(falling);
            o.add(weak);
            o.add(strong);
            o.add(signalError);
            o.add(state.instrument().name());
            states.add(o);
        }
        return states;
    }

    private static JsonObject items() {
        var items = new JsonObject();
        for (Item item : BuiltInRegistries.ITEM) {
            if (!(item instanceof BlockItem)) {
                continue;
            }
            var o = new JsonObject();
            o.add("class", chain(item.getClass(), Item.class));
            o.add("fields", fields(item, Item.class));
            items.add(BuiltInRegistries.ITEM.getKey(item).toString(), o);
        }
        return items;
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var result = new JsonObject();
        result.add("blocks", blocks());
        result.add("states", states());
        result.add("items", items());
        var fluids = new JsonArray();
        for (Fluid fluid : BuiltInRegistries.FLUID) {
            fluids.add(BuiltInRegistries.FLUID.getKey(fluid).toString());
        }
        result.add("fluids", fluids);
        result.add("fire", fields(Blocks.FIRE, Block.class));
        Files.writeString(Path.of(args[0]), new Gson().toJson(result), StandardCharsets.UTF_8);
    }
}
