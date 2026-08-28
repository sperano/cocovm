# Writing about code and interfaces

How prose refers to code, commands, and UI. The code itself is out of scope — only the sentences around it follow these rules.

## Text-formatting summary

| Text | Format |
|---|---|
| UI element names; run-in list lead-ins | **Bold** — bold is used only for these |
| Terms being defined; words as words | *Italic* |
| Titles of full-length works (unlinked) | *Italic* |
| Titles of short works or sections (unlinked) | "Quotation marks" |
| Inline code, commands, filenames, methods, HTTP codes, console output, user input | `Code font` |
| Placeholders | `ALL_CAPS` code font, italic where markdown allows: *`PROJECT_ID`* |
| Code blocks | Fenced code block |
| Headings, titles, table headers, captions | Sentence case |
| Emphasis (rare) | *Italic*, never bold or underline |
| Link text | Underlining is reserved for links; punctuation and quotes stay outside |

Markdown mechanics: `**` for bold, `_` for italic, backticks for code.

## Code font in text

Use code font for: attribute names and values, class and method names, data types, enum values, language keywords, command names (`gcloud`, `kubectl`), filenames and paths, environment variables, constants, package names, HTTP verbs and status codes (`GET`, `400 Bad Request` — say "status code"), content types, IP addresses, ports, DNS record types, database columns, query parameters, command output, text the user types, and URLs or domains used as data.

Don't use code font for: product names ("Google Docs"), domain names in ordinary prose, URLs the reader navigates to (link them descriptively), contact email addresses, or "true/false" as described states ("if the condition evaluates to true" — but "returns `true`" when citing the literal).

Grammar around code items:

- Never pluralize: "an array of `INT64` values," not "`INT64`s."
- Never possessivize: "the value of the `ADDRESS` constant," not "`ADDRESS`'s value."
- Never verb: "send a `POST` request," not "`POST` the data."
- Add a qualifying noun: "the `example.yaml` file," not bare "`example.yaml`."
- Method names drop the class unless ambiguity requires it: "call its `get` method."
- HTML/XML elements drop the angle brackets: "the `script` element."

## Placeholders

- Format: `UPPERCASE_WITH_UNDERSCORES`, descriptive — `PROJECT_ID`, never `foo`, `x`, or `MY_PROJECT`.
- In markdown, italicize inline placeholders in code font: *`BUCKET_NAME`*.
- Explain every placeholder at first use. One placeholder: "Replace `PROJECT_ID` with the ID of your project." Several: introduce with "Replace the following:" and list "`PLACEHOLDER`: description" items, in order of appearance, descriptions starting lowercase.

## Command-line syntax

- Multi-line input: prefix each input line with `$`; single-line commands may drop the prompt — be consistent. Never show the current directory in the prompt.
- Syntax notation: `[OPTIONAL_ARG]`, `{CHOICE_A|CHOICE_B}`, `REPEATED_ARG ...` — strip this notation from copyable examples.
- Break lines over 80 characters with a trailing space and `\` (Linux) or `^` (Windows); indent continuations four spaces.
- Say what a command does before showing it — name its purpose, not "run the following command."
- Show output only when the reader needs to verify or copy something; introduce it with "The output is similar to the following:". Mark omitted output with `...` on its own line.
- Put input and output in separate blocks.

## Code samples in documents

- Introduce a sample with a sentence ending in a colon if the sample directly follows, a period if material intervenes.
- Two-space indentation unless the language's own style dictates otherwise; wrap at 80 characters.
- Mark omissions with a language-appropriate comment, not "…".

## UI elements

- Bold the element's name, matching the UI's capitalization (sentence case if the label is ALL CAPS): "In the **New project** window, select the **New activity** checkbox."
- Drop the word "button": "Click **OK**," not "Click the OK button." Drop trailing ellipses: "Click **Browse**."
- Prefer the task over the widget: "Refresh the page" beats "Click **Refresh**" unless the specific control matters.
- Never use an element name as a verb: "In the **Name** field, enter…" not "**Name** the account."
- Menu paths with **>**: "Select **View > Tools > Developer Tools**."
- Verbs: **click** (mouse), **tap** (touchscreens; Android always), **press** (keys: "Press Control+C"), **enter** (put text in by any means), **select**/**clear** (checkboxes), **choose** (generic decisions), **turn on**/**turn off**, **go to**, **drag** (not "drag and drop"), **hold the pointer over** (not "hover").
- Prepositions: **in** dialogs, fields, lists, menus, and panes; **on** pages, tabs, and toolbars.
- Terms: "dialog" (not "dialog box" or "pop-up"), "the **Owner** box" or "field," "navigation menu" (not "left nav"), "expander arrow" (not "zippy"), a menu item is a "command," "toggle" is a noun only.
- Icon buttons: use the icon's accessible name ("click **Menu**"), never a description of its appearance ("the button with three lines").
- Keyboard: spell out modifiers, capitalize letters, name ambiguous characters: "Press Control+Shift+P," "Press Control+hyphen."
- No directional language: name the element instead of "above," "below," or "on the left."

## Filenames and file types

- New filenames: lowercase, hyphen-separated, ASCII only: `query-data.html` (underscores only for consistency with existing neighbors).
- In prose: code font plus the word "file": "edit the `build.sh` file." Preserve a real file's actual spelling.
- Name file types formally, not by extension: "a PNG file," not "a `.png` file."

## Example values

In **invented samples** — documentation, tutorials, fake walkthroughs, placeholder snippets — never use real data. Domains: example.com, altostrat.com. Emails: dana@example.com. People: gender-neutral names (Alex, Dana, Quinn) with they/them. Companies: Example Organization. Phones: 800-555-0100 to 0199. IPv4: 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24; IPv6: 2001:db8::/32. Resource names: descriptive (`frontend-development`), never `foo`, `bar`, or `baz`.

When the user or environment already supplied facts about their situation, project, or data, use those facts in reader-facing answers, reports, plans, and other artifacts that consume them. Do not anonymize or substitute sample values unless they ask. A single artifact can mix both: keep supplied facts, and still use example.com (and the ranges above) in any invented snippet.
