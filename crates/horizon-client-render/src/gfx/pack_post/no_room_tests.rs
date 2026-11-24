//! A frame that finds no room in video memory for what the pack draws into is given up, keeps
//! nothing that failed, and the next frame makes it (#13624). `frame_targets::NO_ROOM` stands in
//! for the full device. Original fixture, no pack files.
use super::destination_tests::Fixture;
use super::*;
use crate::gfx::frame_targets::NO_ROOM;
use std::io::{Cursor, Write};

const VERTEX: &str = "#version 120\nvarying vec2 uv;\nvoid main() { uv=gl_MultiTexCoord0.xy; gl_Position=ftransform(); }\n";

/// An image of the render size, for `pack`.
const RELATIVE: &str = "true 1.0 1.0";

/// `final` shows (0.25, 0.5, 0.75): its own constant, or with `image` (its relative flag and
/// size) what a compute program stored into an image. Both declare a storage buffer.
fn pack(image: Option<&str>) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, source: &str| {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    };
    add("final.vsh", VERTEX);
    if let Some(size) = image {
        add(
            "shaders.properties",
            &format!(
                "bufferObject.0 = 16\nimage.spray = spraySamp RGBA RGBA16F HALF_FLOAT false {size}\n"
            ),
        );
        add(
            "composite.csh",
            "#version 430\nlayout(local_size_x = 1) in;\nconst ivec3 workGroups = ivec3(1, 1, 1);\n\
             layout(rgba16f) writeonly uniform image2D spray;\n\
             void main() { imageStore(spray, ivec2(0, 0), vec4(0.25, 0.5, 0.75, 1.0)); }\n",
        );
        add(
            "final.fsh",
            "#version 130\nuniform sampler2D spraySamp;\nvarying vec2 uv;\n\
             void main() { gl_FragColor = vec4(texelFetch(spraySamp, ivec2(0, 0), 0).rgb, 1.0); }\n",
        );
    } else {
        add("shaders.properties", "bufferObject.0 = 16\n");
        add(
            "final.fsh",
            "#version 120\nvoid main() { gl_FragColor = vec4(0.25, 0.5, 0.75, 1.0); }\n",
        );
    }
    let pack = horizon_shaders::Pack::from_zip_bytes("no-room", zip.finish().unwrap().into_inner())
        .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
    horizon_shaders::load_post(&pack, "", &properties, &macros, &Default::default())
}

/// A frame at `size` in a window of `window` pixels that draws the pack's colour everywhere.
fn draws(gpu: &Fixture, post: &mut PackPost, size: [u32; 2], window: [u32; 2]) {
    let pixels = gpu
        .frame(post, size, window, &mut |_, _, _, _| {})
        .expect("the frame is drawn");
    assert_eq!(pixels.len(), (window[0] * window[1]) as usize);
    for p in pixels {
        assert!(
            p[0].abs_diff(64) <= 1 && p[1].abs_diff(128) <= 1 && p[2].abs_diff(191) <= 1,
            "{p:?}, notes {:?}",
            post.notes
        );
    }
}

/// The same frame with no room for the first thing it makes: given up.
fn is_given_up(gpu: &Fixture, post: &mut PackPost, size: [u32; 2], window: [u32; 2]) {
    NO_ROOM.set(1);
    let frame = gpu.frame(post, size, window, &mut |_, _, _, _| {});
    assert_eq!(NO_ROOM.replace(0), 0, "nothing was made that could fail");
    assert!(frame.is_none(), "the frame was drawn");
}

/// The crash this guards against: an image of the render size found no room as the size
/// changed, stayed in place invalid, and the next frame's commands failed validation.
#[test]
fn a_frame_without_room_for_an_image_is_given_up_and_the_next_makes_it() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut post = gpu.build(pack(Some(RELATIVE)));
    draws(&gpu, &mut post, [16, 16], [16, 16]);
    is_given_up(&gpu, &mut post, [32, 32], [32, 32]);
    assert!(post.images[0].view.is_none());
    // The buffers of the old size were let go before the image was made: it never meets both.
    assert!(post.targets.is_none());
    draws(&gpu, &mut post, [32, 32], [32, 32]);
}

#[test]
fn a_frame_without_room_for_the_storage_buffers_is_given_up_and_the_next_makes_them() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut post = gpu.build(pack(None));
    is_given_up(&gpu, &mut post, [16, 16], [16, 16]);
    assert!(post.buffers.group.is_none());
    draws(&gpu, &mut post, [16, 16], [16, 16]);
}

#[test]
fn a_frame_without_room_for_the_buffers_is_given_up_and_the_next_makes_them() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut post = gpu.build(pack(None));
    draws(&gpu, &mut post, [16, 16], [16, 16]);
    is_given_up(&gpu, &mut post, [32, 32], [32, 32]);
    assert!(post.targets.is_none());
    draws(&gpu, &mut post, [32, 32], [32, 32]);
}

/// `final`'s picture, made where the render size is not the window's.
#[test]
fn a_frame_without_room_for_final_s_picture_is_given_up_and_the_next_makes_it() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut post = gpu.build(pack(None));
    draws(&gpu, &mut post, [16, 16], [16, 16]);
    is_given_up(&gpu, &mut post, [16, 16], [32, 32]);
    assert!(post.final_target.is_none());
    draws(&gpu, &mut post, [16, 16], [32, 32]);
}

/// A scaled screenshot's frame makes what it draws into without scopes of its own, and its own
/// scopes found no room: the image of a size of its own and the storage buffers it made may be
/// invalid, and are dropped for the next frame to make, not kept for every frame after.
#[test]
fn a_failed_screenshot_frame_keeps_no_fixed_size_image_or_buffer_it_made() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut post = gpu.build(pack(Some("false 4 4")));
    crate::gfx::frame_targets::in_screenshot(|| draws(&gpu, &mut post, [32, 32], [32, 32]));
    post.screenshot_checked(true);
    assert!(post.images[0].view.is_none());
    assert!(post.buffers.group.is_none());
    draws(&gpu, &mut post, [16, 16], [16, 16]);
}

/// Video memory that stays full: the pack says so after `OUT_OF_ROOM_FRAMES` frames given up
/// in a row, for the game to turn it off, and not while a frame in between still drew.
#[test]
fn a_pack_without_room_frame_after_frame_is_out_of_room() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut post = gpu.build(pack(Some(RELATIVE)));
    draws(&gpu, &mut post, [16, 16], [16, 16]);
    for _ in 1..super::targets::OUT_OF_ROOM_FRAMES {
        is_given_up(&gpu, &mut post, [32, 32], [32, 32]);
        assert!(!post.out_of_room());
    }
    // A frame that found room starts the count again.
    draws(&gpu, &mut post, [32, 32], [32, 32]);
    for _ in 1..super::targets::OUT_OF_ROOM_FRAMES {
        is_given_up(&gpu, &mut post, [16, 16], [16, 16]);
        assert!(!post.out_of_room());
    }
    is_given_up(&gpu, &mut post, [16, 16], [16, 16]);
    assert!(post.out_of_room());
    // It kept nothing of what failed for the game to free but the pack itself.
    assert!(post.images[0].view.is_none() && post.targets.is_none());
}
