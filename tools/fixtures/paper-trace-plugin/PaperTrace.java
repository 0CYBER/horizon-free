import java.io.*;
import java.nio.file.*;
import org.bukkit.Bukkit;
import org.bukkit.configuration.file.YamlConfiguration;
import org.bukkit.block.data.type.Farmland;
import org.bukkit.event.block.MoistureChangeEvent;
import org.bukkit.event.*;
import org.bukkit.event.player.*;
import org.bukkit.event.world.ChunkLoadEvent;
import org.bukkit.plugin.java.JavaPlugin;

/** Startup observer; quit ends the window, join validates the bot session. */
public final class PaperTrace extends JavaPlugin implements Listener {
    private BufferedWriter output;
    private YamlConfiguration config;
    @Override public void onEnable() {
        try {
            output = Files.newBufferedWriter(Path.of(System.getProperty("paper.trace")));
        } catch (IOException e) { throw new UncheckedIOException(e); }
        config = YamlConfiguration.loadConfiguration(new File(System.getProperty("paper.trace.config")));
        Bukkit.getPluginManager().registerEvents(this, this);
    }
    private void emit(String name, String fields) { emit(name, fields, false); }
    private void emit(String name, String fields, boolean cancelled) {
        try {
            output.write("{\"tick\":" + Bukkit.getCurrentTick() + ",\"name\":\"" + name
                + "\",\"fields\":" + fields + ",\"cancelled\":" + cancelled + "}\n");
            output.flush();
        } catch (IOException e) { throw new UncheckedIOException(e); }
    }
    @EventHandler(priority = EventPriority.MONITOR)
    public void chunk(ChunkLoadEvent e) {
        emit("ChunkLoadEvent", "{\"world\":\"" + e.getWorld().getKey()
            + "\",\"x\":" + e.getChunk().getX() + ",\"z\":" + e.getChunk().getZ()
            + ",\"new\":" + e.isNewChunk() + "}");
    }
    @EventHandler(priority = EventPriority.HIGHEST)
    public void held(PlayerItemHeldEvent e) {
        if (e.getNewSlot() == config.getInt("player_switch_hotbar_slot_pre.new", -1)) e.setCancelled(true);
        emit("PlayerItemHeldEvent", "{\"old\":" + e.getPreviousSlot()
            + ",\"new\":" + e.getNewSlot() + "}", e.isCancelled());
        Bukkit.getScheduler().runTask(this, () -> emit("_slot", "{\"slot\":"
            + e.getPlayer().getInventory().getHeldItemSlot() + "}"));
    }
    @EventHandler(priority = EventPriority.HIGHEST)
    public void moisture(MoistureChangeEvent e) {
        var b = e.getBlock();
        var pos = java.util.List.of(b.getX(), b.getY(), b.getZ());
        if (pos.equals(config.getIntegerList("moisture_change.pos"))) e.setCancelled(true);
        emit("MoistureChangeEvent", "{\"pos\":" + pos + ",\"from\":"
            + ((Farmland)b.getBlockData()).getMoisture() + ",\"to\":"
            + ((Farmland)e.getNewState().getBlockData()).getMoisture() + "}", e.isCancelled());
    }
    @EventHandler public void join(PlayerJoinEvent e) {
        emit("_join", "{}");
        if (config.contains("window_ticks")) {
            Bukkit.getScheduler().runTaskLater(this, () -> {
                Bukkit.dispatchCommand(Bukkit.getConsoleSender(), "gamerule random_tick_speed 1024");
                emit("_window_start", "{}");
            }, 40);
            Bukkit.getScheduler().runTaskLater(this, () -> {
                Bukkit.dispatchCommand(Bukkit.getConsoleSender(), "gamerule random_tick_speed 0");
                emit("_window_end", "{}");
            }, 40 + config.getInt("window_ticks"));
        }
    }
    @EventHandler public void quit(PlayerQuitEvent e) { emit("_quit", "{}"); }
    @Override public void onDisable() {
        try { if (output != null) output.close(); }
        catch (IOException e) { throw new UncheckedIOException(e); }
    }
}
