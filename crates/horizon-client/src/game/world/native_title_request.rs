//! `host.request_title(request)`: a native mod that draws its own title main page (it cancels `screen_render_pre` on
//! the `title` screen) asks for what that page's buttons do. The request is queued, and the title's next frame runs
//! it through the button's own path (`Title::enter_singleplayer`, `open_new_world`, `open_server_list`,
//! `direct_join`, then `title_action` for a join, Options, Mods and Quit). Nothing happens inside the mod's call. Off the
//! title's main page, and for a `Connect` without the caller's own `[native] world_control`, the answer is a code
//! and nothing is touched.
use super::native_world_request::TitleState;
use super::*;
use mod_api::{NativeTitleResult, TitleRequest};

/// The queue's decision, apart from the game: `title` where the title is, `on_title` whether any title screen is up
/// with no world (where `MainPage` may go), whether a world entry is already waiting for the world resources
/// (`entering`), `granted` whether the caller may join a server (asked only for `Connect`, once the title and the
/// queue allow a request), and the waiting request.
fn queue(
    title: TitleState,
    on_title: bool,
    entering: bool,
    granted: impl FnOnce() -> bool,
    request: TitleRequest,
    pending: &mut Option<TitleRequest>,
) -> NativeTitleResult {
    if pending.is_some() || entering {
        return NativeTitleResult::Busy;
    }
    let here = match request {
        TitleRequest::MainPage => on_title,
        _ => title == TitleState::Main,
    };
    if !here {
        return NativeTitleResult::NotTitle;
    }
    if let TitleRequest::Connect(address) = &request {
        if !granted() {
            return NativeTitleResult::NotPermitted;
        }
        if !horizon_protocol::servers::is_valid_address(address) {
            return NativeTitleResult::InvalidRequest;
        }
    }
    *pending = Some(request);
    NativeTitleResult::Queued
}

impl Game<'_> {
    pub(in crate::game) fn request_title(&mut self, request: TitleRequest) -> NativeTitleResult {
        let title = self.title_state();
        let on_title = self.on_title_screen();
        let entering = self.pending_entry.is_some();
        // The queue slot is taken out so the grant can read the game beside it.
        let mut pending = self.native_title_request.take();
        let answer = queue(
            title,
            on_title,
            entering,
            || self.native_grant(|native| native.world_control).is_some(),
            request,
            &mut pending,
        );
        self.native_title_request = pending;
        answer
    }

    /// A title screen is up (the main page or any other) with no world and no in-game screen over it.
    fn on_title_screen(&self) -> bool {
        self.title.is_some() && self.screen == Screen::Playing
    }

    /// Whether `request` may still run where the title is: `MainPage` from any title screen, the rest on the main page.
    fn title_request_here(&self, request: &TitleRequest) -> bool {
        match request {
            TitleRequest::MainPage => self.on_title_screen(),
            _ => self.title_state() == TitleState::Main,
        }
    }

    /// After each window event: a queued request whose page has closed is dropped, so leaving the main page and
    /// coming back to it before the title's next frame does not run it on the new visit.
    pub(in crate::game) fn forget_stale_native_title_request(&mut self) {
        if self
            .native_title_request
            .as_ref()
            .is_some_and(|request| !self.title_request_here(request))
        {
            self.native_title_request = None;
        }
    }

    /// The title frame's turn for a queued request, run as the main page's button would run it. A request that
    /// outlives the main page (for `MainPage`: the title), or that meets a world entry already waiting for its
    /// resources, is dropped.
    pub(in crate::game) fn drain_native_title_request(
        &mut self,
        event_loop: &dyn EventLoopCtl,
    ) -> Result<()> {
        let Some(request) = self.native_title_request.take() else {
            return Ok(());
        };
        if !self.title_request_here(&request) || self.pending_entry.is_some() {
            return Ok(());
        }
        let Some(title) = self.title.as_mut() else {
            return Ok(());
        };
        let action = match request {
            TitleRequest::WorldList => {
                title.enter_singleplayer();
                None
            }
            TitleRequest::CreateWorld => {
                title.open_new_world();
                None
            }
            TitleRequest::ServerList => {
                title.open_server_list();
                None
            }
            TitleRequest::Connect(address) => title.direct_join(&address),
            TitleRequest::Options => Some(horizon_client::title::Action::Options),
            TitleRequest::Mods => Some(horizon_client::title::Action::Mods),
            TitleRequest::Quit => Some(horizon_client::title::Action::Quit),
            TitleRequest::MainPage => {
                title.set_mode(horizon_client::title::Mode::Main);
                None
            }
            _ => None,
        };
        match action {
            Some(action) => self.title_action(event_loop, action),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_main_page_queues_and_only_a_granted_mod_joins() {
        let mut pending = None;
        let ask = |title, entering, granted: bool, request, pending: &mut Option<TitleRequest>| {
            queue(
                title,
                title == TitleState::Main,
                entering,
                || granted,
                request,
                pending,
            )
        };
        // Off the main page, nothing is queued.
        assert_eq!(
            ask(
                TitleState::Elsewhere,
                false,
                true,
                TitleRequest::WorldList,
                &mut pending
            ),
            NativeTitleResult::NotTitle
        );
        // A join needs the caller's own world_control and an address Direct Connection takes.
        let join = |a: &str| TitleRequest::Connect(a.to_owned());
        assert_eq!(
            ask(
                TitleState::Main,
                false,
                false,
                join("play.example.net"),
                &mut pending
            ),
            NativeTitleResult::NotPermitted
        );
        assert_eq!(
            ask(
                TitleState::Main,
                false,
                true,
                join("bad host:x"),
                &mut pending
            ),
            NativeTitleResult::InvalidRequest
        );
        assert!(pending.is_none());
        // The buttons any player can click need no permission; one request at a time.
        assert_eq!(
            ask(
                TitleState::Main,
                false,
                false,
                TitleRequest::CreateWorld,
                &mut pending
            ),
            NativeTitleResult::Queued
        );
        assert_eq!(
            ask(
                TitleState::Main,
                false,
                true,
                join("play.example.net:25565"),
                &mut pending
            ),
            NativeTitleResult::Busy
        );
        assert_eq!(pending, Some(TitleRequest::CreateWorld));
        // MainPage goes from any title screen (a disconnect screen, the server list), not from a world.
        assert_eq!(
            queue(
                TitleState::Elsewhere,
                true,
                false,
                || false,
                TitleRequest::MainPage,
                &mut None
            ),
            NativeTitleResult::Queued
        );
        assert_eq!(
            queue(
                TitleState::Elsewhere,
                false,
                false,
                || false,
                TitleRequest::MainPage,
                &mut None
            ),
            NativeTitleResult::NotTitle
        );
        // A world entry waiting for its resources keeps the queue shut.
        assert_eq!(
            ask(TitleState::Main, true, true, TitleRequest::Quit, &mut None),
            NativeTitleResult::Busy
        );
    }
}
