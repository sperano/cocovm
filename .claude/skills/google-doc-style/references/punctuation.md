# Punctuation

Detailed rules distilled from the Google Developer Documentation Style Guide. "Rec:" = recommended, "Not:" = not recommended.

## Commas

- Serial (Oxford) comma before the final "and"/"or" in a series of three or more: "zones, regions, and multi-regions."
- Comma after an introductory word or phrase: "Finally, only groups that contain parameters appear."
- Comma before a coordinating conjunction joining two independent clauses — unless both are very short.
  - Rec: "The libraries make feed creation easier, and they ensure that only valid feeds are produced." | Short: "Type your ID and click **OK**."
- No comma before a conjunction joining a clause to a mere predicate.
  - Not: "Direct-access flags are plain variables, and can be read directly."
- Comma before nonrestrictive "which"; restrictive "that" takes no comma.
- Comma after a conjunctive adverb: "The variable must have a value; otherwise, the server returns an error."
- Generally no comma before "because" unless needed to prevent misreading.

## Colons

- The text before a list-introducing colon must be a complete sentence: "The fields are defined as follows:" not "The fields are:"
- Lowercase after a colon unless a proper noun, heading, or quotation follows.
- Use a colon (not a dash) to separate an item from its description: "Example: This is an example."

## Dashes and hyphens

- Em dash (—) with no surrounding spaces marks a break in a sentence.
- Never use en dashes: use a hyphen for ranges (8-20 files) or the word "to."
- Never separate an item from its description with a spaced dash; use a colon.
- Hyphenation:
  - Most prefixes close up: metadata, preprocessing, nonempty. Hyphenate self-/cross- compounds, prefixes before capitals or numbers (non-Google, post-2000), and where clarity demands (re-sign vs. resign).
  - Compound modifiers before a noun: "well-designed app," "Android-specific techniques." Not after a verb: "the app is well designed."
  - Never hyphenate -ly adverbs: "publicly available."
  - Number + spelled-out unit modifier: "64-bit system," "five-minute wait." Number + abbreviated unit: no hyphen — "200 GB disk."
  - Suspended hyphens: "one- or two-hour intervals."
  - Closed compounds: webpage, hostname, tradeoff, workaround.
  - Avoid three-plus-word modifiers before a noun; rearrange the sentence.

## Semicolons

- Avoid where possible; a period or comma usually reads better.
- Acceptable: joining closely related independent clauses; before conjunctive adverbs ("…; therefore, …"); separating series items that contain internal commas.

## Periods and spacing

- End sentences with a period; headings and most list fragments take none.
- Periods and commas go inside quotation marks — except with code font, where punctuation stays outside: "If you enter `escape`, the program crashes."
- One space between sentences.
- Don't end a sentence with a URL if you can avoid it.
- No periods in acronyms (API, not A.P.I.).

## Parentheses

- Don't put important information in parentheses — readers skip it. Prefer commas, em dashes, or a separate sentence.
  - Rec: "Enter a name for the instance—for example, `my-instance-99`." | Not: "(for example, `my-instance-99`)"
- A complete sentence inside parentheses keeps its period inside; otherwise the period goes outside.
- Never "(s)" for optional plurals.

## Quotation marks

- Double straight quotes; single quotes only for quotes nested inside quotes or in code.
- Quotes are for direct citations, unlinked titles of short works or sections, and metaphorical terms ("an 'island' within the network").
- Literal strings and keyboard input take code font, not quotes.

## Ellipses and exclamation points

- Don't use ellipses in prose; state everything necessary and omit the rest.
- Drop the ellipsis when naming a UI element ("click **Save**," even if the button reads "Save…").
- In quotes, an ellipsis marks only mid-quote omissions — never at the start or end.
- No exclamation points, except inside code or literal strings.

## Slashes

- Slashes belong in code, file paths, and URLs — not prose.
- No "and/or": pick "and" or "or," or write "either X, Y, or both."
- No slash dates (3/4/2026), slash fractions (3/4 — use 0.75 or 75%), slash abbreviations (w/, c/o), or slash alternatives ("developed/hosted").
- Write rates with "per" when space allows: "requests per day," not "requests/day."
