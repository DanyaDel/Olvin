#!/usr/bin/env python3
"""Writes the retro-ps1 pack's textures, materials, look and demo scene (pure stdlib).

Textures are tiny (32x32 / 64x64) and procedural, in the spirit of a PS1 game:
a few flat colours with noise, no gradients. Run from anywhere:
    python3 library/tools/gen_retro_ps1.py
"""
import pathlib
import struct
import zlib

PACK = pathlib.Path(__file__).resolve().parents[1] / "retro-ps1/assets"
TEX = PACK / "retro-ps1/textures"
MAT = PACK / "retro-ps1/materials"
SCENES = PACK / "scenes/retro-ps1"
LOOKS = PACK / "retro-ps1/looks"


def png(path, size, pixel):
    """RGB PNG of size x size; `pixel(x, y)` gives (r, g, b)."""
    raw = b"".join(
        b"\x00" + bytes(c for x in range(size) for c in pixel(x, y)) for y in range(size)
    )

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    data = b"\x89PNG\r\n\x1a\n"
    data += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0))
    data += chunk(b"IDAT", zlib.compress(raw, 9))
    data += chunk(b"IEND", b"")
    path.write_bytes(data)


def noise(x, y, seed):
    """Deterministic 0..1 hash noise."""
    n = (x * 374761393 + y * 668265263 + seed * 2147483647) & 0xFFFFFFFF
    n = ((n ^ (n >> 13)) * 1274126177) & 0xFFFFFFFF
    return ((n ^ (n >> 16)) & 0xFFFF) / 65535.0


def shade(base, k):
    return tuple(max(0, min(255, int(c * k))) for c in base)


def snap(c):
    """Round to 5 bits per channel: the textures live on the PS1 lattice."""
    return tuple(int(round(v / 255 * 31) / 31 * 255) for v in c)


def brick(x, y):
    row = y // 8
    off = 8 if row % 2 else 0
    mortar = y % 8 == 0 or (x + off) % 16 == 0
    if mortar:
        return snap(shade((150, 140, 128), 0.85 + 0.3 * noise(x, y, 1)))
    return snap(shade((156, 70, 48), 0.8 + 0.4 * noise(x // 2, y // 2, row + 3)))


def concrete(x, y):
    return snap(shade((128, 128, 124), 0.8 + 0.35 * noise(x, y, 5)))


def wood(x, y):
    plank = x // 16
    grain = 0.85 + 0.15 * noise(x // 2, y // 8, plank + 7) + (0.25 if x % 16 == 0 else 0.0) * -1
    return snap(shade((140, 92, 48), max(0.4, grain) * (0.9 + 0.2 * noise(x, y, 9))))


def grate(x, y):
    bar = x % 8 < 2 or y % 8 < 2
    if bar:
        return snap(shade((118, 122, 128), 0.8 + 0.3 * noise(x, y, 11)))
    return snap((22, 22, 26))


def grass(x, y):
    blade = noise(x, y, 13)
    base = (52, 120, 40) if blade > 0.35 else (38, 92, 30)
    return snap(shade(base, 0.8 + 0.4 * noise(x // 2, y, 15)))


def water(x, y):
    wave = (x * 3 + y * 5 + int(6 * noise(x // 4, y // 4, 17))) % 16
    base = (40, 96, 130) if wave < 9 else (70, 140, 168)
    return snap(shade(base, 0.9 + 0.2 * noise(x, y, 19)))


def tile(x, y):
    grout = x % 16 == 0 or y % 16 == 0
    if grout:
        return snap((70, 78, 78))
    checker = ((x // 16) + (y // 16)) % 2
    base = (186, 196, 190) if checker else (120, 150, 150)
    return snap(shade(base, 0.92 + 0.12 * noise(x, y, 21)))


def skin(x, y):
    return snap(shade((214, 150, 118), 0.9 + 0.2 * noise(x, y, 23)))


TEXTURES = {
    "brick": (64, brick, [1.0, 1.0]),
    "concrete": (32, concrete, [1.0, 1.0]),
    "wood": (64, wood, [1.0, 1.0]),
    "metal_grate": (32, grate, [1.0, 1.0]),
    "grass": (64, grass, [1.0, 1.0]),
    "water": (64, water, [1.0, 1.0]),
    "tile": (32, tile, [1.0, 1.0]),
    "skin": (32, skin, [1.0, 1.0]),
}

for d in (TEX, MAT, SCENES, LOOKS):
    d.mkdir(parents=True, exist_ok=True)
for name, (size, fn, tiling) in TEXTURES.items():
    png(TEX / f"{name}.png", size, fn)
    (MAT / f"{name}.mat.ron").write_text(
        "(\n"
        "    base_color: (1.0, 1.0, 1.0, 1.0),\n"
        f'    base_texture: Some("retro-ps1/textures/{name}.png"),\n'
        "    metallic: 0.0,\n"
        "    roughness: 1.0,\n"
        f"    uv_scale: ({tiling[0]}, {tiling[1]}),\n"
        "    point_filter: true,\n"
        "    retro: Some(true),\n"
        ")\n"
    )

(LOOKS / "ps1.look.ron").write_text(
    """(
    name: "PS1",
    exposure: 1.0,
    filmic: false,
    fxaa: false,
    saturation: 0.85,
    contrast: 1.1,
    retro: Some((
        snap_resolution: (320, 240),
        snap_grid_m: 0.0,
        affine_strength: 1.0,
        affine_fade_near: 4.0,
        color_bits: 5,
        dither_strength: 1.0,
        vertex_lighting: true,
        shadows: false,
        draw_distance: 30.0,
        dissolve: false,
        cull_small_px: 1.0,
        force_nearest: true,
        internal_resolution: (320, 240),
        pixel_scale: 0,
        letterbox: false,
    )),
    fog: Some((color: (0.04, 0.04, 0.06), start: 6.0, end: 28.0)),
)
"""
)

# ---- the demo scene: two rooms joined by a corridor, 8 m modules.
ents = []
nid = [10]
BOX = "collider: Some((shape: Box(size: (1.0, 1.0, 1.0)), center: (0.0, 0.0, 0.0), is_trigger: false, friction: 0.8, restitution: 0.0))"


def f(v):
    return "(" + ", ".join(f"{c:.2f}" for c in v) + ")"


def add(name, pos, scale, material, shape="Cube", rot=(0, 0, 0), collide=True, extra=""):
    nid[0] += 1
    s = (
        f"        (id: {nid[0]}, name: \"{name}\", position: {f(pos)}, rotation: {f(rot)}, scale: {f(scale)},\n"
        f"            mesh: Some((shape: {shape}, color: (1.0, 1.0, 1.0, 1.0))),\n"
        f"            material: Some(\"retro-ps1/materials/{material}.mat.ron\")"
    )
    if collide:
        s += f",\n            {BOX}"
    s += extra + "),"
    ents.append(s)


def slab(name, pos, scale, material, cell=2.0):
    """A big box built from ~2 m cubes (PS1 content was tessellated: affine
    texturing on a few huge polygons smears), plus one invisible collider."""
    def n(v):
        return 1 if v <= 1.2 else max(1, round(v / cell))
    nx, ny, nz = n(scale[0]), n(scale[1]), n(scale[2])
    sx, sy, sz = scale[0] / nx, scale[1] / ny, scale[2] / nz
    for i in range(nx):
        for j in range(ny):
            for k in range(nz):
                p = (
                    pos[0] - scale[0] / 2 + sx * (i + 0.5),
                    pos[1] - scale[1] / 2 + sy * (j + 0.5),
                    pos[2] - scale[2] / 2 + sz * (k + 0.5),
                )
                add(f"{name} {i}.{j}.{k}", p, (sx, sy, sz), material, collide=False)
    nid[0] += 1
    ents.append(
        f"        (id: {nid[0]}, name: \"{name} (collider)\", position: {f(pos)}, rotation: (0.0, 0.0, 0.0), scale: {f(scale)},\n"
        f"            {BOX}),"
    )


def light(name, pos, color, intensity, rng, script=False):
    nid[0] += 1
    s = (
        f"        (id: {nid[0]}, name: \"{name}\", position: {f(pos)}, rotation: (0.0, 0.0, 0.0), scale: (1.0, 1.0, 1.0),\n"
        f"            light: Some((kind: Point, color: {f(color)}, intensity: {intensity}, range: {rng}, spot_angle: 30.0, shadows: false))"
    )
    if script:
        s += ',\n            scripts: [(name: "retro-ps1/lamp_flicker", params: {"flicker_rate": "9.0", "outage": "0.3"})]'
    ents.append(s + "),")


# Room 1 (x, z in -4..4), corridor (x -2..2, z -12..-4), room 2 (z -20..-12).
slab("Floor 1", (0, -0.1, 0), (8, 0.2, 8), "concrete")
slab("Floor Corridor", (0, -0.1, -8), (4, 0.2, 8), "metal_grate")
slab("Floor 2", (0, -0.1, -16), (8, 0.2, 8), "tile")
for name, z in (("Ceiling 1", 0), ("Ceiling Corridor", -8), ("Ceiling 2", -16)):
    w = 4 if "Corridor" in name else 8
    slab(name, (0, 4.1, z), (w, 0.2, 8), "concrete")
slab("Wall 1 West", (-4.2, 2, 0), (0.4, 4, 8), "brick")
slab("Wall 1 East", (4.2, 2, 0), (0.4, 4, 8), "brick")
slab("Wall 1 South", (0, 2, 4.2), (8.8, 4, 0.4), "brick")
slab("Wall 1 North L", (-3.2, 2, -4.2), (2.4, 4, 0.4), "brick")
slab("Wall 1 North R", (3.2, 2, -4.2), (2.4, 4, 0.4), "brick")
slab("Lintel 1", (0, 3.5, -4.2), (4, 1, 0.4), "brick")
slab("Corridor West", (-2.2, 2, -8), (0.4, 4, 8), "concrete")
slab("Corridor East", (2.2, 2, -8), (0.4, 4, 8), "concrete")
slab("Wall 2 South L", (-3.2, 2, -11.8), (2.4, 4, 0.4), "brick")
slab("Wall 2 South R", (3.2, 2, -11.8), (2.4, 4, 0.4), "brick")
slab("Lintel 2", (0, 3.5, -11.8), (4, 1, 0.4), "brick")
slab("Wall 2 West", (-4.2, 2, -16), (0.4, 4, 8), "brick")
slab("Wall 2 East", (4.2, 2, -16), (0.4, 4, 8), "brick")
slab("Wall 2 North", (0, 2, -20.2), (8.8, 4, 0.4), "brick")
# Props.
add("Crate 1", (-3.0, 0.5, 2.8), (1, 1, 1), "wood")
add("Crate 2", (-3.1, 1.5, 2.9), (0.9, 1, 0.9), "wood", rot=(0, 25, 0))
add("Crate 3", (-1.8, 0.4, 3.2), (0.8, 0.8, 0.8), "wood", rot=(0, -12, 0))
add("Planter", (3.0, 0.2, 2.8), (2, 0.4, 2), "grass")
add("Pillar", (-2.5, 2, -14.5), (0.8, 4, 0.8), "concrete")
add("Pool", (2.0, 0.02, -17.5), (3.5, 0.06, 3.0), "water", collide=False)
add("Pedestal", (-2.0, 0.5, -18.2), (1, 1, 1), "tile")
add("Statue Body", (-2.0, 1.6, -18.2), (0.6, 1.2, 0.6), "skin", shape="Capsule", collide=False)
add("Statue Head", (-2.0, 2.4, -18.2), (0.5, 0.5, 0.5), "skin", shape="Sphere", collide=False)
# Lights: no sun (intensity 0), warm room, flickering corridor lamp, green room.
nid[0] += 1
ents.insert(0, f'        (id: {nid[0]}, name: "No Sun", position: (0.0, 10.0, 0.0), rotation: (-50.0, 30.0, 0.0), scale: (1.0, 1.0, 1.0),\n            light: Some((kind: Directional, color: (1.0, 1.0, 1.0), intensity: 0.0, range: 10.0, spot_angle: 30.0, shadows: false))),')
light("Lamp Room 1", (0, 3.4, 0), (1.0, 0.78, 0.5), 5.5, 12.0)
light("Lamp Corridor (flickers)", (0, 3.2, -8), (1.0, 0.85, 0.6), 5.0, 9.0, script=True)
light("Lamp Room 2", (0, 3.4, -16), (0.45, 0.9, 0.6), 5.0, 12.0)
light("Red Glow", (-3.0, 1.2, 3.0), (1.0, 0.2, 0.1), 2.5, 5.0)
ents.append(
    '        (id: 100, name: "Player", position: (0.0, 1.0, 2.0), rotation: (0.0, 0.0, 0.0), scale: (1.0, 1.0, 1.0),\n'
    '            prefab: Some((asset: "prefabs/fps-controller/Player.prefab.ron"))),'
)
body = "\n".join(ents)
(SCENES / "demo.scene.ron").write_text(
    "// Demo for the retro-ps1 pack: two rooms and a corridor in PS1 style. Press Play (WASD, mouse).\n"
    "// Settings > Rendering > Retro / PS1 holds every knob; the corridor lamp flickers from Lua.\n"
    "(\n"
    "    version: 5,\n"
    "    entities: [\n"
    f"{body}\n"
    "    ],\n"
    "    environment: (\n"
    "        sky: (0.2, 0.2, 0.27),\n"
    "        ground: (0.1, 0.09, 0.08),\n"
    "        background: (0.04, 0.04, 0.06),\n"
    "        filmic: false,\n"
    "        fog: Some((color: (0.04, 0.04, 0.06), start: 6.0, end: 28.0)),\n"
    "        post: (fxaa: false, saturation: 0.85, contrast: 1.1),\n"
    "        retro: Some((draw_distance: 30.0)),\n"
    "    ),\n"
    ")\n"
)
print("wrote", len(TEXTURES), "textures and materials,", len(ents), "scene entities")
