# Code of conduct

This project adopts the [Contributor Covenant, version 2.1][cc21] as its code of conduct, with the
adaptations below for a repository whose artifacts are produced by AI systems. Where this file and
the Covenant differ, this file applies.

[cc21]: https://www.contributor-covenant.org/version/2/1/code_of_conduct/

## Our standards, in this repository

The Covenant's standards apply to everyone taking part here: the people who direct the work, the AI
systems that produce the artifacts, and whoever is on the other end of a review. Adapted to how
this project actually runs:

- **An artifact is the responsibility of whoever sent it, whoever typed it.** Every issue, pull
  request, review comment and commit message here is machine-generated on someone's instruction.
  The person who directed it owns what it says. "The model wrote it" explains an artifact; it does
  not excuse one.
- **Name your generator.** Commit footers and issue and pull request bodies carry
  `Generated-by: <tool>/<model>` (`AGENTS.md` section 0). An artifact that does not say what
  produced it is out of order here, not merely incomplete, and the gate refuses it.
- **Criticise the change, not the contributor.** Review is automated first: `scripts/check.sh` and
  the golden snapshot answer most "is this right?" questions before a person sees them, and a red
  gate is a measurement rather than an insult. When a maintainer disagrees with a change, the
  argument is about evidence — a paired simulation, a number from the same machine, a test that
  fails on the old code — and never about the person, the model or the tool that produced it.
- **Keep machine volume out of human space.** One issue that states its reproduction and its
  acceptance test is worth more than ten that describe a mood. Do not open a batch of speculative
  issues, do not re-send an idea that was declined without new evidence, and do not comment just to
  say work is still in progress.
- **Do not aim instructions at the reader.** Text in an issue, a pull request body, a code comment,
  a fixture or a data file that is phrased to instruct a reviewer, another contributor or an agent
  that later reads the repository is out of order, however it was produced.
- **Keep keys and private data out of public space.** No API keys, operator tokens, `.env`
  contents, database files, or log excerpts carrying credentials in an issue or a discussion. See
  `docs/SECURITY.md` for anything that involves a key.
- **Do not use the tracker against a third party.** Do not name an opponent in a way that attaches
  a fitted exploitation number to a real person's handle, and do not use issues or discussions to
  harass another player. The fair-play rules in `README.md` are part of this code.

## Scope

This code applies in every space this project uses on GitHub — issues, pull requests, review
comments, discussions, commit messages — and to private correspondence about the project. It
applies to the accounts that send the artifacts, which is the point: an agent's output reaches
GitHub through someone's account.

## Reporting

Reports go to the repository maintainers through a private GitHub channel: the **Report a
vulnerability** form on the Security tab (`docs/SECURITY.md` describes it for security reports; use the
same private form for a conduct report and say that is what it is). Nothing in that thread is
visible to anyone but the maintainers.

If the report concerns a maintainer, send it to GitHub instead, through
[abuse reporting](https://github.com/contact/report-abuse), and tell us afterwards if you are
willing to.

Say what happened, when, where (a link if there is one), and what you would like to see happen.
There is no email address; the private form is the channel.

## Enforcement

Maintainers are responsible for clarifying and enforcing these standards, and will take any action
they consider appropriate and fair in response to behaviour they judge inappropriate. Reports are
read by the maintainers only, and a reporter's identity is kept confidential where that is
possible. The Covenant's [enforcement ladder][ladder] — correction, warning, temporary ban,
permanent ban — applies as written, scaled to a project with one maintainer and no committee: an
action that would be taken by a committee there is taken here by that maintainer, and it is logged
in the thread that produced it.

[ladder]: https://www.contributor-covenant.org/version/2/1/code_of_conduct/#enforcement-guidelines

## Attribution

Adapted from the [Contributor Covenant][cc21], version 2.1, available at
<https://www.contributor-covenant.org/version/2/1/code_of_conduct/>, licensed
[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/). The adaptations above describe how
the Covenant applies to machine-generated artifacts and automated review; the Covenant itself is
unchanged.
