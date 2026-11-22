# Olvin Wiki

A modular game engine in Rust by [NLPA Collab](https://nlpacollab.neocities.org/). This wiki describes the engine, the hub and the editor: one page per topic, facts instead of stories, links between pages.

![The Olvin editor](images/editor-dark.png)

## Where to start

| I want to | Page |
|---|---|
| Understand what this is | [What is Olvin](start/what-is-olvin.md) |
| Install it | [Installation](start/installation.md) |
| Make a game in an hour | [Tutorial: your first game](tutorial/index.md) |
| Learn the editor | [Editor: overview](editor/overview.md), [hotkeys](editor/hotkeys.md) |
| Write a script | [Scripting](engine/scripting.md), [Lua API reference](reference/lua-api.md) |
| Something broke | [FAQ and troubleshooting](troubleshooting/faq.md) |
| Contribute | [Contributing guide](dev/contributing.md) |

## Sections

- **Start**: [what is Olvin](start/what-is-olvin.md), [installation](start/installation.md), [the hub](start/hub.md), [first editor run](start/editor-first-run.md).
- **Tutorial**: [your first game in an hour](tutorial/index.md), based on the vertical slice `apps/vertical_slice`.
- **Editor**: tabs, hierarchy, inspector, scene view, assets, materials, prefabs, animation, IDE, CSG, modeling, UV, settings.
- **Engine**: project layout, scene format, assets, prefabs, rendering, physics, audio, input, animation, game UI, scripting, states and saves, time and events, profiler.
- **Reference**: [Lua API](reference/lua-api.md), [render settings](reference/render-settings.md), [`olvin` features](reference/olvin-features.md), [`olvin-hub` CLI](reference/hub-cli.md), [environment variables](reference/env-vars.md), [glossary](reference/glossary.md).
- **Troubleshooting**: [FAQ](troubleshooting/faq.md), black window, sound, hot reload, slow frames, build errors, Windows and macOS.
- **Development**: [contributing](dev/contributing.md), [architecture](dev/architecture.md), [roadmap](dev/roadmap.md), [documentation languages](dev/language-policy.md).

Search: the magnifier in the top bar or the `S` key. The language switch is in the top bar on the right. The Russian version is the primary one; this one is a copy.

> **Status:** the wiki describes `main` as of 2026-11-22. Pages about features still in flight (presets and the library, particles in scenes, GPU skinning) carry a "planned" note.

## See also

- [Repository README](https://github.com/DanyaDel/Olvin/blob/main/README.en.md)
- [ARCHITECTURE.en.md](https://github.com/DanyaDel/Olvin/blob/main/ARCHITECTURE.en.md): the source of truth for facts
- [Issue #63: book and tutorial](https://github.com/DanyaDel/Olvin/issues/63)
