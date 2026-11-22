# Чёрное окно и GL

Окно открылось, но чёрное, кадр не показывается или рендер не стартует.

## NVIDIA + Wayland + GL

**Симптом:** при `WGPU_BACKEND=gl` на проприетарном драйвере NVIDIA под Wayland окно остаётся пустым, в логе ошибки презентации.

**Причина:** EGL драйвера не отдаёт native-renderable конфигурацию, и wgpu отказывается презентовать кадр. Ограничение wgpu, не движка.

**Что делает движок:** `olvin_render::check_gl` на старте определяет случай (vendor 0x10de, не Mesa и не nouveau, GL, Wayland). Если `WGPU_BACKEND` не задан, рендер пересоздаёт instance только с Vulkan; если задан или Vulkan нет, пишет одно предупреждение с советом.

**Решение:** не задавайте `WGPU_BACKEND` (Vulkan по умолчанию). Проверить GL-путь можно на Mesa:

```sh
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json WGPU_BACKEND=gl cargo run -p sandbox --release
```

## Нет Vulkan

```sh
sudo pacman -S vulkan-icd-loader vulkan-tools   # + vulkan-radeon / vulkan-intel / nvidia-utils
vulkaninfo --summary                            # должен видеть вашу GPU
```

Без Vulkan используйте Mesa GL (`mesa`, строка выше) или llvmpipe: медленно, зато работает (около 90 fps на 10 тысячах кубов).

## Сцена пустая

- Нет камеры в сцене: добавьте GameObject → Camera. Шаблон хаба без сцены рисует запасной куб.
- Сцена не загрузилась: смотрите лог (`scene loaded: N objects` или `no editor scene (...)`). Путь по умолчанию `assets/scenes/main.scene.ron`.
- Всё чёрное при включённой сцене: проверьте экспозицию и ambient в [Environment](../reference/render-settings.md), дальность света.

## Окно в виртуальной машине

Без 3D-ускорения wgpu падает на llvmpipe: `__EGL_VENDOR_LIBRARY_FILENAMES=… WGPU_BACKEND=gl`.

## См. также

- [Установка: видеокарта](../start/installation.md#видеокарта)
- [Рендер](../engine/rendering.md)
- [FAQ](faq.md)
- [Переменные окружения](../reference/env-vars.md)
