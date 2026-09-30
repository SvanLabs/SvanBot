"""The docker build context carries no secret (#583).

`.git/info/exclude` keeps the GitHub App private key out of git, and the build context never reads
it: docker reads the working tree. `COPY . .` in the Dockerfile then copies everything
`.dockerignore` did not drop into a build-stage layer, and a build-stage layer of a pushed image is
readable by anyone who can pull it. So a path git ignores is still a path docker ships, which is
what this pins.

The rule is stated against the bytes docker would actually copy: the walk is pruned by the ignore
file itself, so the set it yields is the build context, and no file in it may be a credential — or
an environment file, which holds one — except the ones the Dockerfile copies on purpose. On a
fresh clone that set still holds the whole repository, so the case is never vacuous; it simply has
nothing to find until a tree grows a key, which is exactly when it fires.
"""
import fnmatch
import os
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
IGNORE = ROOT / ".dockerignore"
# Credential shapes, plus `.env` and its variants: the API keys live in one of those.
SECRET_NAMES = ("*.pem", "*.key", "*.p12", "*.pfx", "*.jks", "*.ppk", "id_rsa*")
# Copied into the image on purpose (`COPY .env.example Cargo.toml ./`), so it must stay reachable.
ALLOWED = {".env.example"}
# A build context smaller than this means the walk pruned everything and is proving nothing.
MIN_SHIPPED = 50


def patterns() -> list[str]:
    """The `.dockerignore` entries, without comments or blanks."""
    out = []
    for line in IGNORE.read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            out.append(line.rstrip("/"))
    return out


def ignored(rel: str, pats: list[str]) -> bool:
    """Whether docker's ignore rules would drop `rel`, judged on the path and each of its parents."""
    parts = Path(rel).parts
    for i, part in enumerate(parts):
        prefix = "/".join(parts[: i + 1])
        if any(fnmatch.fnmatch(prefix, p) or fnmatch.fnmatch(part, p) for p in pats):
            return True
    return False


def shipped(pats: list[str]) -> list[str]:
    """Repository-relative paths docker would copy, pruned by the same rules it applies."""
    out = []
    for dirpath, dirnames, filenames in os.walk(ROOT):
        root_rel = Path(dirpath).relative_to(ROOT)
        dirnames[:] = [d for d in dirnames if not ignored(str(root_rel / d), pats)]
        for name in filenames:
            rel = str(root_rel / name)
            if not ignored(rel, pats):
                out.append(rel)
    return out


def secret_shaped(name: str) -> bool:
    return name == ".env" or name.startswith(".env.") or any(fnmatch.fnmatch(name, g) for g in SECRET_NAMES)


class BuildContextCarriesNoSecret(unittest.TestCase):
    def test_the_rules_can_tell_ignored_from_shipped(self):
        # A matcher that matched everything — or nothing — would make the case below pass for the
        # wrong reason, so the answers it must give are pinned here first.
        pats = patterns()
        self.assertTrue(ignored(".env", pats))
        self.assertTrue(ignored(".key/svanlabs.2026-09-28.private-key.pem", pats))
        self.assertTrue(ignored(".scratch/svanbot10/notes.md", pats))
        self.assertTrue(ignored("artifacts/svanbot10.db", pats))
        self.assertTrue(ignored("web/node_modules/react/index.js", pats))
        self.assertTrue(ignored("target/dev/gate/thing", pats))
        self.assertFalse(ignored("crates/libs/cards/src/cards.rs", pats))
        self.assertFalse(ignored(".env.example", pats), "the Dockerfile copies .env.example on purpose")

    def test_no_secret_shaped_file_survives_into_the_build_context(self):
        pats = patterns()
        context = shipped(pats)
        self.assertGreaterEqual(
            len(context),
            MIN_SHIPPED,
            "the walk yielded almost nothing, so this case proves nothing about the files it skipped",
        )
        leaked = [rel for rel in context if Path(rel).name not in ALLOWED and secret_shaped(Path(rel).name)]
        self.assertEqual(
            leaked,
            [],
            "these are inside the build context `COPY . .` copies; add them to .dockerignore (a path "
            "kept out of git by .git/info/exclude is not kept out of docker): " + ", ".join(leaked),
        )


if __name__ == "__main__":
    unittest.main()
