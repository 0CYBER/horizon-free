package dev.horizonlab.mixin;

import dev.horizonlab.client.HorizonClient;
import net.minecraft.client.Minecraft;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(Minecraft.class)
public abstract class FrameMixin {
    @Inject(method = "runTick", at = @At("HEAD"))
    private void horizonFrame(boolean advanceGameTime, CallbackInfo callback) {
        HorizonClient.frameStart();
    }
}
