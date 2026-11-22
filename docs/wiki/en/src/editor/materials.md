# Materials

A material is a `*.mat.ron` file (`MaterialDesc`): PBR in the Unity convention (metallic-roughness). The editor edits it in the Inspector, the **Material** section, and writes it to disk at once: the viewport reads files.

## Creating and assigning

- Inspector → Material → New: a file in `materials/`.
- Assets → an image → Create material: a material with that texture.
- Drag a material from Assets onto an object in the viewport or hierarchy.
- A glTF import puts materials in `materials/<model>/`.

A material is assigned to a mesh, a `poly` object and the faces of a CSG brush (`EntityDesc.material`: a path from `assets/`).

## Fields

| Field | Meaning |
|---|---|
| `base_color`, texture | colour and albedo |
| `normal` + strength | normal map |
| `metallic`, `roughness`, `orm` | parameters or an occlusion/roughness/metallic map |
| `emissive` (+ texture) | glow |
| `uv_scale`, `uv_offset`, `triplanar` | mapping |
| `point_filter` | pixel filtering |
| `blend`, `alpha_cutoff` | transparency or a mask |
| `unlit` | no lighting |
| `shader` | a custom surface shader `*.wgsl` |

The inspector has presets. Material edits are in the undo history: each step carries the contents of the edited files.

## Shaders

A surface shader implements the contract `fn surface(in: SurfaceInput) -> SurfaceOutput` (`SURFACE_SHADER_CONTRACT`); error line numbers match the file. Editing a shader in the IDE is picked up on the fly.

> **Status:** a material editor with a preview is not done, there is only the inspector section, see [#41](https://github.com/DanyaDel/Olvin/issues/41). There is no shader graph, see [#22](https://github.com/DanyaDel/Olvin/issues/22).

## See also

- [Rendering](../engine/rendering.md)
- [Assets](assets.md)
- [IDE](ide.md)
- [Render settings (reference)](../reference/render-settings.md)
