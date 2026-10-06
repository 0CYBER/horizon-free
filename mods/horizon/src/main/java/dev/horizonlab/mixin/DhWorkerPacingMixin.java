package dev.horizonlab.mixin;

import com.seibel.distanthorizons.core.config.types.ConfigEntry;
import dev.horizonlab.budget.WorkerPacer;
import java.util.concurrent.TimeUnit;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Pseudo;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Pseudo
@Mixin(targets = "com.seibel.distanthorizons.core.util.threading.RateLimitedThreadPoolExecutor", remap = false)
public abstract class DhWorkerPacingMixin {
    @Shadow @Final public ConfigEntry<Double> runTimeRatioConfig;
    @Unique private final ThreadLocal<WorkerPacer> horizonPacing = ThreadLocal.withInitial(WorkerPacer::new);

    // The inspected superclass's hooks are no-ops. Neither hook changes the runnable or task queue.
    @Inject(method = "beforeExecute", at = @At("HEAD"), cancellable = true, remap = false)
    private void horizonBeforeTask(Thread thread, Runnable task, CallbackInfo callback) {
        horizonPacing.get().begin(System.nanoTime());
        callback.cancel();
    }

    @Inject(method = "afterExecute", at = @At("HEAD"), cancellable = true, remap = false)
    private void horizonAfterTask(Runnable task, Throwable error, CallbackInfo callback) {
        WorkerPacer pacing = horizonPacing.get();
        long delay = pacing.finish(System.nanoTime(), runTimeRatioConfig.get());
        if (delay > 0) {
            try {
                TimeUnit.NANOSECONDS.sleep(delay);
                pacing.pauseCompleted(System.nanoTime());
            } catch (InterruptedException interrupted) {
                pacing.interrupted(System.nanoTime());
            }
        }
        callback.cancel();
    }
}
