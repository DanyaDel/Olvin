# Olvin

Модульный игровой движок на Rust. Каждая подсистема (ядро, ECS, окно, рендер) — отдельный крейт-плагин.
Рендер на wgpu: Vulkan / Metal / DX12 / GLES 3 / WebGL2 / WebGPU.

```sh
cargo run -p sandbox --release                 # 40k кубов, WASD + ПКМ, Shift, Esc
OLVIN_CUBES=250000 cargo run -p sandbox --release
WGPU_BACKEND=gl cargo run -p sandbox --release # слабый путь (тир Compat)
```

Архитектура: [ARCHITECTURE.md](ARCHITECTURE.md).
