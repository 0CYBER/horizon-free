//! The window's root: what is behind the launcher (the sunset scene or a theme's painted sky,
//! Hypr Neon's rain, the dimming away from Home) and the launcher over it, as two sibling views.
//! A view's frame dirties the views it sits in, so a drifting sky inside the launcher drew the
//! whole launcher again on every sky frame. As siblings, a sky frame draws the sky and this thin
//! root; the launcher is drawn from its last frame (GPUI Fast retains views).
use crate::{rain, scene, sky, Launcher};
use gpui::{div, prelude::*, Context, Entity, Hsla, IntoElement, Render, Window};

/// What is behind the launcher, as its last frame asked for.
#[derive(Clone, Copy, PartialEq)]
pub struct Back {
    /// Behind everything, where the sky leaves gaps.
    pub side: Hsla,
    /// The sunset scene (the glass look's own) rather than a theme's sky.
    pub sunset: bool,
    pub rains: bool,
    /// The sunset dimmed behind the work, away from Home.
    pub dim: bool,
}

pub struct Backdrop {
    back: Option<Back>,
    scene: Entity<scene::Scene>,
    sky: Entity<sky::SkyView>,
    rain: Entity<rain::Rain>,
}

impl Backdrop {
    pub fn new(
        scene: Entity<scene::Scene>,
        sky: Entity<sky::SkyView>,
        rain: Entity<rain::Rain>,
    ) -> Self {
        Self {
            back: None,
            scene,
            sky,
            rain,
        }
    }

    /// Called from the launcher's frame: draws again only when something changed.
    pub fn set(&mut self, back: Back, cx: &mut Context<Self>) {
        if self.back != Some(back) {
            self.back = Some(back);
            cx.notify();
        }
    }
}

impl Render for Backdrop {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut back = div().absolute().top_0().left_0().size_full();
        let Some(b) = self.back else {
            return back;
        };
        back = back.bg(b.side);
        back = if b.sunset {
            back.child(self.scene.clone())
        } else {
            back.child(self.sky.clone())
        };
        back.when(b.rains, |d| d.child(self.rain.clone()))
            .when(b.dim, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(gpui::black().opacity(0.38)),
                )
            })
    }
}

pub struct Shell {
    backdrop: Entity<Backdrop>,
    launcher: Entity<Launcher>,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let launcher = cx.new(|cx| Launcher::new(window, cx));
        let backdrop = launcher.read(cx).backdrop.clone();
        Self { backdrop, launcher }
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .child(self.backdrop.clone())
            .child(self.launcher.clone())
    }
}
