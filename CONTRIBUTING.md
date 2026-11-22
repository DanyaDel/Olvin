# Contributing / Контрибьютору

Short entry point; the full guide lives in the wiki: [Руководство контрибьютора](https://danyadel.github.io/Olvin/ru/dev/contributing.html) / [Contributing guide](https://danyadel.github.io/Olvin/en/dev/contributing.html) (sources: [`docs/wiki/ru/src/dev/contributing.md`](docs/wiki/ru/src/dev/contributing.md), [`docs/wiki/en/src/dev/contributing.md`](docs/wiki/en/src/dev/contributing.md)).

## Before you push

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Render tests without a GPU run on Mesa llvmpipe:

```sh
export __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json WGPU_BACKEND=gl
```

## Rules in one screen

- Code, comments, logs, commit messages: English. Docs: Russian first with an English copy (`*.en.md`, wiki `ru/` + `en/`). Issues: Russian. See [STYLE.md, section 10](STYLE.md).
- Commit message: `Area: what was done (#issue)`. If an AI agent helped, keep its `Co-Authored-By:` and `Claude-Session:` trailers.
- Read [ARCHITECTURE.md](ARCHITECTURE.md) first, update it (and [ROADMAP.md](ROADMAP.md)) with significant changes.
- Wiki pages are in [`docs/wiki/`](docs/wiki/README.md): `docs/wiki/tools/build.sh` checks links and the Lua reference and builds both books. A new `ctx:` function in the Lua API must be added to the reference or CI fails.

Подробно (RU): как запускать тесты, добавить плагин движка или элемент редактора, стиль кода и коммитов, политика документации: [wiki](https://danyadel.github.io/Olvin/ru/dev/contributing.html). Закрывает [#62](https://github.com/DanyaDel/Olvin/issues/62).
