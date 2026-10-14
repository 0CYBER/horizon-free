# Horizon's design system

One design language for the launcher, the website and the game's own menus (what "Menu style:
Horizon" draws). Vanilla's classic screens and the HUD are not part of it: they stay exactly
vanilla, because parity is the point of the game.

The tokens live in one place, `crates/horizon-theme`. The launcher (`launcher/src/theme.rs`)
and the game (`crates/horizon-client/src/ui_theme.rs`) read them as Rust constants, and the
website reads them as CSS custom properties generated from the same crate
(`cargo run -p xtask -- tokens` writes `website/src/tokens.css`; a test fails when the file is
out of date). Change a value in the crate and all three follow.

## Where it comes from

The direction was set by [mkdir.directory](https://mkdir.directory/), a directory site that is
almost nothing but type. What we take from it is its restraint rather than its content:

1. **Type is the interface.** One family, Geist, and its mono. Large, tight display type
   (medium weight, -2 % tracking) carries the hierarchy, so there is no need for boxes, icons or
   colour to say what matters. Instrument Serif is gone.
2. **Lists, not cards.** Content is a numbered list: a small mono index on the left (`01`,
   `02`…), the name large, a mono detail on the right, and a 1 px rule between rows. Versions,
   settings, worlds, servers, release notes and the parity table are all this one component.
3. **One accent, used once.** Neutrals do the work. The violet marks the single main action on a
   screen (Play, Download), focus, links and progress; selection is shown in ink, not in colour.
4. **Space instead of borders.** Generous margins (64 px on wide screens, 16 px on phones), no
   shadows, no gradients, no outlines around groups. A surface tint appears only under the
   pointer or behind something that must stand apart (a console, a dialog).
5. **Quiet motion.** Things arrive with a short fade and an 8 px rise, a few tens of
   milliseconds apart, eased out, never bouncy. Hovers fade a soft plate in behind the row.
6. **A signature.** A giant, faint wordmark: at the foot of the website, behind the launcher's
   opening and the game's title screen.

## Tokens

### Colour

Cool, slightly blue neutrals in two appearances with the same restraint, and Horizon's violet.
Every text colour passes WCAG AA (4.5:1) on every background it is used on; the tests in
`horizon-theme` check each pair in both palettes.

| Token        | Light     | Dark      | Use |
|--------------|-----------|-----------|-----|
| `page`       | `#fafafc` | `#0d0e13` | the background |
| `plate_soft` | `#f3f3f8` | `#181a22` | hover plate, fields, segmented track, console |
| `plate`      | `#ebecf4` | `#20222c` | pressed plate, switch off, the faint wordmark |
| `surface`    | `#ffffff` | `#16181f` | dialogs and menus that sit above the page |
| `rule`       | `#e2e3ea` | `#24262f` | 1 px rules between rows, tracks |
| `ink`        | `#16181f` | `#ecedf3` | text, selection, slider fill, the inverted segment |
| `ink_soft`   | `#3b3e49` | `#c5c7d1` | secondary text, values |
| `muted`      | `#646875` | `#8b8f9c` | labels, descriptions, mono indices and details |
| `accent`     | `#5b3fd6` | `#9a86ff` | the main action, links, focus, progress |
| `on_accent`  | `#ffffff` | `#0d0e13` | text on the accent |
| `danger`     | `#b4372a` | `#f07a68` | errors, the crash notice's dot |
| `success`    | `#1d7a48` | `#45c27a` | "Matches vanilla", "Ready" |

The game's menus draw over the live title backdrop, always in the dark palette, with the page
colour as a flat veil so text keeps its contrast whatever the world shows: `VEIL` (0.90) on
every page and in a world, `HOME_VEIL` (0.80) on the title's home page, which shows more of the
world and keeps to `ink` and `ink_soft`. Muted text over the backdrop is `BACKDROP_MUTED`
(`#9fa3ae`). A test holds every pair to AA over a backdrop of luminance 0.77 (the snowiest title
panorama measured reaches 0.49), on the bare veil and on a selected row's plate. Their plates
are the dark `ink` at 8 % and 14 % rather than `plate_soft` and `plate`: over the veil they come
out the same, and they stay visible over a bright backdrop.

### Type

Geist (400, 500, 600) and Geist Mono (400), bundled under the SIL Open Font License.

| Token      | App px | Web (min–max px) | Line | Tracking | Weight | Use |
|------------|--------|------------------|------|----------|--------|-----|
| `micro`    | 11     | 11               | 1.5  | +0.04em  | mono   | indices (`01`), counts, timestamps |
| `caption`  | 12     | 12               | 1.5  | 0        | 400    | footnotes, help under a control |
| `label`    | 13     | 13               | 1.5  | 0        | 400    | row labels, nav, buttons |
| `ui`       | 14     | 14               | 1.5  | 0        | 400    | body text in the apps |
| `body`     | 15     | 15–17            | 1.6  | 0        | 400    | paragraphs |
| `heading`  | 20     | 20–24            | 1.3  | -0.01em  | 500    | section heads |
| `title`    | 28     | 32–48            | 1.05 | -0.02em  | 500    | list item names |
| `display`  | 44     | 34–64            | 1.05 | -0.025em | 500    | page titles |
| `wordmark` | 220    | 18vw             | 0.8  | -0.04em  | 500    | the faint signature |

### Space, shape and lines

- Spacing scale: 4, 8, 12, 16, 24, 32, 48, 64, 96. Page gutter 64 (apps and wide web), 16 on
  phones. Text measure at most 56 characters for paragraphs.
- Radii: 8 for fields and small controls, 12 for plates (hover rows, console, dialogs), full for
  buttons, segmented controls and switches.
- Lines: 1 px, `rule`. No other borders.
- Heights: control 32, large control (Play) 40, segment 28, list row 64 (apps), 86 (web).

### Motion

| Token   | Duration | Easing                          | Use |
|---------|----------|---------------------------------|-----|
| `quick` | 150 ms   | `cubic-bezier(.4, 0, .2, 1)`    | hovers, presses, colour |
| `move`  | 240 ms   | `cubic-bezier(.23, 1, .32, 1)`  | selection sliding, switches, page changes |
| `enter` | 600 ms   | `cubic-bezier(.23, 1, .32, 1)`  | content arriving: opacity 0→1, rise 8 px |
| `stagger` | 40 ms  |                                 | delay between list rows arriving |

`prefers-reduced-motion` (web) turns `enter` into a plain fade.

## Components

- **Index list** (the signature): `micro` mono index in `muted` (left, 40 px column), the name
  in `title` (or `ui`/`label` for dense lists such as settings), details in `micro` mono
  `muted` on the right, a `rule` under every row. Hover: a `plate_soft` plate with radius 12
  fades in, extending 16 px past the row on each side, and the index turns `ink`.
- **Buttons**: pill shaped, `label` size, medium weight, 32 high (40 for the one large action).
  *Primary* is accent with `on_accent` text, at most one per screen. *Secondary* is
  `plate_soft` with `ink` text. *Ghost* is `muted` text that turns `ink` with a `plate_soft`
  plate under the pointer. Pressed: `plate` (or the accent at 85 %). Disabled: 40 % opacity.
- **Fields**: 36 high, `plate_soft`, radius 8, no border; focus draws a 1 px accent ring.
  Monospace for paths and commands.
- **Switch**: 36×20 pill, `plate` when off, accent when on, a white 16 px knob that slides over
  `move`.
- **Slider**: a 2 px `rule` track, the filled part `ink`, a 14 px `ink` knob. The value is
  written in mono beside the label, not on the track.
- **Segmented control / tabs**: a `plate_soft` pill; the selected segment is an `ink` pill with
  `page` text that slides over `move`.
- **Navigation**: text only. Launcher: a top bar with the mark and "Horizon" on the left and
  the pages on the right, `label` size, `muted`, the current one `ink` on a soft pill. Game
  menus keep their sidebar layout (it suits a controller), but as a plain column of text items
  under the mark, with no panel tint and no border: the current one `ink` on a soft pill that
  slides to it.
- **Page header**: `display` title, then one `body` line in `muted`, 56 characters at most.
- **Notices and dialogs**: a `plate_soft` plate, radius 12, 16–24 padding, no border; a 6 px dot
  in `danger` (or `accent`) leads the title. Actions are buttons in a row below.
- **Progress**: a 2 px `rule` track and an `accent` fill; indeterminate work shuttles a 30 %
  segment.
- **Console**: `plate_soft` plate, radius 12, `micro`/`caption` mono in `ink_soft`, newest line
  at the bottom, the command field inside the plate under a `rule`.
- **Empty states**: a short `muted` sentence and one secondary button, centred in the space.

## Surfaces

- **Launcher** (`launcher/`): top bar; Play is a page header (version as the title, its notes'
  first line under it), the client/server segmented control, the version picker and Play, then
  an index list of Resources, Minecraft folder (and the server rows). Versions and Patch notes
  are index lists with the release name as the title; Settings is an index list of switches and
  values. The opening is the faint wordmark drawn letter by letter, then the interface arrives.
  The sunset photograph is gone: the window is the page colour, light or dark with the system.
- **Website** (`website/`): the same page structure as before (what it is, screenshots,
  parity, launcher, patch notes, community), each section a `micro` mono label, a `display`
  heading and a short paragraph, its content an index list. Screenshots stay as images (radius
  12, no frame). Light and dark follow the system, with a toggle. A giant faint wordmark sits
  above the footer. No hero photograph: the first screen is the headline, one paragraph, the
  download button and the parity score as an index row.
- **Game menus** (Horizon style): the title backdrop under a page-colour veil; text sidebar;
  index lists for worlds and servers; options pages as index lists of rows (label left, value
  in mono, the control right). The pause menu is a centred column of plain rows over a darker
  veil.

## The mark

**Horizon line.** A sun half set below a horizon, drawn as three horizontal bars of 4, 8 and
10 units over a 12 unit line, on a 16 unit grid. The bars are the sun cut by the horizon into
slices, which also reads as layers of terrain and as a nod to voxels without drawing a block. It
is built on whole units of the 16 px grid so it stays sharp at 16 px, and it scales without
detail to 1024 px.

- Monochrome: the mark in `ink` on light, in `ink` (dark) on dark; the line can take the accent.
- App icon: the mark in `#ecedf3` with the line in `#9a86ff`, on a `#16181f` rounded square
  (macOS grid: 824 px tile with 185 px corners in a 1024 canvas; Windows and Linux use the full
  square with 22 % corners).
- Exports and where they go: `docs/media/brand/README.md`.
