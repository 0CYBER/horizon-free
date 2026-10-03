package dev.horizonlab.client;

import java.util.LinkedHashMap;
import java.util.Map;

/** Optional Iris reflection keeps baseline clients independent of shader-mod classes. */
final class ShaderSettings {
    private ShaderSettings() {}

    static Map<String, Object> read(boolean irisPresent) {
        if (!irisPresent) return Map.of("iris_capture_status", "not_installed", "iris_shaders_enabled", false);
        try {
            Class<?> api = Class.forName("net.irisshaders.iris.api.v0.IrisApi");
            Object instance = api.getMethod("getInstance").invoke(null);
            boolean enabled = (Boolean) api.getMethod("isShaderPackInUse").invoke(instance);
            Map<String, Object> result = new LinkedHashMap<>();
            result.put("iris_capture_status", "available");
            result.put("iris_shaders_enabled", enabled);
            if (enabled) {
                Class<?> iris = Class.forName("net.irisshaders.iris.Iris");
                result.put("iris_shader_pack", iris.getMethod("getCurrentPackName").invoke(null));
            }
            return result;
        } catch (ReflectiveOperationException | RuntimeException | LinkageError unavailable) {
            return Map.of("iris_capture_status", "unavailable");
        }
    }
}
