# Inclusive and accessible writing

Rules distilled from the Google Developer Documentation Style Guide's inclusive-documentation and accessibility pages. These matter because reader-facing prose reaches people of every background, ability, and first language — and because non-inclusive terms date a document badly.

## Inclusive language

- Avoid gendered language: "person-hours" not "man-hours," "humanity" not "mankind," "staff the desk" not "man the desk."
- Use singular "they" for any person whose pronouns you don't know; never "he/she" or "(s)he."
- Avoid ableist words:
  - Rec: "final check for completeness" | Not: "sanity check"
  - Rec: "baffling outliers" | Not: "crazy outliers"
  - Rec: "slows down the service" | Not: "cripples the service"
  - Rec: "placeholder" | Not: "dummy variable"
- Avoid violent or graphic metaphors:
  - Rec: "If the connection doesn't respond, check for errors." | Not: "If the connection hangs, check for errors."
  - Rec: "Point to **File**, and then click **New**." | Not: "Hover over File and hit New."
- Replace non-inclusive established terms; mention the old term once in parentheses only if readers need it for recognition:
  - allowlist (not whitelist), denylist (not blacklist)
  - primary/replica or controller/worker (not master/slave)
  - "Jenkins controller (master)" — old term parenthesized once, then dropped
- When code itself requires a non-inclusive term, keep it in code font and use the preferred term in prose: "Start the replica by using the `START SLAVE` statement."
- Disability: person-first unless the community prefers identity-first — "people with disabilities," not "the disabled." Don't call nondisabled people "normal" or "healthy." Avoid "suffering from," "wheelchair-bound," "victim of" — use "living with," "uses a wheelchair." No euphemisms ("differently abled," "special").
- Age: "older adults," not "the elderly" or "seniors."
- Avoid divisive framings: "native speaker," "first-class citizen."
- Use globally diverse, gender-neutral names in examples; avoid US-centric references.

## Accessible writing

These carry over directly to any prose, including chat answers:

- Break up walls of text with short paragraphs, headings, and lists.
- Keep sentences under about 26 words.
- Put the most important, distinguishing information first in each sentence and list item.
- Use parallel structure for similar list items.
- Avoid double negatives: "You can continue without a path," not "A missing path won't prevent you from continuing."
- Avoid ALL CAPS and camelCase in prose (screen readers may spell them out); official names and code excepted.
- Don't use "&" for "and" in prose or headings (fine when quoting a UI label or in code).
- Write link text that makes sense out of context; never "click here," "this link," or a bare URL.
- Use "see" for cross-references ("see Configuration options") — it's accessibility-fine.
- Refer to UI elements by their label, not their appearance or position: "Click **Notifications**," not "Click the bell icon."
- Avoid directional language ("above," "below," "on the right"); use "earlier," "preceding," "following," or link directly.
- Make each procedure step its own list item; introduce tables and lists with a lead-in sentence.
- Error-message guidance: state what went wrong and how to fix it.
- Don't put new information only in an image; always carry it in text.

## Docs-only mechanics

Relevant only when producing published HTML documentation, not markdown prose: semantic HTML (`em` over visual italics, real heading elements in order, no skipped levels), alt text on every image (empty alt for decorative), `th`/`scope` on tables, labels on form fields, keyboard reachability, 4.5:1 color contrast, no flashing content, captions and transcripts for media.
