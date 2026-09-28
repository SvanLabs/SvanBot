"""The build environment: only the job that ships carries the commit id (#329).

`crates/apps/bot/src/lib.rs:57` reads `SVANBOT_COMMIT` with `option_env!`, and rustc records the value
of a variable that feeds `env!` in dep-info, so changing it recompiles `sv10-bot` and relinks every
binary linking it. The value is the commit being built, so it differs by definition from one run to
the next — which means any stage that exports it into a target directory shared with the gate
(`target/dev`, kept warm by the pre-commit hook and `check.sh`) rebuilds the crate on every release
that changed nothing, and a stage that alternates between set and unset rebuilds it every other run.

The decision on #329 is to bake the commit in the one job that ships and keep it out of every other
stage. `release.sh` does that: `start_job` takes the commit id per job, and the jobs that build into
`target/dev` pass `no-commit`. These tests pin the call sites and the scripts that could set the
variable at all, because this is a recorded escaped class — an exported var the compiler tracks — and
the fix is a convention nothing else enforces.

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
RELEASE = SCRIPTS / "release.sh"
TESTS = SCRIPTS / "tests"
# The stage job builds the shipped binaries into its own target directory; every other job shares the
# gate's. A job that builds into the shared one must not carry the commit.
SHARED_DIR = "$DEV"
# `SVANBOT_COMMIT=...`, `export SVANBOT_COMMIT=...`, a workflow's `SVANBOT_COMMIT: ...`, and
# `env["SVANBOT_COMMIT"] = ...`. A read — `option_env!`, `${SVANBOT_COMMIT}`, `os.environ.get` — has
# none of these shapes, and `unset SVANBOT_COMMIT` is deliberately not one: keeping it out is the fix.
SETS_IT = re.compile(r"""(?:^|[\s;(&\[])(?:export\s+)?["']?SVANBOT_COMMIT["']?\]?\s*[:=]""")
# The two sites allowed to put it into a build environment: the job that ships, and the A/B harness,
# which pins it for the length of a run and refuses to write the live target directories at all.
ALLOWED = {"scripts/release.sh", "scripts/build-ab.py"}


class OnlyTheShippingJobCarriesTheCommit(unittest.TestCase):
    def test_every_job_that_builds_into_the_shared_directory_unloads_it(self):
        text = RELEASE.read_text()
        calls = [line.strip() for line in text.splitlines() if line.strip().startswith("start_job ")]
        self.assertTrue(calls, "release.sh starts no jobs by name: this test is looking at the wrong shape")
        for line in calls:
            name, target, commit_id = line.split()[1:4]
            with self.subTest(job=name):
                if target == SHARED_DIR:
                    self.assertEqual(
                        commit_id,
                        "no-commit",
                        f"release.sh starts {name} in {SHARED_DIR} carrying the commit id: cargo tracks "
                        "SVANBOT_COMMIT, so that job recompiles sv10-bot and relinks its binaries on "
                        "every release even when no source changed — 12 s each, and 60 s of a "
                        "one-file release when both shared jobs exported it (#329)",
                    )
        self.assertEqual(
            [line.split()[3] for line in calls].count("with-commit"),
            1,
            "exactly one job — the release build into $STAGE — carries the commit id: the shipped "
            "binaries are the only ones whose --version has to name a commit (#329)",
        )
        self.assertIn("unset SVANBOT_COMMIT", text, "release.sh no longer keeps the commit out of the jobs that share the gate's cache (#329)")

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
            "a stage outside release.sh's shipping job and build-ab.py puts SVANBOT_COMMIT into a build "
            f"environment; a shared target directory then recompiles sv10-bot on every run (#329). Found: {found}",
        )


if __name__ == "__main__":
    unittest.main()
