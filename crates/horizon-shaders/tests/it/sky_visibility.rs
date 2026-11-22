//! Original property-to-render-contract controls; no pack content.
use horizon_shaders::{GameInputs, Macros, Pack, Properties, SkyVisibility, load_post};

#[test]
fn independent_sky_properties_reach_the_loaded_render_contract() {
    let root = std::env::temp_dir().join(format!("horizon-sky-properties-{}", std::process::id()));
    std::fs::create_dir_all(root.join("shaders")).unwrap();
    let pack = Pack::open(&root).unwrap();
    let macros = Macros::standard();
    for mask in 0u8..16 {
        let source = ["sky", "stars", "sun", "moon"]
            .into_iter()
            .enumerate()
            .map(|(i, key)| format!("{key}={}", mask & (1 << i) != 0))
            .collect::<Vec<_>>()
            .join("\n");
        let properties = Properties::parse(&source, &macros);
        let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
        assert_eq!(
            post.sky_visibility,
            SkyVisibility {
                sky: mask & 1 != 0,
                stars: mask & 2 != 0,
                sun: mask & 4 != 0,
                moon: mask & 8 != 0
            }
        );
    }
    for text in [
        "",
        "sky=unknown\nstars=off\nsun=FALSE\nmoon=0",
        "#if 0\nsky=false\nstars=false\nsun=false\nmoon=false\n#endif",
    ] {
        let post = load_post(
            &pack,
            "",
            &Properties::parse(text, &macros),
            &macros,
            &GameInputs::default(),
        );
        assert_eq!(post.sky_visibility, SkyVisibility::default());
    }
    let post = load_post(
        &pack,
        "",
        &Properties::parse("sun=false\nsun=true\nmoon = false  ", &macros),
        &macros,
        &GameInputs::default(),
    );
    assert!(post.sky_visibility.sun);
    assert!(!post.sky_visibility.moon);
    std::fs::remove_dir_all(root).unwrap();
}
