pub mod client {
    pub use super::fog;
    pub mod camera {
        pub struct CameraEntity;
        pub struct FovPlayer;
    }
    pub mod first_person {
        #[derive(Clone, Copy)]
        pub struct Hand;
    }
    pub mod widgets {
        pub struct TextEdit;
    }
    pub mod hud_layout {
        pub struct HealthInputs;
        pub struct GuiDraw;
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum HeartKind {}
    }
}
