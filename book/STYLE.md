# Book style guide

This file governs the prose of every chapter in `book/`. It exists because the
first drafts read like lecture notes — technically dense, but compressed to the
point of being tiring. The target register is an O'Reilly-quality technical
book: a knowledgeable author writing for a professional audience, in full
sentences, with room to breathe. Expansion means richer explanation, not
padding.

## The audience

Experienced developers who know Rust (though not necessarily its advanced
corners) and 6809 assembly, are fond of the 8-bit era, have never written an
emulator, and have no graphics programming background. The book is written
for that audience, never for one individual:

- The impersonal, generic "you" of technical instruction is fine: "when you
  run the loop, you'll see the prompt." This is the normal O'Reilly register.
- Direct address that treats the reader as a specific person is not.
  No assumptions about the reader's personal history ("your CoCo 3," "the
  machine you grew up with"), no lifestyle asides ("read this in an
  armchair"), no author-to-reader chat.
- First-person singular never appears — there is no "I" in this book.
  "We" is reserved for author and audience walking through material
  together ("we now have a working bus"), used in moderation.

The book should still be *enjoyable* — readable away from the keyboard,
for pleasure as much as for learning.

## Voice and register

- **Full sentences, full paragraphs.** Paragraphs of three to six sentences
  are the default unit of writing. A sentence fragment is a spice, not a diet.
- **One idea per paragraph, with connective tissue.** Every section opens by
  saying why the reader should care and closes by handing off to what comes
  next. Never start a section cold with a definition or a code block.
- **Cut the tics of note-taking.** The drafts lean hard on em-dashes, nested
  parentheticals, bold-faced inline headings, and "X — because Y" compression.
  Unpack these into sentences. An em-dash pair per paragraph is plenty;
  a parenthetical inside a parenthetical is a bug.
- **Prose over bullets.** Use a list only when the content is genuinely
  enumerable (register tables, step sequences, decision criteria). If a bullet
  contains more than two sentences, it wants to be a paragraph.
- **Narrate the code.** Before a code excerpt: what to look for and why it's
  shaped that way. After: walk the interesting lines in prose. Never let two
  code blocks touch without prose between them.
- **Period context and concreteness are welcome.** The platform's shared
  culture (POKE 65495, the cassette relay click, ERROR ?IO), period context,
  and "what would happen if" thought experiments make the material stick —
  presented as facts about the CoCo world, not as claims about the reader's
  own memories. Dry humor in the O'Reilly tradition is fine; jokes that need
  a wink are not.
- **Explain like the reader is smart but new.** Never hand-wave with "simply"
  or "just"; never assume graphics, DSP, or emulation vocabulary that hasn't
  been introduced. Introduce a term in italics once, then use it plainly.

## Structure conventions (keep these)

- Numbered sections `N.1`, `N.2`, … with descriptive titles.
- The italic chapter opener stating the week and goal — but follow it with a
  real prose introduction; the opener is an epigraph, not the introduction.
- **Rust corner** sidebars for language points, blockquoted, as now.
- **Reading assignment** and **Exercises** sections at the end, then a short
  **What's next** closer that genuinely previews the next chapter's payoff.
- Exercise mix: build / sabotage / read / recall. Sabotage exercises state
  empirically verified outcomes — do not alter their claims without re-running
  the experiment.
- The course frame stays: chapters are weeks, cross-references say
  "week 12," the reading assignments are assignments.

## Technical ground rules (non-negotiable)

- Code excerpts are **verbatim** from this repository. When adding a new
  excerpt, copy it from the source file and cite the path and line numbers.
  Never retype from memory, never "clean up" the excerpt.
- Code references outside fences are GitHub links:
  `https://github.com/sperano/cocovm/blob/main/<path>#L<n>` (ranges `#La-Lb`),
  display text unchanged. No links inside code fences. Never link `roms/` or
  `docs/` (git-ignored). Bare filenames are linked only in Reading sections.
- `$` hex for 6809-side addresses and opcodes (`$FF92`); `0x` hex in Rust
  contexts.
- New technical claims must be traceable: to this repository's source and
  comments, to DESIGN.md, or to facts already established in an earlier
  chapter. Do not introduce new CoCo-specific hardware claims from general
  recall — if a claim can't be verified against the repo, leave it out.
- Chapter length: whatever the material honestly supports. Recent chapters
  land between 1,800 and 2,600 lines; a chapter that needs less should take
  less. Depth and readability first — never pad to hit a number.

## Litmus test

Read a section aloud. If it sounds like someone dictating flashcards, rewrite
it. If it sounds like a good colleague explaining at a whiteboard — complete
thoughts, natural pace, the occasional grin — it's right.
