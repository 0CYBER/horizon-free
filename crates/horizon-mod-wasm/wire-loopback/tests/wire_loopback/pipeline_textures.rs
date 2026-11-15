use super::*;
use mod_api::__private::game::RegistrationContextNew;
use mod_api::{Identifier, PipelineBlend, PipelineTexture, RenderPipeline, WorldDepth};

fn id(text: &str) -> Identifier {
    Identifier::parse(text).unwrap()
}

/// The pipelines a mod registers: a mod's sprite, a vanilla item sprite, and one untextured.
fn pipelines() -> [RenderPipeline; 3] {
    let pipeline = |name: &str| {
        RenderPipeline::new(
            id(name),
            id(name),
            PipelineBlend::Additive,
            WorldDepth::Tested,
        )
    };
    [
        pipeline("wire_test:orb").textured(PipelineTexture::Sprite(id("wire_test:block/charge"))),
        pipeline("wire_test:glow")
            .textured(PipelineTexture::Sprite(id("minecraft:item/diamond")))
            .optional(),
        pipeline("wire_test:plain"),
    ]
}

/// Every named texture refusal crosses the wire as itself.
#[test]
fn pipeline_texture_refusals_round_trip() {
    use mod_api::PipelineTextureError as E;
    for cause in [
        E::NoSuchSprite,
        E::NoSuchTextureFile,
        E::NotASpriteKey,
        E::OverCap,
        E::Undecodable,
        E::NoSlot,
    ] {
        let mut out = Writer::new();
        out.put(&cause);
        assert_eq!(Reader::decode::<E>(out.bytes()).unwrap(), cause);
    }
}

#[test]
fn pipeline_textures_round_trip() {
    for pipeline in pipelines() {
        let mut out = Writer::new();
        out.put(&pipeline);
        assert_eq!(
            Reader::decode::<RenderPipeline>(out.bytes()).unwrap(),
            pipeline
        );
        if let Some(texture) = &pipeline.texture {
            let mut out = Writer::new();
            out.put(texture);
            assert_eq!(
                &Reader::decode::<PipelineTexture>(out.bytes()).unwrap(),
                texture
            );
        }
    }
}

pub(super) struct PipelineTexturesHooks;
impl PipelineTexturesHooks {
    pub(super) fn register_render_pipelines(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterRenderPipelinesEvent<'_>,
    ) {
        event.pipelines.extend(pipelines());
    }
}

struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn register_render_pipelines(
        &self,
        cx: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterRenderPipelinesEvent<'_>,
    ) {
        PipelineTexturesHooks.register_render_pipelines(cx, event);
    }
}

#[test]
fn a_wasm_mods_textured_pipelines_reach_the_host_as_a_native_mods_do() {
    for guest in [false, true] {
        let hooks: Box<dyn Hooks> = if guest {
            Box::new(WasmMod {
                group: 0,
                subscribed: &[Some(mod_api::HookId::RegisterRenderPipelines)],
                listeners: &[],
                corrupt: false,
            })
        } else {
            Box::new(Native)
        };
        mod_api::testing::with_mods(
            vec![mod_api::ModEntry::new("wire_test", hooks)].into(),
            || {
                let mut registered = Vec::new();
                mod_api::dispatch_register_render_pipelines(
                    &mod_api::RegistrationContext::new(),
                    &mut registered,
                );
                let textures: Vec<_> = registered.iter().map(|p| p.texture.clone()).collect();
                assert_eq!(
                    textures,
                    [
                        Some(PipelineTexture::Sprite(id("wire_test:block/charge"))),
                        Some(PipelineTexture::Sprite(id("minecraft:item/diamond"))),
                        None,
                    ],
                    "guest {guest}"
                );
                assert_eq!(
                    registered.iter().map(|p| p.optional).collect::<Vec<_>>(),
                    [false, true, false],
                    "guest {guest}"
                );
                assert!(registered.iter().all(|p| p.owner() == "wire_test"));
            },
        );
    }
}
