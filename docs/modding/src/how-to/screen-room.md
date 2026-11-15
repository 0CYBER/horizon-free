# Room beside a container

`Hooks::screen_room` lets a client mod reserve room for a panel to the right of an open container. This is a layout query, not a drawing or cancellation hook. It does not add recipe properties, slot membership or ghost-placement APIs (Refs arismoko/horizon#3194).

The API is right-only because the existing recipe book occupies the left of the container; mod panels reserve the right. Left reservations are not part of this API.

```rust,ignore
fn screen_room(&self, _: &dyn ClientHost, event: &mut ScreenRoomEvent<'_>) {
    let panel_width_with_gap = 149.0;
    event.right = event.right.max(panel_width_with_gap);
}
```

Import `ClientHost`, `Hooks` and `ScreenRoomEvent` from `mod_api` and implement the method in your subscribed `Hooks` implementation. Keep the query pure and cheap: do not draw, mutate game state or do expensive work here.

## Cooperation and fitting

The answer starts at zero. Later listeners see earlier answers and keep the maximum, not a sum. Panels asking for 80 and 149 GUI pixels reserve 149 pixels together, not 229. They share the reservation and must coordinate their own contents; the engine does not stack panels or assign per-mod offsets.

The query's `event.screen.rect` is the **pre-reservation** container rectangle. Later drawing and input `ScreenView` values expose the shifted rectangle. Do not subtract the shift yourself from those later views.

When the window, the open recipe book (including its category tabs), and the requested right room all fit, the window and book move left by half the requested room, rounded down. Slots, hit boxes and effect panels follow in the same frame. Otherwise nothing moves. A panel should check the later window rectangle's right edge plus its required room against the viewport before drawing there. A request is not a guarantee of space.

A negative or non-finite final answer counts as zero. Zero and absent listeners preserve the original layout. A finite request too large to fit also preserves it. The hook runs for the inventory, containers and custom menus, but not Creative, Merchant or Beacon screens, or a narrow screen where the recipe book covers the menu.

## Frame lifetime

The answer is queried at most once per frame and screen and reused by every layout consumer that frame. A recursive layout query during dispatch sees zero, avoiding recursive dispatch. The next frame queries again. A geometry-changing answer rebuilds the layout that frame; unchanged geometry reuses the existing build, even if the numerical answer changed without affecting the resulting layout.
