package dev.horizonlab.mixin;

import dev.horizonlab.HorizonLab;
import java.util.function.BooleanSupplier;
import net.minecraft.server.MinecraftServer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(MinecraftServer.class)
public abstract class ServerTickMixin {
    @Unique private long horizonTickStart;

    @Inject(method = "tickServer", at = @At("HEAD"))
    private void horizonBegin(BooleanSupplier haveTime, CallbackInfo callback) {
        horizonTickStart = System.nanoTime();
    }

    @Inject(method = "tickServer", at = @At("RETURN"))
    private void horizonEnd(BooleanSupplier haveTime, CallbackInfo callback) {
        HorizonLab.tick(System.nanoTime() - horizonTickStart, (MinecraftServer) (Object) this);
    }
}
