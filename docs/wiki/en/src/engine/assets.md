# Assets and GUIDs

Assets work as in Unity: next to a file lies `<name>.meta` (RON) with a GUID and importer settings. Code: `olvin_assets` (no GPU).

## AssetRoot and .meta

- `AssetRoot` is the `assets/` folder. `resolve` does not leave it (`..` only inside, parts with `:` are dropped).
- `scan()` creates missing `.meta` files: `Meta { guid, importer }`, importer: `None`, `Texture(TextureImport { srgb, max_size, generate_mips, point_filter })`, `Model(ModelImport { scale, .. })`.
- A copy with a foreign `.meta` gets a new GUID. `move_asset` and `delete_asset` carry the `.meta` with the file.

## What it can load

| Type | What |
|---|---|
| Images | PNG etc., HDR, EXR (as float); thumbnails (`ThumbnailCache`, LRU) |
| glTF (`import_gltf`) | meshes, materials, skins to `Skeleton`, animations to `Clip`, cameras; embedded images to `<model>_textures/`; `KHR_texture_transform` baked into UVs |
| Materials | `*.mat.ron` |
| Prefabs | `*.prefab.ron` (`AssetKind::Prefab`) |

Not supported in glTF: morphs, orthographic cameras as a component, animation of nodes outside the skeleton on entities.

## AssetManager

The `AssetManager` resource (driven by `AssetsPlugin` in `Stage::First`, `with_hot_reload()`):

- `load(path)` returns a counted `Handle<T>` at once and loads on a thread pool; the same path gives the same handle.
- `get` → `Arc<T>`, `state` (`Loading` / `Ready` / `Failed`), `is_fully_loaded`, `wait_idle`.
- Dependencies: a material holds its textures, a model the textures of its materials.
- Unloading: `unload_unused` drops records without handles (also in `update` with `auto_unload`).
- Hot reload (`AssetWatcher`, notify): while new data loads the old stays available; on error the last working data stays.
- A decoder panic on a broken file is caught in the worker, the record becomes `Failed`.

In a game with rendering, scenes use the `RenderAssets` cache in the `olvin` facade: path → handle for materials, textures, shaders and sky, with mtime hot reload every 0.5 s.

> **Status:** packing assets into an archive and texture compression are not done ([#26](https://github.com/DanyaDel/Olvin/issues/26)). `Meshes` cannot free slots: spawning scenes repeatedly grows GPU memory.

## See also

- [Assets panel](../editor/assets.md)
- [Scene format](scene-format.md)
- [Project layout](project-layout.md)
- [olvin features](../reference/olvin-features.md)
