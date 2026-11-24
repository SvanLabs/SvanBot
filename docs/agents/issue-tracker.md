# Issue tracker: GitHub

Issues live on GitHub in `SvanLabs/SvanBot` and are worked with `gh`. The box has `gh` 2.46, so sub-issues and blocking go through the REST API. This repo is the
operator's own. Never file issues anywhere else.

Agents work as the GitHub App `svanlabs[bot]`, never as the operator's account. Run `gh` and
`git push` with an installation token of the App in `GH_TOKEN`. On this box,
`~/.config/svanbot/app-token.sh` prints one that lasts an hour. The App's key stays outside the
repo.

## Operations

- **Create:** `gh issue create --title "…" --body-file <file>`. Every body ends with the
  `Generated-by:` line.
- **Read:** `gh issue view <n> --json number,title,body,labels,comments,assignees`.
- **List:** `gh issue list --state open --label v11 --json number,title,labels,assignees`.
- **Database id** (what the API calls below take, not the `#number`): `gh api repos/SvanLabs/SvanBot/issues/<n> --jq .id`.
- **Sub-issue:** `gh api --method POST repos/SvanLabs/SvanBot/issues/<map>/sub_issues -F sub_issue_id=<child-db-id>`.
- **Blocking:** GitHub's native dependencies. Add an edge with `gh api --method POST repos/SvanLabs/SvanBot/issues/<n>/dependencies/blocked_by
  -F issue_id=<blocker-db-id>`, and remove one with `gh api -X DELETE …/dependencies/blocked_by/<blocker-db-id>`.
- **Open blockers:** `gh api repos/SvanLabs/SvanBot/issues/<n> --jq .issue_dependencies_summary.blocked_by`.
  0 means the ticket can start.
- **Comment, label, close:** `gh issue comment`, `gh issue edit --add-label` / `--remove-label`, and
  `gh issue close`. Comment before closing: `gh issue close --comment` drops the comment on an issue
  that is already closed.
- One number space covers both issues and PRs. When a bare `#n` could be either, resolve it with
  `gh pr view` first.

PRs are not a request surface. `/triage` reads issues only.

## Wayfinding operations

- **Map:** **V11-00**, labelled `wayfinder:map`. Its body holds Destination, Notes, Order,
  Decisions so far, Not yet specified, and Out of scope.
- **Ticket:** a sub-issue of the map, titled `V11-<nn>: …`, with a `wayfinder:<type>` label plus
  `v11`.
- **Frontier:** the map's open sub-issues that have no open blocker and no `agent-claimed` label,
  plus every claimed one labelled `agent-handoff`. The first in map order wins.
- **Handoff:** a session that stops mid-ticket pushes its branch, posts the note from
  `bun tools/handoff.ts <ticket>` as a comment, and adds `agent-handoff`. The taker removes the
  label and works on from the branch the note names (`AGENTS.md`, "Handing off").
- **Claim:** `gh issue edit <n> --add-label agent-claimed`, as the session's first write. The label
  is the claim and nobody is assigned: GitHub cannot assign an issue to an App.
- **Resolve:**
  1. Post the answer as a comment.
  2. Close the ticket.
  3. Append a one-line gist with a link to the map's Decisions so far.

## When a skill says "publish to the issue tracker" or "fetch the ticket"

Publishing means creating a GitHub issue. Fetching means `gh issue view <n> --comments`.
