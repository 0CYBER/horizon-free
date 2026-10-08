package dev.horizonlab.metrics;

import com.google.gson.GsonBuilder;
import java.io.BufferedWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Instant;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.CompletableFuture;

public final class Reports {
    private Reports() {}

    public static CompletableFuture<Path> write(Recording.Snapshot snapshot) {
        return CompletableFuture.supplyAsync(() -> {
            try {
                Files.createDirectories(snapshot.output());
                String name = snapshot.label() + "-" + snapshot.kind() + "-" + Instant.now().toEpochMilli();
                Path csv = snapshot.output().resolve(name + ".csv");
                try (BufferedWriter writer = Files.newBufferedWriter(csv, StandardCharsets.UTF_8)) {
                    writer.write("sample,duration_ns\n");
                    for (int i = 0; i < snapshot.samples().length; i++) {
                        writer.write(i + "," + snapshot.samples()[i] + "\n");
                    }
                }
                SampleStats stats = SampleStats.of(snapshot.samples(), snapshot.samples().length);
                Map<String, Object> report = new LinkedHashMap<>(snapshot.metadata());
                report.put("stats", stats);
                report.put("samples_file", csv.getFileName().toString());
                report.put("note", snapshot.kind().equals("frames")
                    ? "Frame start intervals include frame limiting and presentation waits; menus, pauses and unfocused frames are excluded. Not GPU timings. Process RSS, when available, is sampled separately."
                    : "Tick execution durations exclude scheduler sleep. meanRate is execution capacity, not observed TPS.");
                Path json = snapshot.output().resolve(name + ".json");
                Files.writeString(json, new GsonBuilder().setPrettyPrinting().create().toJson(report) + "\n");
                return json;
            } catch (Exception exception) {
                throw new IllegalStateException("Could not write Horizon benchmark", exception);
            }
        });
    }
}
