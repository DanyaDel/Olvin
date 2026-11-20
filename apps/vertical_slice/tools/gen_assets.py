#!/usr/bin/env python3
"""Generates the vertical slice's binary assets, so the repo needs no downloads:

- assets/models/robot.gltf: a blocky skinned robot (7 joints, 2 primitives)
  with two clips, "Idle" and "Wave".
- assets/sfx/door.wav, assets/sfx/hum.wav (seamless loop), assets/sfx/thud.wav
- assets/music/theme.wav (seamless 16 s loop)

Everything is procedural, CC0. Run from anywhere:
    python -I apps/vertical_slice/tools/gen_assets.py
"""

import base64
import json
import math
import random
import struct
import wave
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "assets"


# ---------------------------------------------------------------- glTF robot

# name, parent, translation relative to the parent (bind pose has no rotations)
JOINTS = [
    ("Hips", None, (0.0, 0.95, 0.0)),
    ("Spine", 0, (0.0, 0.15, 0.0)),
    ("Head", 1, (0.0, 0.55, 0.0)),
    ("ArmL", 1, (0.30, 0.45, 0.0)),
    ("ArmR", 1, (-0.30, 0.45, 0.0)),
    ("LegL", 0, (0.12, 0.0, 0.0)),
    ("LegR", 0, (-0.12, 0.0, 0.0)),
]

# primitive -> list of (center, size, joint)
BODY = [
    ((0.0, 0.95, 0.0), (0.36, 0.20, 0.22), 0),
    ((0.0, 1.32, 0.0), (0.44, 0.50, 0.26), 1),
    ((0.0, 1.76, 0.0), (0.28, 0.28, 0.28), 2),
    ((0.32, 1.27, 0.0), (0.12, 0.56, 0.12), 3),
    ((-0.32, 1.27, 0.0), (0.12, 0.56, 0.12), 4),
    ((0.12, 0.47, 0.0), (0.15, 0.90, 0.15), 5),
    ((-0.12, 0.47, 0.0), (0.15, 0.90, 0.15), 6),
]
VISOR = [
    ((0.0, 1.79, 0.145), (0.22, 0.07, 0.02), 2),
    ((0.0, 1.38, 0.135), (0.14, 0.14, 0.02), 1),
]


def global_pos(j):
    x = y = z = 0.0
    while j is not None:
        _, parent, (tx, ty, tz) = JOINTS[j]
        x, y, z = x + tx, y + ty, z + tz
        j = parent
    return (x, y, z)


def box(center, size, joint):
    """24 vertices with flat normals, 36 indices (counter-clockwise outside)."""
    cx, cy, cz = center
    hx, hy, hz = (s * 0.5 for s in size)
    faces = [
        ((1, 0, 0), [(1, -1, -1), (1, 1, -1), (1, 1, 1), (1, -1, 1)]),
        ((-1, 0, 0), [(-1, -1, 1), (-1, 1, 1), (-1, 1, -1), (-1, -1, -1)]),
        ((0, 1, 0), [(-1, 1, -1), (-1, 1, 1), (1, 1, 1), (1, 1, -1)]),
        ((0, -1, 0), [(-1, -1, 1), (-1, -1, -1), (1, -1, -1), (1, -1, 1)]),
        ((0, 0, 1), [(-1, -1, 1), (1, -1, 1), (1, 1, 1), (-1, 1, 1)]),
        ((0, 0, -1), [(1, -1, -1), (-1, -1, -1), (-1, 1, -1), (1, 1, -1)]),
    ]
    pos, nrm, idx = [], [], []
    for normal, corners in faces:
        base = len(pos)
        for sx, sy, sz in corners:
            pos.append((cx + sx * hx, cy + sy * hy, cz + sz * hz))
            nrm.append(normal)
        idx += [base, base + 1, base + 2, base, base + 2, base + 3]
    return pos, nrm, idx, [joint] * len(pos)


class Buf:
    def __init__(self):
        self.data = bytearray()
        self.views = []
        self.accessors = []

    def _view(self, raw, target=None):
        while len(self.data) % 4:
            self.data.append(0)
        view = {"buffer": 0, "byteOffset": len(self.data), "byteLength": len(raw)}
        if target:
            view["target"] = target
        self.data += raw
        self.views.append(view)
        return len(self.views) - 1

    def accessor(self, raw, ctype, count, kind, target=None, minmax=None):
        acc = {"bufferView": self._view(raw, target), "componentType": ctype, "count": count, "type": kind}
        if minmax:
            acc["min"], acc["max"] = minmax
        self.accessors.append(acc)
        return len(self.accessors) - 1

    def floats(self, rows, kind, target=None, minmax=False):
        flat = [v for row in rows for v in (row if isinstance(row, (tuple, list)) else (row,))]
        mm = None
        if minmax:
            n = len(rows[0]) if isinstance(rows[0], (tuple, list)) else 1
            cols = [[r[i] if n > 1 else r for r in rows] for i in range(n)]
            mm = ([min(c) for c in cols], [max(c) for c in cols])
        return self.accessor(struct.pack(f"<{len(flat)}f", *flat), 5126, len(rows), kind, target, mm)


def primitive(buf, parts, material):
    pos, nrm, idx, jnt = [], [], [], []
    for part in parts:
        p, n, i, j = box(*part)
        base = len(pos)
        pos += p
        nrm += n
        idx += [base + k for k in i]
        jnt += j
    a_pos = buf.floats(pos, "VEC3", 34962, minmax=True)
    a_nrm = buf.floats(nrm, "VEC3", 34962)
    joints = b"".join(struct.pack("<4B", j, 0, 0, 0) for j in jnt)
    a_jnt = buf.accessor(joints, 5121, len(jnt), "VEC4", 34962)
    a_wgt = buf.floats([(1.0, 0.0, 0.0, 0.0)] * len(jnt), "VEC4", 34962)
    a_idx = buf.accessor(struct.pack(f"<{len(idx)}H", *idx), 5123, len(idx), "SCALAR", 34963)
    return {
        "attributes": {"POSITION": a_pos, "NORMAL": a_nrm, "JOINTS_0": a_jnt, "WEIGHTS_0": a_wgt},
        "indices": a_idx,
        "material": material,
    }


def quat_axis(axis, deg):
    h = math.radians(deg) * 0.5
    s = math.sin(h)
    return (axis[0] * s, axis[1] * s, axis[2] * s, math.cos(h))


def make_robot():
    buf = Buf()
    prims = [primitive(buf, BODY, 0), primitive(buf, VISOR, 1)]

    ibm = []
    for j in range(len(JOINTS)):
        x, y, z = global_pos(j)
        ibm.append((1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, -x, -y, -z, 1))
    a_ibm = buf.floats(ibm, "MAT4")

    nodes = []
    for name, parent, t in JOINTS:
        node = {"name": name, "translation": list(t)}
        kids = [i for i, (_, p, _) in enumerate(JOINTS) if p == JOINTS.index((name, parent, t))]
        if kids:
            node["children"] = kids
        nodes.append(node)
    body_node = len(nodes)
    nodes.append({"name": "RobotMesh", "mesh": 0, "skin": 0})

    animations = []

    def clip(name, tracks):
        channels, samplers = [], []
        for joint, path, times, values in tracks:
            a_in = buf.floats(times, "SCALAR", minmax=True)
            a_out = buf.floats(values, "VEC4" if path == "rotation" else "VEC3")
            samplers.append({"input": a_in, "output": a_out, "interpolation": "LINEAR"})
            channels.append({"sampler": len(samplers) - 1, "target": {"node": joint, "path": path}})
        animations.append({"name": name, "channels": channels, "samplers": samplers})

    # Idle: breathing bob, arms swing a little, head looks around.
    t = [0.0, 1.0, 2.0]
    clip(
        "Idle",
        [
            (0, "translation", t, [(0, 0.95, 0), (0, 0.93, 0), (0, 0.95, 0)]),
            (3, "rotation", t, [quat_axis((1, 0, 0), 4), quat_axis((1, 0, 0), -4), quat_axis((1, 0, 0), 4)]),
            (4, "rotation", t, [quat_axis((1, 0, 0), -4), quat_axis((1, 0, 0), 4), quat_axis((1, 0, 0), -4)]),
            (2, "rotation", [0.0, 0.5, 1.5, 2.0],
             [quat_axis((0, 1, 0), 0), quat_axis((0, 1, 0), 15), quat_axis((0, 1, 0), -15), quat_axis((0, 1, 0), 0)]),
        ],
    )
    # Wave: right arm up and waving, head tilts.
    tw = [0.0, 0.3, 0.6, 0.9, 1.2]
    clip(
        "Wave",
        [
            (4, "rotation", tw, [quat_axis((0, 0, 1), a) for a in (-140, -165, -135, -165, -140)]),
            (2, "rotation", [0.0, 0.6, 1.2],
             [quat_axis((0, 0, 1), 0), quat_axis((0, 0, 1), -10), quat_axis((0, 0, 1), 0)]),
            (0, "translation", [0.0, 0.6, 1.2], [(0, 0.95, 0), (0, 0.97, 0), (0, 0.95, 0)]),
        ],
    )

    doc = {
        "asset": {"version": "2.0", "generator": "olvin vertical_slice gen_assets.py (CC0)"},
        "scene": 0,
        "scenes": [{"nodes": [0, body_node]}],
        "nodes": nodes,
        "meshes": [{"name": "Robot", "primitives": prims}],
        "materials": [
            {"name": "RobotBody", "pbrMetallicRoughness": {"baseColorFactor": [0.85, 0.45, 0.12, 1.0],
                                                           "metallicFactor": 0.3, "roughnessFactor": 0.6}},
            {"name": "RobotVisor", "pbrMetallicRoughness": {"baseColorFactor": [0.1, 0.9, 1.0, 1.0]},
             "emissiveFactor": [0.1, 0.9, 1.0]},
        ],
        "skins": [{"name": "RobotRig", "joints": list(range(len(JOINTS))), "inverseBindMatrices": a_ibm,
                   "skeleton": 0}],
        "animations": animations,
        "accessors": buf.accessors,
        "bufferViews": buf.views,
        "buffers": [{"byteLength": len(buf.data),
                     "uri": "data:application/octet-stream;base64," + base64.b64encode(bytes(buf.data)).decode()}],
    }
    out = ROOT / "models" / "robot.gltf"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(doc, indent=1))
    print("wrote", out)


# ---------------------------------------------------------------- sounds

RATE = 22050


def write_wav(rel, samples):
    out = ROOT / rel
    out.parent.mkdir(parents=True, exist_ok=True)
    peak = max(1e-9, max(abs(s) for s in samples))
    gain = 0.8 / peak if peak > 0.8 else 1.0
    frames = b"".join(struct.pack("<h", int(max(-1.0, min(1.0, s * gain)) * 32767)) for s in samples)
    with wave.open(str(out), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(frames)
    print("wrote", out, f"({len(samples) / RATE:.2f} s)")


def door():
    n = int(RATE * 0.9)
    out = []
    rnd = random.Random(7)
    lp = 0.0
    for i in range(n):
        t = i / RATE
        chime = 0.0
        for start, f in ((0.0, 660.0), (0.18, 880.0)):
            if t >= start:
                u = t - start
                chime += 0.35 * math.sin(2 * math.pi * f * u) * math.exp(-u * 5.0)
        lp += 0.05 * (rnd.uniform(-1, 1) - lp)
        rumble = 2.5 * lp * min(1.0, t * 8) * math.exp(-t * 2.5) + 0.25 * math.sin(2 * math.pi * 55 * t) * math.exp(-t * 3)
        out.append(chime + rumble)
    write_wav("sfx/door.wav", out)


def hum():
    n = RATE * 2  # 110 Hz and 220 Hz fit whole cycles into 2 s: seamless loop
    out = []
    for i in range(n):
        t = i / RATE
        wob = 1.0 + 0.3 * math.sin(2 * math.pi * 0.5 * t)
        out.append(0.4 * math.sin(2 * math.pi * 110 * t) * wob + 0.15 * math.sin(2 * math.pi * 220 * t))
    write_wav("sfx/hum.wav", out)


def thud():
    n = int(RATE * 0.25)
    rnd = random.Random(3)
    lp = 0.0
    out = []
    for i in range(n):
        t = i / RATE
        lp += 0.15 * (rnd.uniform(-1, 1) - lp)
        out.append((0.8 * math.sin(2 * math.pi * 70 * t) + 1.5 * lp) * math.exp(-t * 22))
    write_wav("sfx/thud.wav", out)


def music():
    bpm = 120
    beat = 60 / bpm
    bars = 8  # 4 beats each: 16 s
    n = int(RATE * beat * 4 * bars)
    out = [0.0] * n

    def note(freq, start, length, vol, kind):
        s0 = int(start * RATE)
        for k in range(int(length * RATE)):
            i = s0 + k
            if i >= n:
                break
            u = k / RATE
            env = min(1.0, u * 60) * math.exp(-u * (2.5 if kind == "pad" else 6.0))
            ph = 2 * math.pi * freq * u
            if kind == "pad":
                v = math.sin(ph) + 0.3 * math.sin(2 * ph)
            else:
                v = 2 / math.pi * math.asin(math.sin(ph))  # triangle
            out[i] += vol * env * v

    a = 440.0
    hz = lambda semi: a * 2 ** (semi / 12)
    chords = [(-12, -9, -5), (-16, -12, -9), (-21, -17, -14), (-14, -10, -7)]  # Am F C G
    for bar in range(bars):
        root = chords[bar % 4]
        t0 = bar * 4 * beat
        for s in root:
            note(hz(s - 12), t0, 4 * beat * 0.95, 0.12, "pad")
        arp = [root[0], root[1], root[2], root[1] + 12, root[2] + 12, root[1] + 12, root[2], root[1]]
        for k, s in enumerate(arp):
            note(hz(s + 12), t0 + k * beat / 2, beat / 2 * 0.9, 0.18, "arp")
    write_wav("music/theme.wav", out)


if __name__ == "__main__":
    make_robot()
    door()
    hum()
    thud()
    music()
