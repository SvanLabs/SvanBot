#!/usr/bin/env python3
"""Survey the assembly of the hot loops in a built binary (0336).

    scripts/asm-survey.py BIN SYMBOL_SUBSTRING [SYMBOL_SUBSTRING ...]

For each matching function it reports what 0336 asks about: size, instruction count, conditional
branches, calls (so a panic or bounds-check path shows up as one), vector instructions and the
scalar bit intrinsics the evaluator actually uses. The hot loop a change belongs to is the *inline*
blob the profiler names, not the out-of-line copy, so both are listed — a small `.llvm.` copy next
to a multi-kilobyte caller means the work moved into the caller.

Same output before and after a codegen change (0337's LTO and codegen-units, 0338's PGO) is how the
"no new branches or panics in the hot path" claim is checked rather than asserted.
"""
import re, subprocess, sys

# objdump prints ` 120118:\tf3 0f b8 c2          \tpopcnt %edx,%eax`.
LINE = re.compile(r"^\s+([0-9a-f]+):\t((?:[0-9a-f]{2} )+)\s*\t(\S+)")
JCC = re.compile(r"^j(?!mp$)[a-z]+$")
# Packed-lane arithmetic: the `v`-prefixed SSE/AVX mnemonics, minus plain moves (`vmov*`, data
# movement the compiler emits for clearing and copying) and minus the *scalar* float forms (`…ss`,
# `…sd`) — f64 math uses SSE registers, so every `vaddsd` would otherwise read as vector code where
# the lanes hold nothing. `verr`/`verw` are the only non-SIMD false friends.
VEC = re.compile(r"^v(?!mov|err|erw)[a-z0-9]+(?<!ss)(?<!sd)$")
# The same function is emitted once per codegen unit; strip the unit's suffixes so the copies group.
HASH = re.compile(r"(\.llvm\.\d+|Cs[0-9A-Za-z]{16}_[a-z0-9_]+)$")
BIT = {"popcnt", "lzcnt", "tzcnt", "bsf", "bsr", "bswap", "pdep", "pext", "andn", "blsi", "blsr", "bzhi"}
# A call into these is a path that cannot happen in a proven-bound loop, but costs a compare.
PANIC = ("panic", "unwrap_failed", "panicking", "slice_index", "bounds")


def symbols(binary):
    out = subprocess.run(["nm", "--defined-only", "--print-size", binary], capture_output=True, text=True).stdout
    found = []
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 4:
            addr, size, _kind, name = parts
            found.append((int(addr, 16), int(size, 16), name))
    return found


def survey(binary, addr, size):
    end = addr + size
    dis = subprocess.run(
        ["objdump", "-d", f"--start-address={hex(addr)}", f"--stop-address={hex(end)}", binary],
        capture_output=True, text=True,
    ).stdout
    n = jcc = jmp = calls = vec = bit = panics = 0
    for line in dis.splitlines():
        m = LINE.match(line)
        if not m:
            continue
        n += 1
        op = m.group(3)
        if JCC.match(op):
            jcc += 1
        elif op == "jmp":
            jmp += 1
        elif op == "call":
            calls += 1
            target = line.split("call", 1)[1]
            if any(p in target for p in PANIC):
                panics += 1
        elif VEC.match(op):
            vec += 1
        elif op in BIT:
            bit += 1
    return n, jcc, jmp, calls, vec, bit, panics


def main(argv):
    if len(argv) < 3:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    binary, needles = argv[1], argv[2:]
    rows = []
    for addr, size, name in symbols(binary):
        if not size or not any(x in name for x in needles):
            continue
        rows.append((size, addr, name, survey(binary, addr, size)))
    seen = {}
    for size, _addr, name, stats in rows:
        base = HASH.sub("", name)
        seen.setdefault((base, size, stats), 0)
        seen[(base, size, stats)] += 1
    order = sorted(seen.items(), key=lambda kv: (-kv[0][2][0], -kv[1]))
    print(f"{'instr':>6} {'bytes':>7} {'jcc':>4} {'jmp':>4} {'call':>5} {'vec':>5} {'bit':>4} {'panic':>5}  function")
    for (base, size, (n, jcc, jmp, calls, vec, bit, panics)), count in order:
        short = base if len(base) <= 68 else base[:32] + "…" + base[-35:]
        copies = f" ×{count}" if count > 1 else ""
        print(f"{n:>6} {size:>7} {jcc:>4} {jmp:>4} {calls:>5} {vec:>5} {bit:>4} {panics:>5}  {short}{copies}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
