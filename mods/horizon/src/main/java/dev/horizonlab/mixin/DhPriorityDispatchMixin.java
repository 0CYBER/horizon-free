package dev.horizonlab.mixin;

import com.seibel.distanthorizons.core.util.threading.PriorityTaskPicker;
import dev.horizonlab.budget.DispatchRetry;
import java.util.concurrent.RejectedExecutionException;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Pseudo;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Constant;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.ModifyConstant;
import org.spongepowered.asm.mixin.injection.ModifyVariable;
import org.spongepowered.asm.mixin.injection.Redirect;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Pseudo
@Mixin(targets = "com.seibel.distanthorizons.core.util.threading.PriorityTaskPicker", remap = false)
public abstract class DhPriorityDispatchMixin {
    @Shadow @Final private AtomicInteger occupiedThreadsRef;
    @Shadow @Final private AtomicBoolean isShutDownRef;
    @Unique private final DispatchRetry horizonDispatch = new DispatchRetry();

    // This site is after the original lock acquisition, including the failed-tryLock early return.
    @Inject(method = "startNextTask", at = @At(value = "INVOKE",
        target = "Lcom/seibel/distanthorizons/core/util/threading/PriorityTaskPicker;getExecutorIteratorSortedByShortestTotalRunTime()Ljava/util/Iterator;"),
        require = 1, allow = 1, remap = false)
    private void horizonBeginDispatchPass(boolean blocking, CallbackInfo callback) {
        horizonDispatch.beginPass();
    }

    @Redirect(method = "startNextTask", at = @At(value = "INVOKE",
        target = "Lcom/seibel/distanthorizons/core/util/threading/PriorityTaskPicker$Executor;canRun()Z"),
        require = 1, allow = 1, remap = false)
    private boolean horizonBeginExecutor(PriorityTaskPicker.Executor executor) {
        horizonDispatch.beginExecutor();
        return executor.canRun();
    }

    @Redirect(method = "startNextTask", at = @At(value = "INVOKE",
        target = "Lcom/seibel/distanthorizons/core/util/threading/PriorityTaskPicker$Executor;runTask(Ljava/lang/Runnable;)V"),
        require = 1, allow = 1, remap = false)
    private void horizonMarkRejected(PriorityTaskPicker.Executor executor, Runnable task) {
        try {
            executor.runTask(task);
        } catch (RejectedExecutionException rejected) {
            horizonDispatch.rejected();
            throw rejected; // Original catch requeues the same task or clears it during shutdown.
        }
    }

    @Redirect(method = "startNextTask", at = @At(value = "INVOKE",
        target = "Ljava/util/concurrent/atomic/AtomicInteger;get()I"), require = 1, allow = 1, remap = false)
    private int horizonLeaveRejectedExecutor(AtomicInteger counter) {
        int occupied = counter.get();
        return counter == occupiedThreadsRef
            ? horizonDispatch.visibleSlots(occupied) : occupied;
    }

    // Inspected DH 3.3.4 local slot 2 is the existing "executor paused" retry flag.
    @ModifyVariable(method = "startNextTask", at = @At("LOAD"), index = 2,
        require = 1, allow = 1, remap = false)
    private boolean horizonRetryRejectedPass(boolean original) {
        return horizonDispatch.needsRetry(original, isShutDownRef.get());
    }

    @ModifyConstant(method = "startNextTask", constant = @Constant(longValue = 1000L),
        require = 1, allow = 1, remap = false)
    private long horizonRetryDelay(long original) {
        return horizonDispatch.retryMilliseconds(original);
    }
}
