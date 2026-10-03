package dev.horizonlab.client;

import com.seibel.distanthorizons.api.interfaces.config.IDhApiConfig;
import com.seibel.distanthorizons.api.interfaces.config.IDhApiConfigValue;
import com.seibel.distanthorizons.api.interfaces.config.client.IDhApiAmbientOcclusionConfig;
import com.seibel.distanthorizons.api.interfaces.config.client.IDhApiGraphicsConfig;
import com.seibel.distanthorizons.api.interfaces.config.client.IDhApiMultiThreadingConfig;
import com.seibel.distanthorizons.api.enums.config.*;
import com.seibel.distanthorizons.api.enums.rendering.*;
import java.lang.reflect.Proxy;
import java.util.Map;

public final class DistantTerrainSettingsTest {
    @SuppressWarnings("unchecked")
    private static <T> T proxy(Class<T> type, java.lang.reflect.InvocationHandler handler) {
        return (T) Proxy.newProxyInstance(type.getClassLoader(), new Class<?>[]{type}, handler);
    }

    private static IDhApiConfigValue<?> effective(Object value) {
        return proxy(IDhApiConfigValue.class, (p, method, args) -> {
            if (method.getName().equals("getValue")) return value;
            throw new AssertionError("Capture must read effective overrides, never mutate settings: " + method);
        });
    }

    private static void check(boolean condition) { if (!condition) throw new AssertionError(); }

    public static void main(String[] args) {
        Map<String, Object> settings = Map.ofEntries(
            Map.entry("renderingEnabled", true), Map.entry("renderingMode", EDhApiRendererMode.DEFAULT),
            Map.entry("chunkRenderDistance", 256), Map.entry("horizontalQuality", EDhApiHorizontalQuality.MEDIUM),
            Map.entry("verticalQuality", EDhApiVerticalQuality.HIGH), Map.entry("maxHorizontalResolution", EDhApiMaxHorizontalResolution.BLOCK),
            Map.entry("transparency", EDhApiTransparency.COMPLETE), Map.entry("lodShading", EDhApiLodShading.AUTO),
            Map.entry("lodOnlyMode", false), Map.entry("useCameraPositionForQualityDropOff", true));
        var ao = proxy(IDhApiAmbientOcclusionConfig.class, (p, method, values) -> effective(true));
        var graphics = proxy(IDhApiGraphicsConfig.class, (p, method, values) ->
            method.getName().equals("ambientOcclusion") ? ao : effective(settings.get(method.getName())));
        var threading = proxy(IDhApiMultiThreadingConfig.class, (p, method, values) ->
            effective(method.getName().equals("threadCount") ? (Object) 2 : 0.05));
        var configs = proxy(IDhApiConfig.class, (p, method, values) ->
            method.getName().equals("graphics") ? graphics : threading);
        Map<String, Object> result = DistantTerrainSettings.read(configs);
        check(result.get("dh_capture_status").equals("available"));
        check(result.get("dh_lod_radius_chunks").equals(256));
        check(result.get("dh_horizontal_quality").equals("MEDIUM"));
        check(result.get("dh_vertical_quality").equals("HIGH"));
        check(result.get("dh_runtime_ratio").equals(0.05));
        check(result.get("dh_thread_count").equals(2));
        check(DistantTerrainSettings.read(null).equals(Map.of("dh_capture_status", "unavailable")));
        var missing = proxy(IDhApiConfig.class, (p, method, values) -> { throw new UnsupportedOperationException(); });
        check(DistantTerrainSettings.read(missing).equals(Map.of("dh_capture_status", "unavailable")));
        var changedApi = proxy(IDhApiConfig.class, (p, method, values) -> { throw new NoSuchMethodError(); });
        check(DistantTerrainSettings.read(changedApi).equals(Map.of("dh_capture_status", "unavailable")));
        check(ShaderSettings.read(false).equals(Map.of("iris_capture_status", "not_installed", "iris_shaders_enabled", false)));
        // Test classpath deliberately omits Iris; it must report unavailable rather than assume disabled.
        check(ShaderSettings.read(true).equals(Map.of("iris_capture_status", "unavailable")));
        System.out.println("Effective terrain settings and absent/unsupported shader APIs checked.");
    }
}
