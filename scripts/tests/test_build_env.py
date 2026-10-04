"""The build environment: only the job that ships carries the commit id (#329).

`crates/apps/bot/src/lib.rs:57` reads `SVANBOT_COMMIT` with `option_env!`, and rustc records the value
of a variable that feeds `env!` in dep-info, so changing it recompiles `sv10-bot` and relinks every
binary linking it. The value is the commit being built, so it differs by definition from one run to
the next — which means any stage that exports it into a target directory shared with the gate
(`target/dev`, kept warm by the pre-commit hook and `check.sh`) rebuilds the crate on every release
that changed nothing, and a stage that alternates between set and unset rebuilds it every other run.

The decision on #329 is to bake the commit in the one job that ships and keep it out of every other
stage. The installer does that (`crates/apps/release/src/release.rs`, behind the `scripts/release.sh`
wrapper): each job names whether it carries the commit id, and the jobs that build into `target/dev` do
not. A unit test there (`only_the_shipped_build_carries_the_commit...`) pins the job list. These tests pin
the files that could set the variable at all, because this is a recorded escaped class — an exported var
the compiler tracks — and the fix is a convention nothing else enforces.

The measured cost, from the issue's own red test on this tree (`cargo build --profile gate -p
sv10-bot`, same machine): repeated with the same environment 0.2 s (a no-op), and 12 s for `sv10-bot`
alone when the value changes. A release relinks about eight binaries from it.
"""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "scripts"
WORKFLOWS = ROOT / ".github" / "workflows"
TESTS = SCRIPTS / "tests"
# `SVANBOT_COMMIT=...`, `export SVANBOT_COMMIT=...`, a workflow's `SVANBOT_COMMIT: ...`, and
# `env["SVANBOT_COMMIT"] = ...`. A read — `option_env!`, `${SVANBOT_COMMIT}`, `os.environ.get` — has
# none of these shapes, and `unset SVANBOT_COMMIT` is deliberately not one: keeping it out is the fix.
SETS_IT = re.compile(r"""(?:^|[\s;(&\[])(?:export\s+)?["']?SVANBOT_COMMIT["']?\]?\s*[:=]""")
# The one script allowed to put it into a build environment: the A/B harness, which pins it for the
# length of a run and refuses to write the live target directories at all. The job that ships is Rust now.
ALLOWED = {"scripts/build-ab.py"}


class OnlyTheShippingJobCarriesTheCommit(unittest.TestCase):
    def test_only_the_installers_release_job_sets_it_in_rust(self):
        found = sorted(
            str(path.relative_to(ROOT))
            for path in (ROOT / "crates" / "apps" / "release" / "src").rglob("*.rs")
            if '"SVANBOT_COMMIT"' in path.read_text()
        )
        self.assertEqual(
            found,
            ["crates/apps/release/src/release.rs"],
            "the installer's release job is the only Rust site that may put SVANBOT_COMMIT into a build",
        )

    def test_no_other_stage_sets_the_build_commit(self):
        found = {}
        for path in sorted(SCRIPTS.rglob("*.sh")) + sorted(SCRIPTS.rglob("*.py")) + sorted(WORKFLOWS.glob("*.yml")):
            # The tests that pin this rule are not build stages.
            if TESTS in path.parents:
                continue
            sites = [f"{n}: {line.strip()}" for n, line in enumerate(path.read_text().splitlines(), 1) if SETS_IT.search(line)]
            if sites:
                found[str(path.relative_to(ROOT))] = sites
        self.assertEqual(
            set(found),
            ALLOWED,
            "a stage outside the installer's shipping job and build-ab.py puts SVANBOT_COMMIT into a build "
            f"environment; a shared target directory then recompiles sv10-bot on every run (#329). Found: {found}",
        )


if __name__ == "__main__":
    unittest.main()
