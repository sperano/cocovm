# Visual Machine Mode — design notes / TODO

Status: **far-future feature**, capturing the discussion so it's not lost.
Concept: a mode where the CoCo setup (console, monitor, disk drives, cartridge,
tape cassette, multipak, etc.) is shown as interactive visual objects — either to
configure the machine or just to represent the chosen config. The monitor screen
displays the live emulation output. Retro **pixel-art** look, not photorealistic.

## The concept splits into two features

1. **Configuration surface** — the scene is a live view of the machine spec.
   Drag a disk drive onto the desk → it's attached. Slide a cartridge into the
   slot → that ROM is mapped. Power switch → real reset. Genuinely useful.
2. **Skin / bezel** — pretty frame around the running screen. Pure atmosphere.

These cost very different amounts. Delight-per-effort is highest in (2), but (1)
is the one with lasting value.

## The one thing to get right NOW (before the feature exists)

Keep machine configuration as a **serializable data model** (`MachineConfig`),
decoupled from UI. The visual scene then becomes *a view over it* and drag-drop
*edits it*. If config stays scattered imperative setup code, the visual mode
forces a rewrite.

Model the machine as composable, hot-pluggable devices:
- cartridge slot: empty | ROM pak | multipak (which has its own 4 slots)
- drive controller: 0–4 floppies, each with an optional mounted disk
- RAM size: 128K / 512K / 2048K
- cassette: attached tape file + counter / play state
- monitor type: composite vs RGB (affects any later CRT treatment)

Action item when next touching machine wiring: land `MachineConfig` and route the
existing insert/eject/attach through it. Everything else is additive polish.

## Aesthetic direction (decided-ish)

- **Pixel art, retro.** Not photorealistic. This kills the hard 3D/shader cliff
  and softens the copyrighted-art problem (renditions are original artwork).
- **Curated palette, not 256 arbitrary colors.** 32–64 deliberate colors.
  Candidates: known palette (DB32, AAP-64) OR the **GIME's own 64-color palette**
  (self-referential — the room drawn in colors the CoCo can actually display).

### Must-get-right: pixel-grid unity
- Author the whole room at a fixed low virtual resolution (e.g. 320×200 or
  384×240), scale the whole frame to the window by **integer factors only**
  (2×/3×/4×), nearest-neighbor. Never fractional. egui's DPI/points scaling
  fights this — work in a virtual canvas and blit up, `TextureOptions::NEAREST`
  on every sprite.
- Snap dragged object positions to the virtual pixel grid (sub-pixel drag
  shimmers and breaks the illusion).
- Elegant target: **one CoCo pixel == one room pixel.** Emulator native output
  (256×192, 320×200, …) sits in the bezel at the same pixel scale as the room →
  the whole thing reads as one coherent pixel artifact.

## Open forks / decisions still to make

- **Flat/side view vs isometric.** Recommendation: **flat/side.** Preserves
  pixel-grid unity, keeps the emulator screen a clean integer rectangle, far less
  art labor. Isometric looks great but puts the screen on an angled plane that
  fights the pixel grid. Save iso for later, if ever.
- **Monitor screen treatment:** same flat pixel look as the room ("this whole
  thing is one game") vs a "CRT window" with subtle scanlines/glow ("a real CRT
  in a pixel-art world"). Undecided.

## egui implementation notes (when we build it)

- Interaction is the **easy** part. Drag/hover/click via `ui.interact` + `Response`
  (`dragged()`, `drag_delta()`, `hovered()`, `clicked()`). Built-in drop zones:
  `dnd_drag_source` / `dnd_drop_zone` fit cartridge-into-slot.
- Composite the emulator framebuffer into the bezel with `painter.image` using its
  `TextureId`.
- Treat the scene as a **custom canvas**: one `Painter`, own sprites; egui as the
  input+draw layer, skip widgets.
- The **actual work** is owning scene state in immediate mode (positions,
  attachments, in-flight drag) and reconciling every frame — i.e. it loops back to
  the `MachineConfig` data model. Clean state → easy interaction.
- Z-order/overlap: manage draw order + bring-to-top yourself. Annoying, not hard.
- Custom GPU rendering (CRT shader / 3D) = the one genuine cliff, via
  `PaintCallback` → wgpu. Optional and isolated; skip for pixel-art path.
- Animation: sprite-sheet frame loops (disk LED flicker, tape reels, power
  switch), not tweening. Drive LED off FDC/PIA access; motor whir via cpal.

## Hiring a pixel artist (when ready)

Venues, best-fit first:
- **Lospec** — pixel-art hub (palettes + community + job board); artists already
  think in fixed palettes/pixel density.
- **itch.io** — indie retro community; commissions + direct outreach to artists
  whose game art you like.
- **Reddit** — `r/gameDevClassifieds` (`[PAID]`), `r/PixelArt`, `r/HungryArtists`.
  Avoid `r/INAT` (rev-share, not paid).
- **Bluesky / X** — active community; best for targeted DM outreach to a style
  you like (`#pixelart`).
- **ArtStation** — pro tier, pricier. **Fiverr/Upwork** — bounded/variable, OK for
  a single test asset only.

How to hire well:
1. **One artist for the whole set** — consistency across sprites is the hard part.
2. **Lock constraints before briefing:** palette, virtual resolution/pixel
   density, flat-side view, light direction.
3. **Design a system, not N one-offs:** one cartridge *shell* + stampable label
   template + device sprites.
4. **Paid test first:** one cartridge + monitor bezel before the full set.
5. **Rights in writing:** work-for-hire / rights assignment (this may ship inside
   a distributed emulator).
- Note: AI pixel-art generators won't hold palette/grid consistency across a set.
- Timing: don't hire yet. Build a reference board of artists/styles now; hire once
  palette + pixel-density are locked (those *are* the brief).

## Traps to remember

- **Copyrighted cartridge art** — design for generated/placeholder labels
  (template + title text, or user-supplied art via sidecar file); real art is a
  local-only asset the user drops in. Don't bake a scraped-art pipeline into repo.
- **Scope creep into a full editor** — ship the config-as-data model first; add
  drag-drop to the most-used devices first.

## Next concrete steps (none urgent)

- [ ] When next touching machine wiring: introduce `MachineConfig` and route
      insert/eject/attach through it.
- [ ] Decide monitor-screen treatment (flat vs CRT-window).
- [ ] Pick the palette + virtual resolution (these become the artist brief).
- [ ] (optional early win) CRT/scanline treatment as a small isolated experiment.
- [ ] Throwaway egui spike: 2 draggable sprites + 1 drop zone wired to a stub
      `MachineConfig`, to feel the loop before committing to assets.
- [ ] Build artist reference board; write creative brief when palette/res locked.
