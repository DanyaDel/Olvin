# Localization

`olvin_locale` (feature `locale`, `olvin::locale`) keeps the game's text out of the code: one table per language, looked up by key.

```rust
app.add_plugin(olvin::locale::LocalePlugin::new());   // assets/locales/*.ron
let locale = app.resources.fetch::<olvin::locale::Locale>();
let title = locale.tr("menu.play");
```

## Tables

One RON map per language in `assets/locales/`, the file name is the language (`en.ron`, `ru.ron`, `pt-BR.ron`):

```ron
{
    "menu.play": "Play",
    "hello": "Hello, {name}!",
    "coins.one": "{n} coin",
    "coins.other": "{n} coins",
}
```

## Lookup

`tr(key)` looks in the current language, then in the **fallback** language (`en` by default), then returns the key itself, so a missing string is visible on screen and logged once. `tr_with("hello", &[("name", &"Dan")])` fills `{name}`; write `{{` and `}}` for literal braces.

## Plurals

`plural("coins", 5, &[])` picks `coins.one` / `coins.few` / `coins.many` / `coins.other` by the rules of the current language and sets `{n}`. Built in: English-like (one, other), Russian, Ukrainian and Belarusian (one, few, many), Polish, Czech and Slovak, French and Portuguese (0 and 1 are singular), and languages with one form (Japanese, Chinese, Korean and others). A missing form falls back to `.other`.

## Choosing the language

`LocalePlugin` takes `OLVIN_LANG`, then `LC_ALL`, `LC_MESSAGES`, `LANG` (`ru_RU.UTF-8` selects `ru`; a regional tag with no table uses the base language). To let the player choose: `.language("ru")` on the plugin, or `locale.set_language("ru")` at run time (returns `false` and keeps the current language if there is no such table). `locale.languages()` lists what is loaded.

## For translators

`locale.missing_in("ru")` lists the keys the fallback language has and `ru` lacks.

## See also

- [Game UI](game-ui.md)
- [States and saves](game-states-saves.md)
- [Environment variables](../reference/env-vars.md)
