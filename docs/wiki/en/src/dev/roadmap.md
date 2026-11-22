# Roadmap

The full plan: [ROADMAP.en.md](https://github.com/DanyaDel/Olvin/blob/main/ROADMAP.en.md) ([Russian](https://github.com/DanyaDel/Olvin/blob/main/ROADMAP.md)). Tasks live in [issues](https://github.com/DanyaDel/Olvin/issues); ROADMAP ticks are set together with code.

## Principle

The engine grows from a game, not from a feature list. The vertical slice (`apps/vertical_slice`) is done; its [GAPS.md](https://github.com/DanyaDel/Olvin/blob/main/apps/vertical_slice/GAPS.md) showed what was missing, and all the gaps are closed.

## Phases

| Phase | Topic | State |
|---|---|---|
| 0 | CI, reliability, docs that do not lie | mostly done |
| 1 | core: assets, prefabs, scene versions, states, time, events, profiler, saves | done; localization and a parallel scheduler are planned |
| 2 | rendering: shadows, post-processing, transparency, particles, 2D, UI | partial (GPU skinning, IBL, clusters, LOD, terrain ahead) |
| 3 | gameplay: physics, audio, Lua, input | partial (joints, navmesh, networking, timers ahead) |
| 4 | build and platforms | `olvin-hub build` exists; Windows/macOS CI, web, Android ahead |
| 5 | editor | mostly done (configurable keys, multi-scene, a material editor with preview ahead) |
| 6 | ecosystem and documentation | this wiki, examples, CHANGELOG, publishing crates |

## In progress

> **Status:** planned or in development (pages will appear with the code): a presets library and a presets panel, particles in the scene format and editor ([#11](https://github.com/DanyaDel/Olvin/issues/11)), GPU skinning ([#8](https://github.com/DanyaDel/Olvin/issues/8)), post-processing and shaders ([#15](https://github.com/DanyaDel/Olvin/issues/15), [#22](https://github.com/DanyaDel/Olvin/issues/22)).

## Open topics by area

- Rendering: [#13](https://github.com/DanyaDel/Olvin/issues/13) shadows, [#15](https://github.com/DanyaDel/Olvin/issues/15) post-processing, [#16](https://github.com/DanyaDel/Olvin/issues/16) LOD, [#18](https://github.com/DanyaDel/Olvin/issues/18) clusters, [#19](https://github.com/DanyaDel/Olvin/issues/19) IBL.
- Gameplay: [#2](https://github.com/DanyaDel/Olvin/issues/2) physics, [#34](https://github.com/DanyaDel/Olvin/issues/34) navmesh, [#33](https://github.com/DanyaDel/Olvin/issues/33) networking.
- Platforms: [#50](https://github.com/DanyaDel/Olvin/issues/50), [#51](https://github.com/DanyaDel/Olvin/issues/51), [#52](https://github.com/DanyaDel/Olvin/issues/52).
- Tools: [#70](https://github.com/DanyaDel/Olvin/issues/70), [#45](https://github.com/DanyaDel/Olvin/issues/45), [#44](https://github.com/DanyaDel/Olvin/issues/44), [#41](https://github.com/DanyaDel/Olvin/issues/41).
- Release: [#65](https://github.com/DanyaDel/Olvin/issues/65) examples, [#66](https://github.com/DanyaDel/Olvin/issues/66) CHANGELOG, [#67](https://github.com/DanyaDel/Olvin/issues/67) crates.io.

## See also

- [Architecture](architecture.md)
- [Contributing guide](contributing.md)
- [What is Olvin](../start/what-is-olvin.md)
- [Documentation languages](language-policy.md)
