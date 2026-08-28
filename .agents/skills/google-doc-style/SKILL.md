---
name: google-doc-style
description: "Write all reader-facing prose in the style of the Google Developer Documentation Style Guide. Use this skill whenever you produce explanatory text a person will read — answering questions, explaining concepts or code, summarizing findings, writing READMEs, reports, tutorials, how-tos, or any documentation — even when the user doesn't mention style, formatting, or documentation. It governs prose only and never changes code, identifiers, code comments, commit messages, or configuration."
license: MIT
metadata:
  author: wekoodo
  version: "1.1"
  based_on: "Google Developer Documentation Style Guide (Google LLC), CC BY 4.0"
  source: "https://developers.google.com/style"
---

# Google developer documentation style

Write reader-facing prose the way the Google Developer Documentation Style Guide prescribes: like a knowledgeable friend — conversational, direct, and precise, for a global audience. This skill distills that guide. Follow the core rules below in reader-facing prose (see Scope); open the reference files when a specific situation calls for detail.

## Scope

This skill governs **prose a person reads**: chat answers and explanations, summaries, reports, READMEs, docs, tutorials, and how-tos. Word-list substitutions apply to that prose only — never to identifiers, flags, paths, or quoted terms from the project.

It never changes:

- Code, identifiers, string literals, or code comments — write those to match the surrounding codebase.
- Commit messages, configuration files, or machine-read output.
- Quoted material, error messages, or command output that you report verbatim.
- Machine-parsed markdown (for example `AGENTS.md` and other agent contracts). Match the file's existing voice and structure; do not restyle it as public documentation.
- Tool-call arguments.

When rules conflict, the order of precedence is: an explicit style request from the user, then the constraints of your environment (for example, response-length or formatting rules from your system prompt), then this skill. This skill shapes *how* you write, not *how much* — keep whatever brevity your environment requires.

## Voice and tone

Aim for the middle path: not stuffy, not silly. A knowledgeable friend explains things plainly without performing.

- Use second person ("you"), not first person plural ("we"). "The user" means the reader's users, never the reader. If the user or environment sets a different grammatical person (team or product "we", a defined persona, legal first person), keep it.
- Be conversational without being frivolous: contractions are fine; slang, hype, pop-culture references, and exclamation marks are not.
- Don't call anything "simple," "easy," "quick," or "straightforward" — if the reader finds it hard, you've told them the problem is them.
- Don't say "please" in instructions: "Click **Save**," not "Please click **Save**."
- Don't use "let's," or filler like "note that," "as you can see," "at this time."
- Write timelessly: no "currently," "new," "soon," "as of this writing" — and never pre-announce future features.
- Don't anthropomorphize software: a service doesn't "think," "want," or "see"; it checks, requires, detects, returns.
- Avoid figurative language, idioms, and jargon — they confuse readers whose first language isn't English. Write around jargon or define it on first use.
- No superlatives or absolutes: "best," "fastest," "always," "never," or unqualified security guarantees.

**Too informal:** "Just garbage-collect and you're golden."
**Right:** "To clean up, call the `collectGarbage` method."
**Too formal:** "The invocation of the garbage-collection facility may be effected as follows."

## Grammar essentials

- **Active voice.** Say who does what: "The server sends a response," not "a response is sent."
- **Present tense.** "The command returns a list," not "will return." Future tense only for genuinely later events.
- **Conditions before instructions.** "To delete the document, click **Delete**" — the goal or condition comes first, so the reader knows whether the step is for them.
- **Precise modals.** "Must" for requirements, "can" for options, "might" for possibilities, "we recommend" for recommendations. Avoid "should," "could," "would," and "may" (policy only).
- **American spelling** (Merriam-Webster when in doubt).
- **Short sentences.** Under about 26 words; one idea per paragraph, key point first.
- **Keep helper words** that remove ambiguity: "if X, *then* Y"; "assumes *that* you"; "the rules *that* you defined"; "Start the profiler, *and then* run the app."
- **Spell out an abbreviation on first use** — "two-factor authentication (2FA)" — unless it's better known than its expansion (API, URL, HTML). Never "e.g.," "i.e.," or "etc." — write "for example," "that is," "such as."

For articles, capitalization, contractions, plurals, possessives, pronouns, claims, and jargon in depth, read [references/grammar.md](references/grammar.md).

## Formatting essentials

- **Sentence case for every heading and title.** Task headings start with a bare infinitive: "Create an instance," not "Creating an instance."
- **Numbered lists for sequences; bulleted lists for everything else.** One imperative action per step; introduce every list with a complete sentence ending in a colon; keep items parallel.
- **Serial comma.** "Servers, proxies, and load balancers."
- **Code font** (backticks) for code in text: commands, filenames, paths, API names, parameters, literal values, HTTP status codes.
- **Bold** for UI elements only: "click **Deploy**." Not for emphasis.
- **Descriptive link text.** Link the name of the thing — never "click here" or a bare URL in prose. "For more information, see X."
- **Spell out zero through nine**; numerals for 10 and up, and for every number with a unit ("8 bits," "64 GB" with a space).
- **Unambiguous dates.** "August 19, 2026" or ISO 8601 (2026-08-19) — never "8/19/26."
- **Avoid semicolons, parentheses for important information, "and/or," and slashes in prose.** Em dashes—without surrounding spaces—are fine for breaks.
- **Notices sparingly.** A note or warning loses force when every paragraph is one.

For headings, lists, procedures, tables, numbers, dates, and notices in depth, read [references/formatting.md](references/formatting.md). For punctuation edge cases (commas, hyphens, dashes, quotation marks), read [references/punctuation.md](references/punctuation.md).

## Word choices

A few high-frequency choices; the full table is in [references/word-list.md](references/word-list.md) — scan it whenever a term feels off:

| Use | Instead of |
|---|---|
| sign in | log in / login (verb) |
| click **OK** | click on **OK** |
| lets you | allows you to / enables you to |
| use | utilize / leverage |
| to | in order to |
| affect (verb) | impact (verb) |
| run | execute |
| stop, exit, cancel | kill / terminate / abort |
| earlier / later (docs, versions) | above / below, higher / lower |
| using / through | via |
| because | since (for causation) |
| although | while (for contrast) |
| email | e-mail |
| allowlist / denylist | whitelist / blacklist |

## Inclusive and accessible writing

Write so the widest audience can read you: no ableist terms ("sanity check" → "quick check"), no gendered defaults (use singular "they"), no violent metaphors ("hangs" → "stops responding"), replace non-inclusive terms ("master/slave" → "primary/replica"). Refer to UI by label, not position or appearance; keep link text meaningful out of context; avoid directional language ("above"/"below" → "earlier"/"following"). Detail: [references/inclusive-accessible.md](references/inclusive-accessible.md).

## Writing about code and interfaces

When prose refers to code or UI — placeholders, command-line syntax, UI navigation, what gets code font versus bold versus italics — follow [references/code-in-text.md](references/code-in-text.md). Two rules come up constantly:

- Never inflect a code item: "send a `POST` request," not "`POST` the data"; "`Intent` objects," not "`Intent`s."
- Placeholders are descriptive `UPPERCASE_WITH_UNDERSCORES`, each explained with "Replace the following:" — never `foo` or `x`.

Invented sample values are never real: example.com, dana@example.com, Example Organization, 192.0.2.0/24, gender-neutral names (Alex, Dana, Quinn). Facts the user or environment already supplied stay as given — do not replace them with sample data.

## Self-check

Before finishing, scan for leakage first, then the most common style slips:

1. Did I apply this skill outside reader-facing prose (see Scope)? If yes, revert that.
2. Did I replace facts the user or environment supplied with sample or example data? If yes, restore the real facts.
3. An exclamation mark, or the words "simply," "easy," "easily," "just," "please," or "currently."
4. A Title Case Heading, or a heading starting with an -ing verb.
5. Passive voice hiding the actor, or "will" where present tense works.
6. "We" where the reader is "you" and no user or environment voice says otherwise; "should" where it should be "must" or "can."
7. Code, a filename, or a command not in code font; bold used for emphasis.
8. A missing serial comma, or a bare URL in running text.

## Attribution

This skill distills the [Google Developer Documentation Style Guide](https://developers.google.com/style), created by Google LLC and used under the [Creative Commons Attribution 4.0 License](https://creativecommons.org/licenses/by/4.0/). The guide is the authority; when this skill and the guide disagree, the guide wins.
