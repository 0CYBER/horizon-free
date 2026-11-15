use super::*;

impl Game<'_> {
    /// `getEmptyBundleDescriptionTextHeight`'s lines: the empty bundle's description wrapped at
    /// the grid's width in this font.
    fn bundle_description_lines(&self) -> usize {
        let text =
            horizon_client::text::Text::translate("item.minecraft.bundle.empty.description", &[]);
        self.gui.atlas.font.split_lines(&text, 96.0).len()
    }

    /// `RenderTooltipEvent.Pre` for a tooltip of `lines` then `images`, hanging from `at`: the
    /// point a mod moved it to, `None` when one cancelled it. Only built when a mod listens.
    pub(in crate::game) fn tooltip_pre(
        &self,
        batch: &mut HudBatch,
        stack: Option<&ItemStack>,
        lines: &[(horizon_client::text::Text, u32)],
        images: &[TooltipImage],
        at: (f32, f32),
        screen: (f32, f32),
    ) -> Option<(f32, f32)> {
        if !mod_api::__private::listened(mod_api::HookId::RenderTooltipPre) {
            return Some(at);
        }
        let mut elements: Vec<TooltipElement> = lines
            .iter()
            .map(|(text, color)| {
                text.try_into_api()
                    .map(|text| TooltipElement::Text(text, *color))
            })
            .collect::<Result<_, _>>()
            .ok()?;
        elements.extend(images.iter().cloned().map(TooltipElement::Image));
        self.tooltip_pre_elements(batch, stack, &elements, at, screen)
    }
    pub(in crate::game) fn tooltip_pre_elements(
        &self,
        batch: &mut HudBatch,
        stack: Option<&ItemStack>,
        elements: &[TooltipElement],
        at: (f32, f32),
        screen: (f32, f32),
    ) -> Option<(f32, f32)> {
        let stack = stack.try_into_api().ok()?;
        let mut place = mod_api::TooltipPlace {
            x: at.0 as i32,
            y: at.1 as i32,
        };
        let before = place;
        let mut cancel = false;
        let mut draw = crate::game::host::draw::GameDrawContext::new(self, batch);
        self.native_render_tooltip_pre(mod_api::__private::RenderTooltipPreCall {
            draw: &mut draw,
            stack: stack.as_ref(),
            screen_width: screen.0 as i32,
            screen_height: screen.1 as i32,
            elements,
            place: &mut place,
            cancel: &mut cancel,
        });
        if cancel {
            return None;
        }
        Some(if place == before {
            at
        } else {
            (place.x as f32, place.y as f32)
        })
    }

    /// A stack's tooltip: its ordered lines and a bundle's contents
    /// (`ClientBundleTooltip`: the 4 wide grid, the `+N` of what it leaves
    /// out, and the weight bar).
    pub(in crate::game) fn draw_stack_tooltip(
        &mut self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        x: f32,
        y: f32,
    ) {
        self.draw_stack_tooltip_with(batch, stack, false, x, y, Vec::new());
    }
    /// [`Self::draw_stack_tooltip`] with `TooltipFlag.isCreative` set when
    /// `creative` (a creative tab's own slot), and `after_name` inserted from
    /// line 1, as the creative screen adds its tab names.
    pub(in crate::game) fn draw_stack_tooltip_with(
        &mut self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        creative: bool,
        x: f32,
        y: f32,
        after_name: Vec<(horizon_client::text::Text, u32)>,
    ) {
        let mut lines = self.stack_tooltip_lines(stack, false, creative);
        if lines.is_empty()
            && !horizon_client::item_tooltip::tooltip_display(stack).name_visible(creative)
        {
            return;
        }
        let after_name_index = lines.len().min(1);
        lines.splice(after_name_index..after_name_index, after_name);
        let mut component = None;
        self.native_tooltip_component(stack, &mut component);
        let images: Vec<TooltipImage> = tooltip_component_size(component)
            .and_then(|size| {
                Some(TooltipImage {
                    source: mod_api::TooltipImageSource::ItemPart(stack.try_into_api().ok()?),
                    size,
                })
            })
            .into_iter()
            .collect();
        let Some(elements) =
            self.gather_tooltip_components(stack, lines, images, (batch.width, batch.height))
        else {
            return;
        };
        if elements.is_empty() {
            return;
        }
        let screen = (batch.width, batch.height);
        let api_elements = elements
            .iter()
            .map(|e| match e {
                Element::Text(t, c) => t.try_into_api().map(|t| TooltipElement::Text(t, *c)),
                Element::Image(i) => Ok(TooltipElement::Image(i.clone())),
            })
            .collect::<Result<Vec<_>, _>>();
        let Ok(api_elements) = api_elements else {
            return;
        };
        let Some(at) = self.tooltip_pre_elements(batch, Some(stack), &api_elements, (x, y), screen)
        else {
            return;
        };
        let sizes: Vec<_> = elements
            .iter()
            .map(|e| match e {
                Element::Text(t, _) => [self.gui.atlas.font.width_text(t), 10.0],
                Element::Image(i) => i.size,
            })
            .collect();
        let (size, offsets) = layout(&sizes, true);
        let (w, h) = (size[0].ceil(), size[1].ceil());
        let (x, y) = tooltip_position(at, screen, (w, h));
        self.tooltip_box(batch, Some(stack), x, y, w, h);
        passes(
            &elements,
            &offsets,
            [x, y],
            [w, h],
            |element, image_pass, origin, size| match (element, image_pass) {
                (Element::Text(text, color), false) => {
                    batch.text_styled(&self.gui.atlas, text, origin[0], origin[1], *color, true);
                }
                (Element::Image(image), true) => {
                    self.draw_tooltip_image(batch, image, origin, size);
                }
                _ => {}
            },
        );
    }

    fn draw_tooltip_image(
        &mut self,
        batch: &mut HudBatch,
        image: &TooltipImage,
        origin: [f32; 2],
        size: [f32; 2],
    ) {
        match &image.source {
            mod_api::TooltipImageSource::ItemPart(stack) => {
                self.draw_tooltip_item_part(batch, stack, origin, size);
            }
            mod_api::TooltipImageSource::Bundle(stack) => {
                let Ok(stack) = crate::game::api_values::stack_value(stack) else {
                    return;
                };
                if let Some(layout) = horizon_client::item_tooltip::bundle_tooltip(
                    self.items(),
                    &stack,
                    origin[0],
                    origin[1],
                    size[0],
                    self.bundle_description_lines(),
                ) {
                    self.draw_bundle_tooltip(batch, &layout);
                    self.draw_bundle_selected_name(batch, &layout, origin[0], origin[1], size[0]);
                }
            }
        }
    }
    fn draw_tooltip_item_part(
        &mut self,
        batch: &mut HudBatch,
        stack: &mod_api::ItemStack,
        origin: [f32; 2],
        size: [f32; 2],
    ) {
        let Ok(stack) = crate::game::api_values::stack_value(stack) else {
            return;
        };
        self.native_tooltip_component_draw(&stack, batch, origin, size);
    }

    /// `RenderTooltipEvent.GatherComponents` for an item's tooltip: the
    /// lines and the images the item's part answered go to
    /// `render_tooltip_gather_components` as one list. A mod's `max_width`
    /// wraps the lines (`Font.split`; unset, -1, leaves them as they are); an
    /// image a mod gave an invalid size is dropped, as an invalid answer of
    /// the part is. `None` when a mod cancelled it, so nothing is drawn.
    pub(super) fn gather_tooltip_components(
        &self,
        stack: &ItemStack,
        lines: Vec<(horizon_client::text::Text, u32)>,
        images: Vec<TooltipImage>,
        (width, height): (f32, f32),
    ) -> Option<Vec<Element>> {
        let mut elements: Vec<TooltipElement> = lines
            .into_iter()
            .map(|(text, color)| {
                text.try_into_api()
                    .map(|text| TooltipElement::Text(text, color))
            })
            .collect::<Result<_, _>>()
            .ok()?;
        let index = usize::from(!elements.is_empty());
        let bundle = horizon_client::item_tooltip::bundle_tooltip(
            self.items(),
            stack,
            0.0,
            0.0,
            96.0,
            self.bundle_description_lines(),
        );
        let bundle = bundle
            .map(|layout| {
                stack.try_into_api().map(|stack| TooltipImage {
                    source: mod_api::TooltipImageSource::Bundle(stack),
                    size: [96.0, layout.height],
                })
            })
            .transpose()
            .ok()?;
        elements.splice(
            index..index,
            bundle.into_iter().chain(images).map(TooltipElement::Image),
        );
        let (mut max_width, mut cancel) = (-1, false);
        self.native_render_tooltip_gather_components(
            stack,
            width as i32,
            height as i32,
            &mut elements,
            &mut max_width,
            &mut cancel,
        );
        if cancel {
            return None;
        }
        let font = &self.gui.atlas.font;
        let mut result = Vec::new();
        for element in elements {
            match element {
                TooltipElement::Text(text, color) => {
                    let text = text.try_into_engine(&()).ok()?;
                    if max_width < 0 {
                        result.push(Element::Text(text.resolve_legacy(), color));
                    } else {
                        result.extend(
                            font.split(&text.resolve_legacy(), max_width as f32)
                                .into_iter()
                                .map(|line| Element::Text(line, color)),
                        );
                    }
                }
                TooltipElement::Image(image) => {
                    if let Some(size) = tooltip_component_size(Some(image.size)) {
                        result.push(Element::Image(TooltipImage { size, ..image }));
                    }
                }
            }
        }
        Some(result)
    }
}

pub(super) enum Element {
    Text(horizon_client::text::Text, u32),
    Image(TooltipImage),
}

fn passes<T>(
    elements: &[T],
    offsets: &[f32],
    origin: [f32; 2],
    size: [f32; 2],
    mut draw: impl FnMut(&T, bool, [f32; 2], [f32; 2]),
) {
    for image_pass in [false, true] {
        for (element, offset) in elements.iter().zip(offsets) {
            draw(element, image_pass, [origin[0], origin[1] + offset], size);
        }
    }
}

fn layout(sizes: &[[f32; 2]], extra_space: bool) -> ([f32; 2], Vec<f32>) {
    let mut height = if extra_space && sizes.len() > 1 {
        0.0
    } else {
        -2.0
    };
    let mut width = 0.0_f32;
    let mut top = 0.0;
    let mut offsets = Vec::new();
    for (index, size) in sizes.iter().enumerate() {
        offsets.push(top);
        width = width.max(size[0]);
        height += size[1];
        top += size[1] + if index == 0 && extra_space { 2.0 } else { 0.0 };
    }
    ([width, height], offsets)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "daily"]
    fn rendered_image_callback_receives_whole_box() {
        use mod_api::{
            ClientHost, Hooks, ItemPart, ModEntry, Mods, Registrar,
            RenderTooltipGatherComponentsEvent, TooltipComponentDrawEvent, TooltipComponentEvent,
        };
        use std::sync::atomic::{AtomicBool, Ordering};
        static DRAWN: AtomicBool = AtomicBool::new(false);
        #[derive(Clone)]
        struct Image;
        #[mod_api::horizon_part]
        impl ItemPart for Image {
            fn tooltip_component(
                &self,
                _: &dyn ClientHost,
                _: &mut TooltipComponentEvent<'_>,
            ) -> Option<[f32; 2]> {
                Some([70.0, 16.0])
            }
            fn tooltip_component_draw(
                &self,
                _: &dyn ClientHost,
                event: &mut TooltipComponentDrawEvent<'_>,
            ) {
                assert_eq!(event.origin, [22.0, 20.0]);
                assert_eq!(event.size, [70.0, 46.0]);
                DRAWN.store(true, Ordering::Relaxed);
            }
        }
        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let key = r.item_key("minecraft:stick");
                r.item_part(key, Image);
            }
            fn render_tooltip_gather_components(
                &self,
                _: &dyn ClientHost,
                event: &mut RenderTooltipGatherComponentsEvent<'_>,
            ) -> mod_api::Flow {
                assert!(matches!(event.elements[1], TooltipElement::Image(_)));
                let image = event.elements[1].clone();
                *event.elements = vec![
                    TooltipElement::Text(mod_api::Text::literal("A"), u32::MAX),
                    image,
                    TooltipElement::Text(mod_api::Text::literal("B"), u32::MAX),
                    TooltipElement::Text(mod_api::Text::literal("C"), u32::MAX),
                ];
                mod_api::Flow::Continue
            }
        }
        let mods: Mods = vec![ModEntry::new("image", Box::new(Mod))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                DRAWN.store(false, Ordering::Relaxed);
                game.draw_stack_tooltip(
                    &mut HudBatch::scratch([320, 240]),
                    &ItemStack::new("minecraft:stick", 1),
                    10.0,
                    20.0,
                );
                assert!(DRAWN.load(Ordering::Relaxed));
            });
        });
    }
    #[test]
    fn image_between_lines_draws_at_its_position_after_all_text() {
        let elements = [false, true, false, false];
        let mut calls = Vec::new();
        passes(
            &elements,
            &[0.0, 12.0, 28.0, 38.0],
            [4.0, 8.0],
            [70.0, 46.0],
            |e, image, at, size| match (e, image) {
                (false, false) => calls.push((false, at, size)),
                (true, true) => calls.push((true, at, size)),
                _ => {}
            },
        );
        assert_eq!(
            calls,
            vec![
                (false, [4.0, 8.0], [70.0, 46.0]),
                (false, [4.0, 36.0], [70.0, 46.0]),
                (false, [4.0, 46.0], [70.0, 46.0]),
                (true, [4.0, 20.0], [70.0, 46.0])
            ]
        );
    }

    #[test]
    fn mixed_box_measurements() {
        let sizes = [[70.0, 10.0], [12.0, 16.0], [30.0, 10.0], [40.0, 10.0]];
        assert_eq!(
            layout(&sizes, true),
            ([70.0, 46.0], vec![0.0, 12.0, 28.0, 38.0])
        );
        assert_eq!(
            layout(&sizes, false),
            ([70.0, 44.0], vec![0.0, 10.0, 26.0, 36.0])
        );
        assert_eq!(layout(&sizes[..1], true).0, [70.0, 8.0]);
    }
}

#[cfg(test)]
#[path = "tooltip_bundle_tests.rs"]
mod bundle_tests;
