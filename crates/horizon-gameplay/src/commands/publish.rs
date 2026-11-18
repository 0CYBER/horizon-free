//! `/publish` and `/unpublish`.
use super::host::{CommandHost, R};
use horizon_client::{
    command::{Arg, Context},
    text::{self, Style, Text},
};

/// What `/publish` and `/unpublish` need from the game: the LAN world it opens and closes.
pub trait PublishHost: CommandHost {
    /// The port the world is open on, if it is: the hosted server's published port, else the
    /// port of the LAN settings.
    fn lan_port(&self) -> Option<u16>;
    /// Opens the world to the LAN on `port` (guests may run commands when `guest_commands`):
    /// false when the game hosts no server or the server fails to publish.
    fn open_to_lan(&mut self, port: u16, guest_commands: bool) -> bool;
    /// Closes the world to the LAN: false when it was not open.
    fn close_to_lan(&mut self) -> bool;
}

/// `PublishCommand.publish` with `IntegratedServer.publishServer`.
pub fn publish(host: &mut dyn PublishHost, ctx: &Context) -> R<i32> {
    let allow = matches!(ctx.get("allowCommands"), Some(Arg::Bool(true)));
    let port = match ctx.get("port") {
        Some(Arg::Int(p)) => *p,
        // HttpUtil.getAvailablePort.
        _ => std::net::TcpListener::bind((
            if std::env::var_os("HORIZON_TEST_LOOPBACK").is_some_and(|v| v == "1") {
                "127.0.0.1"
            } else {
                "0.0.0.0"
            },
            0,
        ))
        .and_then(|l| l.local_addr())
        .map_or(25564, |a| i32::from(a.port())),
    };
    if let Some(port) = host.lan_port() {
        return Err(host.error("commands.publish.alreadyPublished.lan", &[&port]));
    }
    // A dedicated server is open from the start (`lan_port` above).
    if !host.open_to_lan(port as u16, allow) {
        return Err(host.error("commands.publish.failed", &[]));
    }
    let port_text = port.to_string();
    let link = Text::literal(port_text.clone()).with_style(&Style {
        color: Some(text::GREEN),
        hover: Some(std::sync::Arc::new(Text::translate("chat.copy.click", &[]))),
        ..Style::default()
    });
    host.feedback(Text::translate(
        "commands.publish.started.lan",
        &[Text::translate("chat.square_brackets", &[link])],
    ));
    Ok(port)
}

/// `/unpublish`.
pub fn unpublish(host: &mut dyn PublishHost) -> R<i32> {
    if !host.close_to_lan() {
        return Err(host.error("commands.unpublish.notPublished", &[]));
    }
    host.feedback(Text::translate("commands.unpublish.success", &[]));
    Ok(1)
}
