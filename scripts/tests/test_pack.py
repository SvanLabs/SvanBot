"""sv10-pack against zlib (0229): zlib decodes every stream and frame our encoder writes, and our
decoder reads what zlib writes, at every level, with and without a preset dictionary, over random,
repetitive and JSON-like inputs. The committed fixtures (crates/deps/pack/tests/fixtures) pin the
decoder in `cargo test`; this pins the encoder against the reference implementation."""
from __future__ import annotations

import json
import os
import random
import struct
import subprocess
import unittest
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def zcli() -> str:
    env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get("CARGO_TARGET_DIR", str(ROOT / "target/dev")))
    out = subprocess.run(["cargo", "build", "--profile", "gate", "-q", "-p", "sv10-pack", "--example", "zcli", "--message-format=json"],
                         cwd=ROOT, env=env, capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        msg = json.loads(line)
        if msg.get("reason") == "compiler-artifact" and msg.get("target", {}).get("name") == "zcli" and msg.get("executable"):
            return msg["executable"]
    raise RuntimeError("zcli example not built")


def inputs() -> list[bytes]:
    rng = random.Random(229)
    rows = "\n".join(json.dumps({"hand_id": f"h{i}", "seat": i % 6, "action": rng.choice(["fold", "call", "raise"]),
                                 "amount": rng.randint(0, 5000), "pot": rng.randint(15, 9000)}) for i in range(600)).encode()
    return [b"", b"x", b"abc" * 5000, bytes(rng.getrandbits(8) for _ in range(20000)), rows,
            bytes(rng.choice(b"ab") for _ in range(70000)), rows[:5000] + bytes(rng.getrandbits(8) for _ in range(3000)) + rows[5000:]]


class PackAgainstZlib(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.bin = zcli()
        cls.dict = json.dumps([{"hand_id": "h0", "seat": 0, "action": "call", "amount": 100, "pot": 250}] * 40).encode()

    def run_cli(self, mode: str, level: int, data: bytes, dictionary: bytes | None = None) -> bytes:
        args = [self.bin, mode, str(level)]
        if dictionary is not None:
            path = Path(os.environ.get("TMPDIR", "/tmp")) / f"sv10-pack-dict-{os.getpid()}"
            path.write_bytes(dictionary)
            args.append(str(path))
        return subprocess.run(args, input=data, capture_output=True, check=True).stdout

    def test_zlib_decodes_our_streams(self):
        for level in (0, 1, 3, 4, 6, 9):
            for data in inputs():
                ours = self.run_cli("d", level, data)
                self.assertEqual(zlib.decompress(ours, -15), data, f"level {level}, {len(data)} bytes")

    def test_zlib_decodes_our_dictionary_streams_and_frames(self):
        for data in inputs():
            ours = self.run_cli("d", 6, data, self.dict)
            d = zlib.decompressobj(-15, zdict=self.dict)
            self.assertEqual(d.decompress(ours) + d.flush(), data)
            frame = self.run_cli("p", 6, data, self.dict)
            magic, dict_id, n, crc = frame[:3], frame[3], struct.unpack("<Q", frame[4:12])[0], struct.unpack("<I", frame[12:16])[0]
            self.assertEqual((magic, dict_id, n, crc), (b"SVZ", 1, len(data), zlib.crc32(data)))
            d = zlib.decompressobj(-15, zdict=self.dict)
            self.assertEqual(d.decompress(frame[16:]) + d.flush(), data)

    def test_we_decode_zlib_streams(self):
        for level in (1, 6, 9):
            for strategy in (zlib.Z_DEFAULT_STRATEGY, zlib.Z_FIXED, zlib.Z_HUFFMAN_ONLY, zlib.Z_RLE):
                for data in inputs():
                    c = zlib.compressobj(level, zlib.DEFLATED, -15, 8, strategy)
                    self.assertEqual(self.run_cli("i", 0, c.compress(data) + c.flush()), data)
                    c = zlib.compressobj(level, zlib.DEFLATED, -15, 8, strategy, zdict=self.dict)
                    self.assertEqual(self.run_cli("i", 0, c.compress(data) + c.flush(), self.dict), data)


if __name__ == "__main__":
    unittest.main()
