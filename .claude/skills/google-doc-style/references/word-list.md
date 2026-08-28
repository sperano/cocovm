# Word list

High-value entries distilled from the Google Developer Documentation Style Guide word list (598 entries; verified against the raw page). Scan this when choosing between terms or when a word feels like jargon.

## Modal verbs

**can** = ability or option. **might** = possibility. **must** = requirement. **may** = reserved for policy or legal permission. Avoid **should**, **could**, **would** (ambiguous — state the requirement or recommendation explicitly), **will** (use present tense), and **shall**.

## Words to delete outright

please (in instructions) · simply · simple · easy · easily · quick(ly) · just (as filler) · currently · presently · at present · as of this writing · now (for product features) · new/newer · soon · eventually · in the future · does not yet · latest (unanchored) · note that · actually · let's

## Substitutions

| Don't use | Use |
|---|---|
| in order to | to |
| via | using, through |
| e.g. / i.e. | for example / that is |
| etc. / and so on | specific examples, or "such as" (non-exhaustive) |
| allows you to / enables you to | lets you |
| leverage / utilize | use |
| comprise | consist of, contain, include |
| impact (verb) | affect |
| execute | run |
| terminate / kill / abort | stop, exit, cancel, end |
| desire(d) / wish | want, need |
| above / below (doc position) | earlier, preceding / later, following |
| higher / lower (versions) | later / earlier |
| version 2.2+ | version 2.2 or later |
| while (contrast) | although |
| since (causation) | because |
| as (causation) | because |
| once (meaning after) | after |
| per (attribution) | according to |
| possible / impossible | you can / you can't |
| vs. | versus |
| performant | fast, efficient, or the specific property |
| functionality | capabilities, features |
| regex | regular expression |
| repo | repository |
| config (prose) | configuration |
| admin (prose) | administrator |
| k8s | Kubernetes |
| ssh (as verb) | connect using `ssh` |
| Google (as verb) | search with Google |
| hit (for click/press) | click, press, enter |
| hover | hold the pointer over, point to |
| check / uncheck | select / clear (checkboxes) |
| click on | click |
| login (verb) / log in | sign in ("sign in to," never "sign into") |
| grayed-out | unavailable |
| hang | stop responding |
| access (verb) | see, view, edit, use |
| surface (verb) | expose, make available |
| spin up | create, start |
| ingest (plain moving) | import, load, copy |
| roll out (figurative) | phased, gradual, in stages |
| copy and paste (mechanics) | say what to enter |
| tl;dr | To summarize, |
| aka | also known as |
| vice versa | state the reverse, or "conversely" |
| N/A (unexplained) | not applicable (spell out on first use) |

## Inclusive replacements

| Don't use | Use |
|---|---|
| whitelist / blacklist | allowlist / denylist (nouns only; rewrite verb forms) |
| master / slave | primary, main, controller, leader / replica, worker, secondary |
| sanity check / sane | quick check, validation / valid, sensible |
| crazy, insane | complicated, unexpected, strange (inanimate only) |
| cripples | slows down |
| dummy variable | placeholder |
| grandfathered | legacy, exempt |
| man-hours / manned / manpower | person-hours / staffed / workforce |
| man-in-the-middle | on-path attacker, person-in-the-middle (PITM) |
| guys | everyone, folks |
| native (for people) | (rewrite); for software: built-in |
| first-class citizen | built-in, fully supported |
| black-box / white-box testing | opaque-box / clear-box testing |
| blackhat / whitehat | unethical / ethical |
| female/male adapter | socket / plug |
| war room | incident-management team |
| tribal knowledge | institutional knowledge (or describe it) |
| ninja / guru / wizard (people) | expert, teacher, guide |
| hamburger menu / kebab menu | the icon's label: **Menu**, **More** |
| zippy / disclosure triangle | expander arrow |
| blast radius | affected area |
| single pane of glass | unified interface |

When code itself uses a non-inclusive term, keep the literal term in code font and use the preferred term in prose: "Start the replica by using the `START SLAVE` statement."

## Usage notes

- **data** — singular mass noun: "the data is."
- **deprecate** — means "recommend against using," not "removed."
- **dialog** — the UI element (never "dialog box" or "pop-up"); "dialogue" is human conversation.
- **directory vs. folder** — directory in CLI contexts, folder in GUI.
- **display** — transitive: "the window appears," never "the window displays."
- **email** — never e-mail; not a verb.
- **enter vs. type** — enter = get text in by any means; type = keystrokes.
- **enable vs. turn on** — pick one per document and stick with it.
- **API** — a whole API, never a single method.
- **each** — individual items; not a synonym for "all."
- **either** — exactly two choices.
- **between vs. among** — between for distinct items (any number); among for a mass or group.
- **if vs. whether** — whether for alternatives; keep "then" in if…then sentences.
- **like vs. such as** — both fine; lists they introduce are non-exhaustive, so never add "etc."
- **media type** — not MIME type.
- **method** — in OO prose, don't also use it to mean "approach."
- **OS** — fine unexpanded; so are AI, API, CPU, REST (never expand REST).
- **page** — a console tab is a "page."
- **select vs. choose** — select for UI elements; choose for decisions.
- **they** — preferred singular gender-neutral pronoun, plural verb.
- **typically** — fine for the normal case; don't open a sentence with it.
- **user** — your reader's users only; the reader is "you."
- **with (means)** — "use the tool to debug," not "debug with the tool"; "a phone that has 2 GB," not "with 2 GB."

## Spelling and compounds

- One word: backend, frontend, checkbox, codebase, dataset (per product), endpoint, filename, hostname, inline, lifecycle, microservices, namespace, timestamp, toolkit, touchscreen, walkthrough, website, webpage, whitespace, wildcard, whitepaper, healthcare, hardcoded, screenshot, runbook.
- Two words: data center, data source, data type, file system, name server, web server, plain text (plaintext only in cryptography), time zone (noun).
- Verb/noun splits: set up / setup, sign in / sign-in, log in / login, back up / backup, start up / startup, time out / timeout, fail over / failover, plug in / plugin, roll back / rollback.
- Prefixes usually close up: autoscaling, prebuilt, preemptible, multiregional, nonempty. Exceptions: pre-existing, pre-shared key, multi-region, multi-tenancy, self-*, cross-*.
- Always hyphenated: on-premises (never on-prem), read-only, third-party (adjective; "third party" as noun), drag-and-drop (adjective only).
- Plurals: appendixes, indexes, matrixes (not -ices, outside math).
- Other: curl (lowercase), OAuth 2.0, Unicode, UTF-8, US (no periods), "v1.2" (lowercase v), RFC 2318 (space before number), I/O, ID (not Id), Gbps vs. GBps (bits vs. bytes).

## Product names and trademarks

- Capitalize product names exactly as officially styled; if a name starts lowercase (macOS), don't start a sentence with it.
- No "the" before product names ("using Cloud Datastore"); "the" before tool and API names ("the Transcoder API," "the `gcloud` CLI").
- Never abbreviate official names (no "GCP"), never verb them, never pluralize or possessivize them.
- Trademarks modify a noun: "a Chromebook computer," not "a Chromebook"; "Chromebook computers," not "Chromebooks."
