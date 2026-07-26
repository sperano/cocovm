//! Deterministic coverage for the CoCo 1/2 fixed-VDG colour source
//! (`docs/coco12-plan.md` Phase 3), the MC6847/MC6847T1 font/lowercase
//! divergence, and the CoCo 3's own GIME-generated CoCo-compatible text.
//! Style mirrors `tests/render.rs`/`tests/render_graphics.rs`, but driven
//! through `Machine` (like `render.rs`'s
//! `text_renderer_follows_sam_page_register`) since the colour-source
//! dispatch lives in `lib.rs`, not `video.rs` itself.

#[path = "render_coco12/common.rs"]
mod common;

#[path = "render_coco12/coco3_compat_text.rs"]
mod coco3_compat_text;
#[path = "render_coco12/colour_source.rs"]
mod colour_source;
#[path = "render_coco12/mc6847_fonts.rs"]
mod mc6847_fonts;
