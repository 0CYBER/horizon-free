package dev.horizonlab.metrics;

import java.util.LinkedHashMap;
import java.util.Map;

/** Copies a recording-start snapshot; never accesses a graphics device on a server thread. */
public final class ServerContext {
    private ServerContext() {}

    public static Map<String, Object> capture(String runtimeKind, Map<String, ?> clientContext) {
        Map<String, Object> result = new LinkedHashMap<>();
        String kind = switch (runtimeKind) {
            case "integrated", "dedicated" -> runtimeKind;
            default -> "unknown";
        };
        result.put("server_runtime_kind", kind);
        boolean captured = kind.equals("integrated") && clientContext != null;
        for (String key : new String[]{"gpu", "graphics_backend", "graphics_driver"}) {
            Object value = clientContext == null ? null : clientContext.get(key);
            if (!(value instanceof String text) || text.isBlank()) captured = false;
        }
        result.put("client_graphics_capture_status", captured ? "captured_at_recording_start" : "not_supplied");
        if (captured) for (String key : new String[]{"gpu", "graphics_backend", "graphics_driver"})
            result.put(key, clientContext.get(key));
        return Map.copyOf(result);
    }
}
