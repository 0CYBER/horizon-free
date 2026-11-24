# Textures made at run time

A client mod that makes its images while the game runs (an emoji set it downloaded, a panel, a gauge, glyphs from
its own font) creates each one under a name and draws it as a GUI sprite, with no resource reload. This is
vanilla's `DynamicTexture`: Java registers one with `TextureManager.register(id, texture)`, sends its pixels with
`DynamicTexture.upload()`, draws it with `GuiGraphics.blit(id, ...)` and frees it with `TextureManager.release(id)`.
In Horizon the image lives in a page of the GUI atlas, so it draws wherever a GUI sprite does.

Sunrise added this capability first, so the calls, `DynamicTextureBudget` and the refusals have Sunrise's names
and signatures: code that calls only these four and matches `DynamicTextureError` non-exhaustively reads the
same in both games. Sunrise's size classes (`DYNAMIC_TEXTURE_CLASSES`) and its `ClassChange` refusal have no
counterpart here, so a mod that names either builds only for Sunrise. The byte budget and the longest side are
Sunrise's too; the image budget is not: 1024 images a mod here, 16 there.

```rust,ignore
use mod_api::prelude::*;
use mod_api::{DynamicTextureError, Identifier};

const SMILE: &str = "mymod:emoji/smile";

/// Upload one emoji: 32 by 32 pixels, row by row from the top left, four bytes (RGBA) each.
/// `made` says whether this mod already created it.
fn upload_smile(
    host: &mut dyn ClientHost,
    made: &mut bool,
    rgba: &[u8],
) -> Result<(), DynamicTextureError> {
    let id = Identifier::parse(SMILE).unwrap();
    if *made {
        // Already created by this mod: give it the new pixels instead.
        return host.update_dynamic_texture(&id, 32, 32, rgba);
    }
    // `NameTaken` here means a pack's sprite holds the name: there is no image to update.
    host.create_dynamic_texture(&id, 32, 32, rgba)?;
    *made = true;
    Ok(())
}

fn gui_render_post(&self, _: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
    // The whole image (`src` in its own pixels), drawn over 9 by 9 GUI pixels.
    event
        .batch
        .sprite(SMILE, [0.0, 0.0, 32.0, 32.0], [8.0, 8.0, 9.0, 9.0], None);
}
```

The four calls are on the client host. Create, update and free change the game's atlas, so they take
`&mut dyn ClientHost`: call them from a hook that has it, such as `client_tick_post`, not from inside a draw. The
budget only reads, so a shared `&dyn ClientHost` is enough for it.

- **`create_dynamic_texture(id, width, height, rgba)`** makes the image. From then on its id is a GUI sprite of
  `width` x `height` pixels, and its pixels reach the GPU with the next frame.
- **`update_dynamic_texture(id, width, height, rgba)`** gives it new pixels. The size may change: the image stays
  where it is when the new size fits there, and moves when it is larger. Update when the image changed, not every
  frame: each update copies the pixels and uploads the image.
- **`free_dynamic_texture(id)`** frees it: the name is gone at once, and its room in the mod's half of the page and
  its share of the budget come back. An image never freed lasts until the game ends.
- **`dynamic_texture_budget()`** answers what the mod may still create, `DynamicTextureBudget { images, bytes }`.

## Where it draws

The id is a GUI sprite like any other:

- `DrawContext::sprite(id, src, dst, tint)` draws it; `src` is in the image's own pixels.
- `gui_sprite(&id)` finds where it lies in the atlas and its size.
- An inline atlas object in text, `TextContent::Object(TextObject::Atlas { atlas: "minecraft:gui".into(), sprite:
  "mymod:emoji/smile".into(), fallback: None })`, draws it in one 8 x 8 glyph cell. That is how a chat emoji works:
  in `client_chat_received`, replace `message` with a `Text` whose `:smile:` spans are such objects. The text box the
  player types in holds plain characters, so it shows `:smile:` until the message is sent.

Sampling is the atlas's: nearest, as GUI sprites. Each image keeps a transparent texel to its right and below it,
so its neighbours never bleed into its edges.

## Names

The id is in a namespace the calling mod owns: its own id (`mymod:...`) or another namespace its manifest declares;
the path is the mod's to choose. A mod never changes another's image: an id outside its namespaces, or `minecraft:`,
is `NotOwned`. An id that already names an image, or a GUI sprite of the packs, is `NameTaken` on create.

An id has two forms, `mymod:emoji/smile` and `mymod:gui/sprites/emoji/smile`: `DrawContext::sprite` and
`gui_sprite` find the first, and an inline atlas object of `minecraft:gui` looks for the second first. They don't
map one onto the other: a pack's sprite in the `mymod:gui/sprites/...` form is not found by `DrawContext::sprite`
or `gui_sprite` under the short id. So while
either form names an image or a pack's sprite, `create` refuses the other one too, whichever came first. That way
one id always draws one picture.

## The page and the budgets

The images live in one page of the GUI atlas, 2112 x 1056 texels (8.5 MiB), which the atlas holds only when a
client mod is loaded: a game without mods pays nothing. The page is two halves of 1056 x 1056, and all of a mod's
images go into one of them. A mod's first image picks the half that fewer mods hold images in, the left on a tie,
and its later images follow it there:

- the second mod always gets the half the first does not use, so with two mods neither can take room from the
  other;
- a third mod shares the half fewer mods use. That only happens while both halves hold other mods' images;
- a mod that frees all its images picks again with its next one.

In its half, each image takes its size plus one transparent texel to its right and one below it. An image takes the
smallest free space it fits, so images of one size fill the half row by row, and a short image beside a tall one
starts rows of its own next to it. When the mod's half has no space left for an image, `create` is `NoSlot`, and so
is an `update` that grows an image past what fits. Free an image, or make it smaller.

| Limit | Value | Refusal |
|---|---|---|
| Images a mod holds | `DYNAMIC_TEXTURE_MOD_IMAGES`, 1024 | `OverBudget` |
| Pixels a mod holds | `DYNAMIC_TEXTURE_MOD_BYTES`, 4 MiB (`width * height * 4` an image) | `OverBudget` |
| Longest side | `DYNAMIC_TEXTURE_MAX_SIDE`, 1024 pixels | `TooLarge` |

What a half holds for a mod that has it to itself, as each of two mods does:

- 1024 images of any one size up to 32 x 32, the whole image budget;
- a whole byte budget of images of one size whose sides are powers of two from 32 to 1024: 1024 emoji of 32 x 32,
  256 images of 64 x 64, 64 of 128 x 128, 16 of 256 x 256, 4 of 512 x 512, or one of 1024 x 1024. The same holds for
  sides that differ, such as two images of 1024 x 512.

Other shapes use the half less well, because the transparent texels cost more next to a short side. A strip of
1 x 1024 takes 2 x 1025 texels, so a half holds 528 of them, not the 1024 the byte budget allows. Mixed sizes leave
gaps between rows, so the half can be full before the budget is spent. Such a mod is refused with `NoSlot`, and
the other half's room does not change.

These are limits of the atlas page; vanilla's `TextureManager` has none, so read what remains with
`dynamic_texture_budget` rather than assume. The bytes are what the game keeps of each image's last pixels, to put
them back after a resource reload; an update counts its new size in place of the old, and a free gives its share
back.

## Refusals

Every call answers `Result<_, DynamicTextureError>`, and a refused call changes nothing:

| Refusal | When |
|---|---|
| `NotOwned` | the id is outside the calling mod's namespaces, or the image is another mod's |
| `BadSize` | a side is 0, or the pixels are not `width * height * 4` bytes |
| `TooLarge` | a side is longer than 1024 |
| `NoSlot` | the mod's half of the page has no room left for the image |
| `OverBudget` | the mod would pass 1024 images or 4 MiB |
| `NameTaken` | `create` of an id that names an image or a GUI sprite, in either form |
| `Unknown` | `update` or `free` of an id that names no image |
| `Unavailable` | no page here: a server, or no client mod loaded |

Match `DynamicTextureError` non-exhaustively. Sunrise's `ClassChange` is not here: its images live in slots of
fixed size classes, and an update across classes is refused there; here an update that does not fit moves.

## Resource reloads

A resource pack change rebuilds the GUI atlas. Every image keeps its name and its place in the new atlas's page,
and the game puts its last pixels back by itself, with the next frame: there is nothing to upload again. Ask
`gui_sprite` again after a reload if you kept a region: the page may lie elsewhere in the new atlas. Ask again
too after an update that changes the size, and after a free and a new create: a moved image leaves its old room
to the next one. If a resource
pack brings a sprite under an image's id, in either form, the pack's sprite keeps the name until that pack is
turned off again.

## Sprites made at load instead

An image that does not change while the game runs can be a runtime sprite instead
(`Hooks::register_runtime_sprites`): a PNG the atlas stitches at every load. Those are capped at 128 for all mods
together and change only with a resource reload; `mod_image` is one 1024 x 1024 slot shared by every mod. A
downloaded image set, or anything the mod makes after startup, belongs here.
