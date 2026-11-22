# Black window and GL

The window opened but it is black, no frame shows, or the renderer does not start.

## NVIDIA + Wayland + GL

**Symptom:** with `WGPU_BACKEND=gl` on the proprietary NVIDIA driver under Wayland the window stays empty, with presentation errors in the log.

**Cause:** the driver's EGL does not provide a native-renderable config and wgpu refuses to present a frame. A wgpu limitation, not the engine's.

**What the engine does:** at startup `olvin_render::check_gl` detects the case (vendor 0x10de, not Mesa or nouveau, GL, Wayland). If `WGPU_BACKEND` is unset the renderer re-creates its instance with Vulkan only; if it is set or Vulkan is missing it logs one warning with advice.

**Fix:** do not set `WGPU_BACKEND` (Vulkan is the default). The GL path can be checked on Mesa:

```sh
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json WGPU_BACKEND=gl cargo run -p sandbox --release
```

## No Vulkan

```sh
sudo pacman -S vulkan-icd-loader vulkan-tools   # + vulkan-radeon / vulkan-intel / nvidia-utils
vulkaninfo --summary                            # must see your GPU
```

Without Vulkan use Mesa GL (`mesa`, the line above) or llvmpipe: slow but it works (about 90 fps on 10 thousand cubes).

## The scene is empty

- No camera in the scene: add GameObject → Camera. The hub template without a scene draws a fallback cube.
- The scene failed to load: read the log (`scene loaded: N objects` or `no editor scene (...)`). The default path is `assets/scenes/main.scene.ron`.
- Everything is black with a scene loaded: check exposure and ambient in [Environment](../reference/render-settings.md) and light range.

## A window in a virtual machine

Without 3D acceleration wgpu fails; use llvmpipe: `__EGL_VENDOR_LIBRARY_FILENAMES=… WGPU_BACKEND=gl`.

## See also

- [Installation: GPU](../start/installation.md#gpu)
- [Rendering](../engine/rendering.md)
- [FAQ](faq.md)
- [Environment variables](../reference/env-vars.md)
