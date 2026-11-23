#!/usr/bin/env python3
"""Writes the colour grading LUT strips of the looks packs (pure stdlib).

A strip is N slices of N x N texels side by side (N = 16: 256 x 16 pixels):
blue picks the slice, red runs along a slice, green down it. The input is the
sRGB-encoded colour, as in olvin_render::lut_strip. Run:
    python3 library/tools/gen_luts.py
"""
import pathlib
import struct
import zlib

N = 16
OUT = pathlib.Path(__file__).resolve().parents[1] / "looks-cinematic/assets/looks-cinematic/luts"


def png(path, width, height, rows):
    raw = b"".join(b"\x00" + bytes(r) for r in rows)

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    data = b"\x89PNG\r\n\x1a\n"
    data += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
    data += chunk(b"IDAT", zlib.compress(raw, 9))
    data += chunk(b"IEND", b"")
    path.write_bytes(data)


def strip(name, f):
    rows = []
    for g in range(N):
        row = []
        for b in range(N):
            for r in range(N):
                out = f(r / (N - 1), g / (N - 1), b / (N - 1))
                row += [max(0, min(255, round(c * 255))) for c in out] + [255]
        rows.append(row)
    png(OUT / name, N * N, N, rows)
    print("wrote", OUT / name)


def posterize(levels):
    return lambda c: round(c * (levels - 1)) / (levels - 1)


def ps1(r, g, b):
    # 5 levels per channel (a 15-bit look), washed towards a warm grey.
    p = posterize(6)
    r, g, b = p(r), p(g), p(b)
    luma = 0.3 * r + 0.59 * g + 0.11 * b
    mix = lambda c: luma * 0.3 + c * 0.7
    return (mix(r) * 1.04, mix(g), mix(b) * 0.94)


def night_vision(r, g, b):
    luma = min(1.0, (0.3 * r + 0.59 * g + 0.11 * b) ** 1.6 * 1.3)
    q = round(luma * 11) / 11
    return (q * 0.2, q, q * 0.25)


OUT.mkdir(parents=True, exist_ok=True)
strip("retro_ps1.png", ps1)
strip("night_vision.png", night_vision)
