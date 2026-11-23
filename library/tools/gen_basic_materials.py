#!/usr/bin/env python3
"""Writes library/basic-materials/assets/basic-materials/materials/*.mat.ron.

Colour / PBR values only (linear RGB), no textures. Run from anywhere:
    python3 library/tools/gen_basic_materials.py
"""
import pathlib

OUT = pathlib.Path(__file__).resolve().parents[1] / "basic-materials/assets/basic-materials/materials"

# name: (base_color rgba, metallic, roughness, extras)
MATERIALS = {
    "stone": ((0.32, 0.31, 0.3, 1.0), 0.0, 0.92, {}),
    "brick": ((0.42, 0.14, 0.09, 1.0), 0.0, 0.85, {}),
    "wood": ((0.33, 0.17, 0.07, 1.0), 0.0, 0.7, {}),
    "steel": ((0.56, 0.57, 0.58, 1.0), 1.0, 0.3, {}),
    "gold": ((1.0, 0.71, 0.29, 1.0), 1.0, 0.22, {}),
    "plastic": ((0.8, 0.05, 0.04, 1.0), 0.0, 0.3, {}),
    "rubber": ((0.03, 0.03, 0.035, 1.0), 0.0, 0.95, {}),
    "glass": ((0.7, 0.85, 0.9, 0.22), 0.0, 0.03, {"alpha_blend": "true"}),
    "emissive": ((0.02, 0.02, 0.02, 1.0), 0.0, 0.6, {"emissive": "(0.2, 2.2, 3.0)"}),
    "grass": ((0.1, 0.3, 0.05, 1.0), 0.0, 0.95, {}),
}

OUT.mkdir(parents=True, exist_ok=True)
for name, (color, metallic, roughness, extra) in MATERIALS.items():
    lines = [
        f"    base_color: ({', '.join(str(c) for c in color)}),",
        f"    metallic: {metallic},",
        f"    roughness: {roughness},",
    ]
    lines += [f"    {k}: {v}," for k, v in extra.items()]
    (OUT / f"{name}.mat.ron").write_text("(\n" + "\n".join(lines) + "\n)\n")
print(f"wrote {len(MATERIALS)} materials to {OUT}")
