# Olvin

Модульный игровой движок на Rust. Каждая подсистема (ядро, ECS, окно, рендер) — отдельный крейт-плагин.
Рендер на wgpu: Vulkan / Metal / DX12 / GLES 3 / WebGL2 / WebGPU.

```sh
cargo run -p sandbox --release                 # 40k кубов, WASD + ПКМ, Shift, Esc
OLVIN_CUBES=250000 cargo run -p sandbox --release
WGPU_BACKEND=gl cargo run -p sandbox --release # слабый путь (тир Compat)
```

## Хаб

`olvin-hub` создаёт проекты игр и ставит в них модули: окно, рендер, физику, аудио и т.д.

```sh
cargo install --path hub/olvin_hub_cli   # olvin-hub
cargo install --path hub/olvin_hub_gui   # olvin-hub-gui
olvin-hub new my_game && cd my_game
olvin-hub list                 # каталог, [x] — установлено
olvin-hub add physics audio
olvin-hub run --release
```

Архитектура: [ARCHITECTURE.md](ARCHITECTURE.md).

## Лицензия

На выбор: [MIT](LICENSE-MIT) или [Apache-2.0](LICENSE-APACHE).
