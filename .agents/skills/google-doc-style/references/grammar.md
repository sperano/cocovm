# Language, grammar, and principles

Detailed rules distilled from the Google Developer Documentation Style Guide. "Rec:" = recommended, "Not:" = not recommended.

## Contents

- [Voice, person, and mood](#voice-person-and-mood)
- [Tense](#tense)
- [Prescriptive wording: must, can, might](#prescriptive-wording-must-can-might)
- [Timeless writing](#timeless-writing)
- [Sentence structure](#sentence-structure)
- [Writing for a global audience](#writing-for-a-global-audience)
- [Claims and objectivity](#claims-and-objectivity)
- [Jargon](#jargon)
- [Abbreviations](#abbreviations)
- [Capitalization](#capitalization)
- [Contractions](#contractions)
- [Pronouns](#pronouns)
- [Plurals and possessives](#plurals-and-possessives)
- [Articles and prepositions](#articles-and-prepositions)
- [Reference-doc verbs](#reference-doc-verbs)

## Voice, person, and mood

- Use active voice; make the actor explicit.
  - Rec: "Send a query to the service. The server sends an acknowledgment." | Not: "The service is queried, and an acknowledgment is sent."
- Never patch passive voice with "by" phrases — recast as active.
- Passive is acceptable in three cases: to emphasize the object ("The file is saved"), to de-emphasize the actor ("Over 50 conflicts were found in the file" — kinder than "You created over 50 conflicts"), or when the actor is irrelevant ("The database was purged in January").
- Address the reader as "you"; use imperative mood for instructions ("Click **Submit**").
- "The user" means the reader's users, never the reader.
  - Rec: "This document shows you how to develop an app for your organization." | Not: "This document shows the user how to develop an app."
- First person plural only when speaking as the organization ("we recommend"); never "let's."
  - Rec: "Consider adding a description to your table." | Not: "Let's add a description to our table."
- Don't anthropomorphize software: a system doesn't "see," "think," "want," or "tell" — it detects, checks, requires, specifies.
  - Rec: "The PC detects a new device." | Not: "The PC sees a new device."

## Tense

- Present tense for product behavior and general statements.
  - Rec: "The server sends an acknowledgment." | Not: "The server will send an acknowledgment."
- Future tense only for events genuinely later than the action described.
  - Rec: "Add the filename to the backup list. The file will be archived the next time the backup process runs."
- Avoid hypothetical "would."
  - Rec: "If you send an unsubscribe message, the server removes you from the mailing list." | Not: "The server would then remove you from the mailing list."

## Prescriptive wording: must, can, might

Avoid "should" — it leaves the reader unsure whether something is required. Map intent to wording:

| Intent | Use |
|---|---|
| Required | "must," or an imperative instruction |
| Recommended | "we recommend" |
| Optional | "can" |
| Possible outcome | "might" or "can" |

- State expected outcomes plainly, without "should."
  - Rec: "The column of the data table that the filter operates on." | Not: "…that the filter should operate on."
- Be prescriptive: tell the reader what to do rather than listing every possibility.

## Timeless writing

- Describe the current state; don't anchor to past versions or future plans.
- Avoid: "currently," "now," "new," "soon," "latest," "as of this writing," "does not yet," "eventually," "in the future," "existing," "old."
  - Rec: "The emulator supports the following filters:" | Not: "The emulator now supports the following filters:"
- Never pre-announce or speculate about future features or versions.
- If "new" is unavoidable, anchor it to a date or version.

## Sentence structure

- State the circumstance, condition, or goal before the instruction.
  - Rec: "For more information, see [link]." | Not: "See [link] for more information."
  - Rec: "To delete the document, click **Delete**." | Not: "Click **Delete** if you want to delete the document."
  - Rec: "If your app is in one of the following regions, custom domains might add latency:" | Not: "Custom domains might add latency if your app is in one of the following regions:"
- Keep sentences under about 26 words; break up long ones.
- Put the distinguishing information first in a sentence or list item.
- Avoid double negatives and exceptions to exceptions.
  - Rec: "You can continue without a path." | Not: "A missing path won't prevent you from continuing."
- Vary sentence openers — don't start every sentence with "You can" or "To do."
- Prefer positive constructions — say what the reader can do, not what they can't.

## Writing for a global audience

- Prefer simple words: "use" not "utilize" or "leverage"; "start" not "commence"; "so" not "consequently"; "some" not "a number of."
- Keep optional helper words — they cost nothing and remove ambiguity:
  - Rec: "If the key is not found, then the default value is returned." (keep "then")
  - Rec: "assumes that you have the following knowledge" (keep "that")
  - Rec: "Start the profiler, and then run the app." | Not: "Start the profiler, then run the app."
  - Rec: "update the rules that you previously defined" | Not: "update the rules you previously defined"
- Repeat nouns rather than leaning on pronouns; repeat words when repetition removes ambiguity ("both IAM segmentation and network segmentation").
- Limit noun stacks to two nouns modifying another noun; break up longer stacks with prepositions or hyphens.
- Place modifiers next to what they modify: "Request only one token," not "Only request one token."
- Add a qualifying noun to technical keywords: "the `example.yaml` file," not bare "`example.yaml`."
- Use each word in one consistent sense; one term per concept, with identical capitalization throughout.
- Standard subject-verb-object order; keep subject and verb near the start.
- No culturally specific references, idioms, humor, holidays, sports, or hemisphere-dependent seasons; use globally diverse example names.

## Claims and objectivity

- No superlatives or absolutes: "best," "simplest," "fastest," "never," "always."
- Use "ensure" or "guarantee" only when the guarantee truly holds.
- Back performance claims with data and cite the source.
- No absolute security claims — frame security measures as part of an overall strategy that "helps prevent" problems.
- Don't disparage third-party products; qualify comparative claims with verifiable reasons.

## Jargon

- Write around jargon in plain language: "affected area" not "blast radius"; "import" not "ingest"; "review what worked" not "hold a post-mortem."
- If a term of art is needed once, define it inline: "a _cold standby_ (a backup system identical to the primary)."
- When code uses a jargon term, keep the literal term in code font and use the plain term in prose.

## Abbreviations

- Spell out on first use with the abbreviation in parentheses; use the abbreviation alone afterward.
- Capitalize the spelled-out form only if it's a proper noun: "data manipulation language (DML)," not "Data Manipulation Language (DML)."
- Skip spelling out abbreviations better known than their expansions: API, URL, HTML, PDF, RAM, REST, USB, AI, file formats, units.
- Never "i.e." or "e.g." — write "that is" or "for example." Never "etc." — write "and so on," or rewrite with "such as" or "like."
- No internet slang (tl;dr, ymmv, RTFM). Prefer "approximately" over "approx.," "10 times" over "10x."
- Don't use abbreviations as verbs.
  - Rec: "Use SSH to log in to your remote shell." | Not: "Then ssh into your remote shell."
- Choose a/an by spoken pronunciation: "a SQL query," "an SAP system."

## Capitalization

- Sentence case for titles, headings, list items, table headers and cells, and figure captions. No trailing periods on headings.
- Don't capitalize for emphasis or to coin meaning; no ALL CAPS or camelCase outside official names and code.
- Product names exactly as officially styled; capitalization of a product name never varies.
- After a colon: lowercase, unless what follows is a proper noun, a heading, or a quotation.
- A hyphenated word starting a sentence or heading capitalizes only its first element ("Load-balance the traffic").
- Don't name casing styles in prose — show the format: "with no spaces and each word capitalized — for example, `AssertionAccount`."

## Contractions

- Use common two-word contractions (you're, don't, there's) — they keep tone friendly.
- Prefer negation contractions (isn't, can't, don't) over "is not" / "cannot" — a scanning reader can miss the "not."
- No invented contractions (guides're), no three-word contractions (mightn't've), and don't contract a noun with "is" ("the browser's fast").

## Pronouns

- Every pronoun needs an unambiguous antecedent; when in doubt, repeat the noun.
  - Rec: "If you type text in the field, the text doesn't change." | Not: "…it doesn't change."
- Follow "this," "these," and "that" with a noun: "Set this value to true," not "Set this to true."
- Use singular "they"; never "he/she," "(s)he," or gendered defaults.
- "That" introduces restrictive clauses (no comma); "which" introduces nonrestrictive clauses (with comma).
- Use "who," not "that," for people.

## Plurals and possessives

- Never form a plural with 's: "APIs," "IDEs," "the 2020s."
- Don't pluralize code names: "`Intent` objects," not "`Intent`s." Don't pluralize trademarks or product names.
- No optional plurals: "your API key," never "your API key(s)."
- "One or more X" takes a plural verb; "more than one X" takes singular.
- Units: "1 degree, 15 degrees"; don't pluralize unit symbols ("64 GB," not "64 GBs").
- Singular possessive: 's (even after s); plural ending in s: apostrophe only.
- No possessives on product names or code items — restructure: "the return value of `wordCount`," not "`wordCount`'s return value."

## Articles and prepositions

- Keep articles (a, an, the) even in headings: "Create a VM instance," not "Create VM instance."
- Ending a sentence with a preposition is fine; don't contort to avoid it.
  - Rec: "…the language you're interacting with." | Not: "…the language with which you're interacting."
- Cut prepositions that add nothing; avoid stacking several in a row.

## Reference-doc verbs

- When describing what a method or function does in reference material, use third-person singular present, not imperative: "`tasks.insert`: Creates a new task," not "Create a new task."
