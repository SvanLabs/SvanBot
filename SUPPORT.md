# Support

SvanBot is self-hosted software with one maintainer and no support contract. This file says where a
question goes, what to include with it, and what nobody here will do for you.

## Read first

Three documents answer most questions before anyone has to:

| Document | Answers |
|---|---|
| `README.md` | What the project is, what it needs, and the commands to start it |
| `docs/GUIDE.md` | Every configuration key, the dashboard panel by panel, the decision path, the learner |
| `docs/OPERATIONS.md` | The runbook: everyday commands, benchmarks, fault drills, the host checklist |
| `docs/LESSONS.md` | Why the rules are what they are, each one a mistake already paid for |

If you are about to change the code, read `CONTRIBUTING.md` and `AGENTS.md` before you start; they
are the standard the gate enforces.

## Where things go

| You have | Go to |
|---|---|
| A question about using, configuring or extending it | **Discussions → Q&A** |
| A bug you can describe, with a way to see it | **Issues → Bug report** (template `01-bug.yml`) |
| A specific change, with an acceptance test | **Issues → Task** (template `02-task.yml`) |
| A vulnerability, or anything involving a key | The private advisory form — `SECURITY.md` |
| A rule, a ruling, an account problem, another bot | Open Poker, not this repository |

Blank issues are disabled: a bug report and a task ask for different things, and the forms ask for
them. A question asked as an issue will be moved to Discussions rather than answered there.

## What to include

- **The build.** The dashboard's `/api/health` endpoint reports the commit the running binary was
  built from, and `scripts/status.sh` reports what is installed and how each bot is doing.
- **The configuration that matters.** Which `SVANBOT_*` keys you set and what you set them to —
  names and non-secret values, never keys or the operator token.
- **The command and its output.** The smallest one that shows the problem: a `sim` or `review`
  invocation, a `scripts/check.sh` run, a request to the API.
- **What you expected, and what happened instead.**
- **Log lines with credentials removed.** The gate and the pre-commit hook refuse keys in commits;
  the same care applies to a pasted log.

## What is not supported

- **Running the fleet for you.** Nobody here will take your API key, log in as you, configure your
  machine or operate your bots. That is not a service this project offers, and a key should not be
  handed to anyone who offers it.
- **API keys and accounts.** Not supplied, not created, not obtained. They come from openpoker.ai
  (dashboard → bot → Self Host), and account problems go to Open Poker.
- **Paid support, priority or a response time.** There is one maintainer, and the work here is
  directed rather than demanded. An unanswered issue means nobody has had time yet; it is not a
  queue position and not a promise.
- **Guarantees about poker results.** No win rate, rank or profit is promised, and no support
  request will be answered with one. Chips are virtual and the leaderboard is a game; the project
  measures what a change does to expected chips and reports the measurement, nothing more.
- **Forks and modified trees.** Support is for the code as it is published. If you have changed it,
  the first question will be whether `scripts/check.sh full` passes on your tree.
- **Other platforms.** Linux x86-64, CPU only, is what this is built and tested for. Windows,
  macOS, ARM and GPU paths are not supported and none is planned.
- **Help doing something the fair-play rules forbid.** No collusion, no chip dumping, no extra
  accounts, no scraping a rival's play, and no assistance towards any of them.

## How contributions work here

Every artifact in this repository — code, issues, pull requests, review comments, commit messages —
is produced by an AI system and names it with a `Generated-by: <tool>/<model>` line
(`AGENTS.md` section 0). A human may direct the work, choose the goal and approve the result; the
artifact itself must be machine-produced. Hand-written contributions are declined however good they
are, because the provenance rule is what makes the rest of the review meaningful.

That is also why a question here is welcome from an agent as much as from a person: ask it clearly,
say what produced it, and put it in the right place.
