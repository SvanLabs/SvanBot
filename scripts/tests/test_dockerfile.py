"""The container image carries the tools the scripts inside it run (#582).

`Dockerfile` copies `scripts/` into the runtime stage and starts `scripts/container-start.sh`, which
runs `scripts/start.sh` exactly as a host does. Every command the session those two lead into
invokes therefore has to exist in the **runtime** stage — the build stage's packages do not reach
the image, and a tool missing there fails quietly, because the scripts redirect their own output to
a log.

`python3` was the first casualty: it was installed only where the binaries are compiled, so the
results monitor (`scripts/start.sh:126`) could not start at all, and `scripts/status.sh` reported
interpreter failures instead of totals. The same omission left `status.sh` without the `sqlite3` CLI
and without `curl`.

The rule is checked against the image's own install list. The table is written by hand, and covers
the session the entrypoint leads into — the fleet, its monitor, and the `status.sh` an operator runs
inside the container — not every script under `scripts/`: `clean.sh`, for instance, is copied in but
runs `git` and `findmnt`, which no container path installs, and installing them for a script the
container never invokes would be the wrong fix. Each entry is asserted to be a command this
repository really runs, so the table cannot drift into fiction.
"""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DOCKERFILE = ROOT / "Dockerfile"
# The last stage is the image: earlier stages are build scaffolding and their packages go nowhere.
STAGE = re.compile(r"^FROM\s+\S+", re.M)
INSTALL = re.compile(r"apt-get install[^\n]*?(?=\n\s*&&|\n\s*RUN|\n$)", re.S)

# command -> (package that provides it, where in this repository it is run).
REQUIRED = {
    "python3": ("python3", "scripts/status.sh runs its API checks with it, and the one-release monitor shim (#717) needs it"),
    "sqlite3": ("sqlite3", "scripts/status.sh reads hands and decisions with the CLI"),
    "curl": ("curl", "scripts/status.sh asks the dashboard API for the fleet summary"),
    "zstd": ("zstd", "scripts/clean.sh compresses rotated logs"),
    "pgrep": ("procps", "scripts/clean.sh and status.sh look for running builds and processes"),
}


def final_stage() -> str:
    """The text of the last `FROM` in the Dockerfile: the image the user gets."""
    text = DOCKERFILE.read_text()
    starts = [m.start() for m in STAGE.finditer(text)]
    assert starts, "Dockerfile has no FROM"
    return text[starts[-1] :]


def installed_packages() -> set[str]:
    packages = set()
    for block in INSTALL.findall(final_stage()):
        block = block.split("--no-install-recommends", 1)[-1]
        # The shell line continuations are part of the text, not of any package name.
        packages.update(block.replace("\\", " ").split())
    return {p for p in packages if not p.startswith("-")}


class RuntimeToolsAreInstalled(unittest.TestCase):
    def test_the_final_stage_is_the_one_being_read(self):
        # Reading the build stage instead would pass this suite on the broken image.
        stage = final_stage()
        self.assertIn("CMD [", stage, "the last FROM is not the image that declares the entrypoint")
        self.assertNotIn("cargo build", stage, "the last FROM is the build stage")
        self.assertGreaterEqual(len(installed_packages()), 3, "no install list was parsed")

    def test_every_command_the_runtime_scripts_run_is_installed(self):
        packages = installed_packages()
        missing = []
        for command, (package, why) in REQUIRED.items():
            if package not in packages:
                missing.append(f"{command} (package {package}) is missing: {why}")
        self.assertEqual(missing, [], "; ".join(missing))

    def test_the_table_names_commands_this_repository_really_runs(self):
        # A table of tools nothing calls would keep passing after the callers moved or were removed.
        scripts = "".join(p.read_text() for p in (ROOT / "scripts").glob("*.sh"))
        scripts += "".join(p.read_text() for p in (ROOT / "scripts").glob("*.py"))
        never = [c for c in REQUIRED if not re.search(rf"(^|[^\w/.-]){re.escape(c)}\s", scripts, re.M)]
        self.assertEqual(never, [], f"nothing under scripts/ runs these any more: {never}")


if __name__ == "__main__":
    unittest.main()
