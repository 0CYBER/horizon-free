package dev.horizonlab.dispatch.fixture;

import com.google.gson.JsonObject;
import com.mojang.renderpearl.api.device.BackendCreationException;
import com.mojang.renderpearl.api.device.GpuBackend;
import com.mojang.renderpearl.api.device.GpuDebugOptions;
import com.mojang.renderpearl.api.device.GpuDevice;
import net.minecraft.client.Minecraft;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

/** Private diagnostic wrapper. Calls the original operations and rethrows their errors. */
@Mixin(Minecraft.class)
public abstract class GraphicsBackendTraceMixin {
    private static void trace(GpuBackend backend, String operation, Throwable failure) {
        var result = new JsonObject();
        String name = backend.getName();
        result.addProperty("backend", name.equals("OpenGL") || name.equals("Vulkan") ? name : "other");
        result.addProperty("operation", operation);
        result.addProperty("result", failure == null ? "success" : "failure");
        if (failure != null) result.addProperty("exception_type", failure.getClass().getName());
        if (failure instanceof BackendCreationException backendFailure)
            result.addProperty("reason", backendFailure.getReason().name());
        System.out.println("HORIZON_GRAPHICS_BACKEND_ATTEMPT " + result);
    }

    @Redirect(method = "<init>", at = @At(value = "INVOKE",
        target = "Lcom/mojang/renderpearl/api/device/GpuBackend;loadLibrary()V"), require = 1, allow = 1)
    private void horizon$traceLibrary(GpuBackend backend) throws BackendCreationException {
        try {
            backend.loadLibrary();
            trace(backend, "load_library", null);
        } catch (BackendCreationException | RuntimeException | LinkageError | AssertionError failure) {
            trace(backend, "load_library", failure);
            throw failure;
        }
    }

    @Redirect(method = "<init>", at = @At(value = "INVOKE",
        target = "Lcom/mojang/renderpearl/api/device/GpuBackend;createDevice(Lcom/mojang/renderpearl/api/device/GpuDebugOptions;)Lcom/mojang/renderpearl/api/device/GpuDevice;"), require = 1, allow = 1)
    private GpuDevice horizon$traceDevice(GpuBackend backend, GpuDebugOptions options) throws BackendCreationException {
        try {
            GpuDevice device = backend.createDevice(options);
            trace(backend, "create_device", null);
            var info = device.getDeviceInfo();
            var metadata = new JsonObject();
            metadata.addProperty("gpu", info.name());
            metadata.addProperty("graphics_backend", info.backendName());
            metadata.addProperty("graphics_driver", info.driverInfo());
            System.out.println("HORIZON_GRAPHICS_DEVICE_CREATED " + metadata);
            return device;
        } catch (BackendCreationException | RuntimeException | LinkageError | AssertionError failure) {
            trace(backend, "create_device", failure);
            throw failure;
        }
    }
}
