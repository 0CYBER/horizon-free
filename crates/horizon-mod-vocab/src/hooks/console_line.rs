//! The `console_line` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A line the server logged (what its console prints and
    /// `logs/latest.log` keeps): `level` is `INFO`, `WARN` or `ERROR`,
    /// `thread` the thread it names (`Server thread`), `message` the line
    /// without its time and thread. Heard at the start of the next server
    /// tick, in the order logged; the lines logged before the first tick a mod
    /// listens are not heard, and nor are the lines a `console_line`
    /// listener logs itself. At most 4096 lines wait between two ticks; the
    /// oldest are dropped with no notice. A `message` is cut to its first
    /// 8192 bytes and a `thread` to its first 128, at a character boundary.
    /// Server-side.
    #[notes = r#"## Console line

- **`console_line(host, event)`**: every line of the server's log, for a mod that mirrors the console elsewhere (a Discord console channel) or audits it. Paper has no event for this; plugins add a Log4j appender to the root logger. Lines are collected from every thread while a mod listens and handed over on the server thread at the next tick's start, so a listener may use the host. Each line is kept to 8192 bytes of message and 128 of thread name (cut at a character boundary), at most 4096 lines wait between two ticks, and the oldest are dropped, so the queue holds at most about 34 MB. A line logged inside the hook is not heard again, so a listener that logs what it hears does not feed itself."#]
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no log event; mods add a Log4j appender)",
        paper = "none (plugins add a Log4j appender)"
    )]
    fn console_line();
}
