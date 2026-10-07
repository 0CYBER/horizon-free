package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.sounds.SoundEvent;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;

/**
 * Original exporter of per-state gameplay facts read through vanilla's own accessors:
 * light, hardness, sounds and tool requirements. Item components come from the
 * vanilla data generator's component reports.
 */
public final class VanillaGameplayFactsExporter {
    private static JsonObject sound(SoundEvent event) {
        var result = new JsonObject();
        result.addProperty("id", event.location().toString());
        return result;
    }

    private static JsonArray states() {
        var states = new JsonArray();
        var level = EmptyBlockGetter.INSTANCE;
        for (var state : Block.BLOCK_STATE_REGISTRY) {
            var o = new JsonObject();
            var block = state.getBlock();
            o.addProperty("id", Block.getId(state));
            o.addProperty("emission", state.getLightEmission());
            o.addProperty("dampening", state.getLightDampening());
            o.addProperty("skylight_down", state.propagatesSkylightDown());
            o.addProperty("shape_occlusion", state.useShapeForLightOcclusion());
            o.addProperty("can_occlude", state.canOcclude());
            o.addProperty("solid_render", state.isSolidRender());
            o.addProperty("hardness", state.getDestroySpeed(level, BlockPos.ZERO));
            o.addProperty("requires_tool", state.requiresCorrectToolForDrops());
            o.addProperty("replaceable", state.canBeReplaced());
            o.addProperty("random_ticks", state.isRandomlyTicking());
            o.addProperty("block_entity", state.hasBlockEntity());
            o.addProperty("render_shape", state.getRenderShape().name());
            o.addProperty("liquid", state.liquid());
            o.addProperty("ignited_by_lava", state.ignitedByLava());
            o.addProperty("push", state.getPistonPushReaction().name());
            o.addProperty("offset", state.hasOffsetFunction());
            o.addProperty("full_collision", state.isCollisionShapeFullBlock(level, BlockPos.ZERO));
            o.addProperty("map_color", state.getMapColor(level, BlockPos.ZERO).col);
            o.addProperty("friction", block.getFriction());
            o.addProperty("speed_factor", block.getSpeedFactor());
            o.addProperty("jump_factor", block.getJumpFactor());
            o.addProperty("explosion_resistance", block.getExplosionResistance());
            o.addProperty("item", BuiltInRegistries.ITEM.getKey(block.asItem()).toString());
            var sound = state.getSoundType();
            var sounds = new JsonObject();
            sounds.addProperty("volume", sound.getVolume());
            sounds.addProperty("pitch", sound.getPitch());
            sounds.add("break", sound(sound.getBreakSound()));
            sounds.add("step", sound(sound.getStepSound()));
            sounds.add("place", sound(sound.getPlaceSound()));
            sounds.add("hit", sound(sound.getHitSound()));
            sounds.add("fall", sound(sound.getFallSound()));
            o.add("sound", sounds);
            states.add(o);
        }
        return states;
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var result = new JsonObject();
        result.add("states", states());
        Files.writeString(Path.of(args[0]), new Gson().toJson(result), StandardCharsets.UTF_8);
    }
}
