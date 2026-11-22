# Documentation languages

The policy from STYLE.md, section 10.

| What | Language |
|---|---|
| Editor and hub interface (windows, menus, dialogs, toasts, CLI messages) | English; there is no localization yet |
| Documentation (`README`, `ARCHITECTURE`, `STYLE`, `ROADMAP`, `docs/`) | Russian first, an English copy beside it (`*.en.md`) updated together with it |
| Code: identifiers, comments, doc comments, logs and errors | English |
| Commit messages | English, `Area: what was done (#number)` |
| Issues and discussions | Russian |

## The wiki

Two books: `docs/wiki/ru/` (primary) and `docs/wiki/en/` (a copy). File names are identical, and `check_links.py` fails if a page exists in only one language. The "English / Русский" switch is in the top bar of every page. UI element names are not translated (the interface is English): "Window → Add Panel".

When interface localization appears, string keys will be in English and Russian will be the first translated locale.

## See also

- [Contributing guide](contributing.md)
- [Architecture](architecture.md)
- [STYLE.md, section 10](https://github.com/DanyaDel/Olvin/blob/main/STYLE.en.md)
- [Hub](../start/hub.md)
