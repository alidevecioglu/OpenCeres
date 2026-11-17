# Translating OpenCeres

OpenCeres is used by asteroid-search teams around the world, and every
language helps. You do not need to know Rust: a translation is one plain
text file.

## How the files work

The app's texts live in `crates/desktop/locales/`, one UTF-8 file per
language, named by its language code: `en.txt` (English), `tr.txt`,
`es.txt`, `pt-BR.txt`, …

```
# A comment: the whole line is ignored.
onboarding.next = Next
onboarding.step = Step {n} of {total}
```

- Each line is `key = text`. Translate the text on the right; never change
  the key on the left.
- `{n}`, `{total}`, `{key}`, … are filled in by the app. Keep each one
  exactly as written (you may move it within the sentence).
- `\n` starts a new line.
- English (`en.txt`) is the reference. A key your file leaves out is shown
  in English, so a partial translation is fine and useful.

## Adding a language

1. Copy `crates/desktop/locales/en.txt` to `crates/desktop/locales/<code>.txt`,
   using the language's [BCP 47 code](https://en.wikipedia.org/wiki/IETF_language_tag)
   (`tr`, `de`, `pt-BR`).
2. Translate the texts. Delete lines you have not translated yet rather than
   leaving English in them, so the completeness figure stays honest.
3. Add the language to `LANGUAGES` in `crates/desktop/src/i18n.rs`, with its
   name in itself (the *endonym*, e.g. `Türkçe`) and in English:

   ```rust
   Language {
       code: "tr",
       endonym: "Türkçe",
       english: "Turkish",
       text: include_str!("../locales/tr.txt"),
   },
   ```

4. Run `./dev check`. The tests make sure that every file parses, that it
   has no key English lacks, that every text keeps the placeholders of the
   English one, and that the fonts can draw the language's name.
5. Open a pull request. If you cannot do steps 3 and 4, open an issue with
   your file attached and we will do them.

## Style

- **Short labels.** Buttons and menu items have little room; prefer the
  shortest natural wording. Check the result in the app.
- **Keep placeholders and keys.** `{name}` stays as it is. Keyboard keys
  (`Space`, `?`, `S`, `⌘`) are the keys the user presses, so keep them as on
  the keyboard.
- **Format and service names stay untranslated:** MPC, MPCORB, ADES, IASC,
  SkyBoT, Gaia, VizieR, FITS, observatory codes such as F51, and the
  report-header keywords (`COD`, `OBS`, `MEA`, `TEL`, `ACK`). They are names
  people search for and type into other systems.
- **Neutral and friendly.** OpenCeres is used by clubs, schools,
  observatories and individuals; do not assume who the reader is.
- **Use your language's typography:** its quotation marks, decimal comma or
  point, and a non-breaking space before `%` or `:` where your language puts
  one.

## How complete a language is

The language list (Welcome window and Settings → General) shows
"NN % translated" for a language that does not yet have every English key.
To see the figure for every language at once:

```sh
./dev cargo test -p openceres-desktop completeness -- --nocapture
```

## Which scripts work today

The app embeds its own fonts (Inter and Hack, cut down to keep the app
small), so a language can only be added if those fonts can draw it:

| Script | Status |
|---|---|
| Latin, with Latin-1 and Extended-A/B (Turkish, most European languages, Indonesian, Swahili, …) | Ready |
| Greek (modern, monotonic) | Ready |
| Cyrillic, basic block U+0400–U+045F (Russian, Bulgarian, Serbian, Belarusian, Macedonian) | Ready |
| Cyrillic beyond that block (Ukrainian Ґ ґ, Kazakh, Mongolian, …), Vietnamese (Latin Extended Additional) | Needs the font subset extended (`crates/desktop/assets/fonts`, see `theme.rs`) |
| Chinese, Japanese, Korean | Not yet: the fonts would be far too large to embed; these will use the system's fonts later |
| Right-to-left scripts (Arabic, Hebrew, Persian, …) | Not yet supported: the layout is left-to-right only |

If your language needs one of the last three rows, please open an issue: it
tells us where to go next.

## What is translated so far

The translation system is new. Today the Welcome window and the language
setting go through it; the rest of the app is being moved over. New keys
will appear in `en.txt` as that happens, and the completeness figure of
every language will drop until they are translated.
