# Formatting and organization

Detailed rules distilled from the Google Developer Documentation Style Guide. "Rec:" = recommended, "Not:" = not recommended.

## Contents

- [Headings](#headings)
- [Paragraphs](#paragraphs)
- [Lists](#lists)
- [Procedures](#procedures)
- [Tables](#tables)
- [Notices](#notices)
- [Numbers](#numbers)
- [Units of measure](#units-of-measure)
- [Dates and times](#dates-and-times)
- [Italics and emphasis](#italics-and-emphasis)
- [Examples and example values](#examples-and-example-values)
- [Links and cross-references](#links-and-cross-references)

## Headings

- Sentence case, always. No ending period. No links inside headings.
- Task headings start with a bare infinitive: "Create an instance," not "Creating an instance."
- Conceptual headings are noun phrases without -ing verbs: "Migration to Google Cloud," not "Migrating to Google Cloud."
- Optional sections: "Optional: Customize your alias," not "Customize your alias (optional)."
- Keep articles: "Create a VM instance," not "Create VM instance."
- Don't skip heading levels, number headings to show sequence, or leave a heading with no text before its subheading.
- Avoid unfamiliar abbreviations and bare code items in headings; add a descriptive noun ("The `runtime` property").
- Refer to sections as "the following sections," not spatially.

## Paragraphs

- One idea per paragraph; put the key point in the first sentence — readers scan.
- Break up walls of text; five or more sentences in a paragraph usually means it holds more than one idea.
- Single-sentence paragraphs are fine.

## Lists

- Numbered lists when order matters; bulleted lists otherwise; term-definition pairs as a description list (in markdown, a bold run-in term).
- Introduce every list with a complete sentence ending in a colon.
  - Rec: "Use the **Submit** button for any of the following purposes:" | Not: "Use the **Submit** button to:"
- Capitalize the first word of each item. End items with a period unless they are single words, verbless fragments, or entirely code or link text.
- Keep items grammatically parallel — same structure, same capitalization, same punctuation.
- Run-in bold lead-ins: "**Term**: description in lowercase" (colon) or "**Term.** Description as a sentence." (period) — pick one pattern per list; never a dash.
- No one-item lists. Nested numbered lists use lowercase letters, then lowercase Roman numerals.
- Don't end a list with "etc." — give the full set, or introduce it as illustrative ("such as").

## Procedures

- Introduce with a complete sentence: "To customize the buttons, follow these steps:"
- One imperative action per step; start each step with the verb; write complete sentences; keep steps parallel.
- Order within a step: location, then goal, then action, then result.
  - Rec: "In Google Docs, click **File > New > Document**."
  - Rec: "To start a new document, click **File > New > Document**."
  - Rec: "Click **Run**. The query results appear."
- A single-step procedure is one bulleted sentence, not a numbered list of one.
- Combine trivially small UI actions with **>**: "Click **Next > Finish**."
- Mark optional steps with a leading "Optional:".
- Put prerequisites before the procedure, never inside a step or a note.
- Document the one best (simplest, most accessible) way to do a task; alternatives go under their own heading, not inline.
- Say what a command does before showing it, not just "run the following command."
- No "please," no directional language ("above"/"below").

## Tables

- Use a table only for genuinely two-dimensional data — each row needs two or more related values. One-dimensional data is a list.
- Introduce a table with a sentence ("…as listed in the following table:").
- Refer to tables as "the following table" or "the preceding table," never "the table below/above."
- Column headers: sentence case, concise, no end punctuation. Sort rows in a logical or alphabetical order.
- Split long or complicated tables. Never merge cells.

## Notices

- Use notes, cautions, and warnings sparingly — overuse makes them invisible. Never stack two consecutively.
- Note = helpful but skippable. Caution = proceed carefully. Warning = potential for data loss, security exposure, or irreversible damage.
- Never put prerequisites, required steps, or expected results in a note — those belong in the main flow.

## Numbers

- Spell out zero through nine in prose; numerals for 10 and up.
- Always numerals with units and technical quantities: "6 queries per second," "8 bits."
- Spell out a number that starts a sentence, or rewrite the sentence.
- If any comparable number in a sentence is 10+, use numerals for all of them: "15 options, but 6 of them."
- Spell out ordinals: "first," "twelfth" — never "1st."
- Prefer decimals to fractions (0.75); leading zero under one (0.3).
- Percentages: numeral plus % with no space (40%).
- Thousands separators for four-plus digits: 1,532,784.
- Ranges: hyphen with no spaces (2012-2016), or "from 8 to 20" — never mixed.
- Dimensions: lowercase x, no spaces (192x192).

## Units of measure

- Space between number and unit: "64 GB," not "64GB." No space before %, °, or currency symbols.
- Don't pluralize unit symbols ("64 GB," never "64 GBs").
- Ranges repeat the unit with "to": "-40 °C to 85 °C."
- Hyphenate multiplied units: "40 person-hours."
- Decimal (MB, GB) versus binary (MiB, GiB) units must match the technology — never interchange them.

## Dates and times

- Spell out dates: "January 19, 2017"; with weekday: "Tuesday, January 19, 2017." No comma in "January 2017"; comma after the year mid-sentence.
- Numeric dates only as ISO 8601: 2017-04-15. Never 3/4/2026.
- 12-hour clock with a space and capitals: "3:45 PM"; drop ":00" on the hour ("3 PM").
- Prefer "10 AM your local time"; otherwise name the zone with offset: "Pacific Standard Time (UTC-8)."
- Never use seasons to mark time of year — hemispheres differ. Use months or quarters.

## Italics and emphasis

- Italicize a term only when introducing and defining it: "A *Clos network* is a multistage switching network."
- Italicize words-as-words: "Use the word *and* instead."
- Bold is for UI elements and run-in list lead-ins, not general emphasis. Use sparingly for genuinely critical emphasis.

## Examples and example values

- Introduce short examples with "such as," "for example," or "like," set off by a comma, em dash, or parentheses — not a semicolon.
- Longer examples get their own sentence: "For example, …"
- Placeholder data must never be real:
  - Domains: example.com, example.org, altostrat.com. Emails: dana@example.com.
  - Names: gender-neutral (Alex, Dana, Quinn) with they/them; no stereotyped roles; Alice/Bob only where protocol convention requires.
  - Companies: "Example Organization." Phones: 800-555-0100 through 0199. IPs: RFC 5737 ranges (192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24); IPv6 2001:db8::/32.
  - Resource names: meaningful and descriptive — never foo, bar, or baz.
- In numeric example dates, pick days greater than 12 so day and month can't be confused.

## Links and cross-references

- Link selectively; every link is cognitive load and an exit. A brief in-place explanation often beats a link.
- Link text is short, unique, and descriptive — the page title or a descriptive phrase. Front-load the key words.
- Never "click here," "this document," or a raw URL as link text.
  - Rec: "For more information, see Make headings into link targets." | Not: "Want more? Click here."
- Standard phrasing: "For more information about X, see Y." Use "about," not "on"; use "see."
- Include the abbreviation inside the link text: "[Google Kubernetes Engine (GKE)]."
- When linking a flag or command, include its noun in the link: "the [`--hostname` flag]."
- Flag surprising behavior in the text: downloads, same-page jumps, cross-domain targets ("see OS-level virtualization (Wikipedia)").
- Punctuation goes outside the link text.
- Don't repeat the same link within a page.
- Avoid footnotes entirely — use a cross-reference, note, or parenthetical instead.
