This is the early project coco emulator in rust using egui

I want to build a super nice emulator juste like Virutal][, but For the Coco!
I'd like you to help me design it? 
How we would have the cpu loop, the display, the i/o, the memory, etc.

I'd like to start with emulating the GIME?

## Local resources (git-ignored — copyrighted, present only on this machine)

- `./docs/` — authoritative reference PDFs (6809/6309 instruction sets, MC6809
  programming manual, CoCo 3 Service Manual, Super Extended BASIC Unravelled II,
  memory maps). Verify hardware claims against these with
  `pdftotext -layout <pdf>` instead of guessing or web search.
- `./roms/` — real ROM images: `coco3.rom` (32K Super Extended Color BASIC,
  maps to `$8000–$FFFF`) and `disk11.rom` (8K Disk BASIC). Used to boot real
  code and trace-diff against XRoar/MAME. `crates/coco-core`'s boot tests read
  `roms/coco3.rom`.

Both directories are in `.gitignore`; don't commit their contents.
