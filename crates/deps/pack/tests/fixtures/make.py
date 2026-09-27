#!/usr/bin/env python3
"""Regenerate the zlib-produced fixtures for tests/zlib.rs (0229).

Inputs are NAME.in; CASE-NAME.z is a raw DEFLATE stream zlib made from NAME.in; dict.bin is the
preset dictionary of the dict-* cases. The inputs are synthetic (no stored data) and small, and
cover every block type zlib emits: stored (level 0), fixed Huffman (Z_FIXED), dynamic Huffman at
greedy and lazy levels, Huffman-only and run-length strategies, many small blocks (memLevel 1
ends a block every 128 symbols), and a preset dictionary.
Run from this directory: python3 make.py
"""
import json
import random
import zlib

rng = random.Random(20260926)


def rows(n: int) -> bytes:
    out = []
    for i in range(n):
        out.append({"hand_id": f"h{100000 + i}", "seat": i % 6, "street": ["preflop", "flop", "turn", "river"][i % 4],
                    "action": rng.choice(["fold", "check", "call", "raise"]), "amount": rng.randint(0, 4000),
                    "pot": rng.randint(15, 9000), "profit": rng.randint(-2000, 2000) / 10})
    return "\n".join(json.dumps(r, separators=(",", ":")) for r in out).encode()


inputs = {
    "empty": b"",
    "one": b"a",
    "text": b"The quick brown fox jumps over the lazy dog. " * 20,
    "rows": rows(120),
    "noise": bytes(rng.getrandbits(8) for _ in range(3000)),
    "runs": b"".join(bytes([rng.getrandbits(8)]) * rng.randint(1, 300) for _ in range(60)),
    "long": rows(300),
}
dictionary = rows(200)
open("dict.bin", "wb").write(dictionary)
cases = [
    ("stored", 0, zlib.Z_DEFAULT_STRATEGY, ["text", "noise", "empty"]),
    ("fixed", 6, zlib.Z_FIXED, ["one", "text", "rows"]),
    ("fast", 1, zlib.Z_DEFAULT_STRATEGY, ["rows", "runs", "long"]),
    ("default", 6, zlib.Z_DEFAULT_STRATEGY, ["empty", "text", "rows", "noise", "runs", "long"]),
    ("best", 9, zlib.Z_DEFAULT_STRATEGY, ["rows", "long"]),
    ("huffman", 6, zlib.Z_HUFFMAN_ONLY, ["rows", "noise"]),
    ("rle", 6, zlib.Z_RLE, ["runs", "rows"]),
]
for name, data in inputs.items():
    open(f"{name}.in", "wb").write(data)
for tag, level, strategy, names in cases:
    for name in names:
        c = zlib.compressobj(level, zlib.DEFLATED, -15, 9, strategy)
        open(f"{tag}-{name}.z", "wb").write(c.compress(inputs[name]) + c.flush())
for name in ["rows", "long", "noise"]:
    c = zlib.compressobj(6, zlib.DEFLATED, -15, 1, zlib.Z_DEFAULT_STRATEGY)
    open(f"blocks-{name}.z", "wb").write(c.compress(inputs[name]) + c.flush())
for name in ["rows", "long", "text"]:
    c = zlib.compressobj(6, zlib.DEFLATED, -15, 9, zlib.Z_DEFAULT_STRATEGY, zdict=dictionary)
    open(f"dict-{name}.z", "wb").write(c.compress(inputs[name]) + c.flush())
