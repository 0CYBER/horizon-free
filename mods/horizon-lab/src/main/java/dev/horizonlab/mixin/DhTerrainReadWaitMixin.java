package dev.horizonlab.mixin;

import dev.horizonlab.budget.TerrainReadWait;
import java.util.concurrent.CompletableFuture;
import net.minecraft.world.level.chunk.storage.IOWorker;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

/** Only blending metadata reads on pinned DH workers become interruptible; saves stay upstream. */
@Mixin(IOWorker.class)
public abstract class DhTerrainReadWaitMixin {
    @Redirect(method = "isOldChunkAround", at = @At(value = "INVOKE",
        target = "Ljava/util/concurrent/CompletableFuture;join()Ljava/lang/Object;"))
    private Object horizon$waitForBlendingMetadata(CompletableFuture<?> future) {
        return TerrainReadWait.join(future);
    }
}
