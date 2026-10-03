package dev.horizonlab.mixin;

import com.seibel.distanthorizons.core.api.internal.chunkUpdating.ChunkPosQueue;
import com.seibel.distanthorizons.core.api.internal.chunkUpdating.ChunkUpdateData;
import com.seibel.distanthorizons.core.api.internal.chunkUpdating.ChunkUpdateQueueManager;
import com.seibel.distanthorizons.core.pos.DhChunkPos;
import dev.horizonlab.budget.DebounceWait;
import dev.horizonlab.budget.WakeableIdleWait;
import java.util.concurrent.locks.LockSupport;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Pseudo;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.Redirect;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Pseudo
@Mixin(targets = "com.seibel.distanthorizons.core.api.internal.chunkUpdating.ChunkUpdateQueueManager", remap = false)
public abstract class DhDebounceWaitMixin {
    @Shadow @Final public ChunkPosQueue preUpdateQueue;
    @Unique private volatile Thread horizonQueueWorker;
    @Unique private long horizonLastIdleCleanup;

    @Inject(method = "runQueueingLoop", at = @At("HEAD"), remap = false)
    private void horizonRememberWorker(CallbackInfo callback) {
        horizonQueueWorker = Thread.currentThread();
        horizonLastIdleCleanup = System.nanoTime();
    }

    @Inject(method = "runQueueingLoop", at = @At("RETURN"), remap = false)
    private void horizonReleaseWorker(CallbackInfo callback) {
        horizonQueueWorker = null;
    }

    @Redirect(method = "runQueueingLoop", at = @At(value = "INVOKE",
        target = "Ljava/lang/Thread;sleep(J)V"), require = 1, allow = 1, remap = false)
    private void horizonWakeableIdle(long milliseconds) throws InterruptedException {
        if (milliseconds != 50 || Thread.currentThread() != horizonQueueWorker) {
            Thread.sleep(milliseconds);
            return;
        }
        long remaining = 50_000_000L - (System.nanoTime() - horizonLastIdleCleanup);
        WakeableIdleWait.pause(this, Math.max(0, remaining));
    }

    // Early wakeups must not turn the former 50 ms maintenance cadence into per-arrival work.
    @Redirect(method = "runQueueingLoop", at = @At(value = "INVOKE",
        target = "Lcom/google/common/cache/Cache;cleanUp()V"), require = 1, allow = 1, remap = false)
    private void horizonBoundIdleMaintenance(com.google.common.cache.Cache<?, ?> cache) {
        long now = System.nanoTime();
        if (Thread.currentThread() != horizonQueueWorker || now - horizonLastIdleCleanup >= 50_000_000L) {
            cache.cleanUp();
            horizonLastIdleCleanup = System.nanoTime();
        }
    }

    // Ordinal zero is specifically the debounce retry. Executor rejection retries remain unchanged.
    @Redirect(method = "tryDispatchReadyChunkUpdate", at = @At(value = "INVOKE",
        target = "Lcom/seibel/distanthorizons/core/api/internal/chunkUpdating/ChunkPosQueue;addItem(Lcom/seibel/distanthorizons/core/pos/DhChunkPos;Lcom/seibel/distanthorizons/core/api/internal/chunkUpdating/ChunkUpdateData;)V",
        ordinal = 0), require = 1, allow = 1, remap = false)
    private void horizonWaitForDebounce(ChunkPosQueue queue, DhChunkPos position, ChunkUpdateData update) {
        queue.addItem(position, update);
        if (Thread.currentThread() != horizonQueueWorker) return;
        long delay = DebounceWait.delay(System.nanoTime(), update.firstQueuedNanoTime,
            ChunkUpdateQueueManager.UPDATE_DEBOUNCE_NANOS, !preUpdateQueue.isEmpty());
        if (delay > 0) LockSupport.parkNanos(this, delay);
    }

    @Inject(method = "addItemToQueue", at = @At("RETURN"), remap = false)
    private void horizonWakeForNewWork(CallbackInfo callback) {
        Thread worker = horizonQueueWorker;
        if (worker != null && worker != Thread.currentThread()) LockSupport.unpark(worker);
    }
}
