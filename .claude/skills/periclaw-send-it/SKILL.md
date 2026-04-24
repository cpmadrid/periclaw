---
name: periclaw-send-it
description: Ship a PR end-to-end in the PeriClaw repo. Use when Chris says "send it", "ship it", "open the PR", "create a PR", "merge this", or asks to push, PR, and merge PeriClaw changes. PeriClaw-specific counterpart to the global /send-it; always verifies through ./dev and watches the macOS CI/release checks before squash-merging.
---

# PeriClaw Send-It

End-to-end PR flow for PeriClaw. The goal is to move the current branch from local changes to a merged PR without dropping repo-specific verification.

Core rules:

1. **Use `./dev`, not raw `cargo`, for repo checks.**
2. **Run `./dev ci` before pushing.**
3. **Watch both GitHub checks before merging:** `fmt + clippy + test` and `build-release (macos-14)`.
4. **Squash-merge only after green CI.**

## 1. Pre-flight

Start from the repo root:

```bash
git status --short --branch
git diff --stat
git diff --check
```

If the branch is `main`, create a feature branch with the `codex/` prefix before committing:

```bash
git switch -c codex/short-descriptive-name
```

Treat "send it" as approval to include the intentional current worktree changes. If unrelated or surprising files are present, inspect them and ask before staging. Never revert user changes to make the branch cleaner.

Review the actual diff before committing:

```bash
git diff
git diff --cached
```

## 2. PeriClaw verification

Always run the full local CI gate before pushing:

```bash
./dev ci
```

This runs the same critical pipeline as GitHub: format check, clippy with warnings denied, tests, and release build. If it fails, fix the failure, rerun the relevant narrow command, then rerun `./dev ci` before pushing.

For UI, scene, sprite, status-flow, or demo fixture changes, also consider the visual smoke path:

```bash
./dev demo-smoke
```

`demo-smoke` launches the bundled app in demo mode and prints the timing checklist. Use it when visual behavior is part of the change; otherwise note it as not run in the PR body. Remember that `./dev run` on macOS refreshes the `.app` bundle, while `cargo build` alone does not.

## 3. Commit

Use a conventional commit title with a PeriClaw scope when helpful:

```bash
git add <files>
git commit -m "$(cat <<'EOF'
fix(scene): short imperative summary

Briefly explain why the change exists and any behavior worth preserving.
EOF
)"
```

Good scopes for this repo include `app`, `scene`, `sprite`, `ui`, `net`, `config`, `demo`, `ci`, `docs`, and `build`. Keep the title specific enough that branch-protection check names remain readable, since GitHub may display them as `<PR title> / fmt + clippy + test`.

## 4. Push and PR

Push the current branch:

```bash
git push -u origin "$(git branch --show-current)"
```

Check whether the branch already has a PR:

```bash
gh pr view --json number,url,state,isDraft 2>/dev/null || true
```

If there is already a PR, edit or continue watching that PR instead of opening a duplicate. Otherwise create a PR with a clear body. PeriClaw does not currently have a PR template, so include these sections explicitly:

```bash
gh pr create --title "fix(scene): short imperative summary" --body "$(cat <<'EOF'
## Summary

- What changed.
- Why this is the right behavior for PeriClaw.

## Verification

- [x] ./dev ci
- [x] ./dev demo-smoke  # keep only if run
- [ ] Not run: <reason> # keep only if a relevant check was skipped

## Notes

- Any OpenClaw gateway assumptions, manual follow-up, or "N/A".
EOF
)"
```

Use `N/A` for empty sections rather than omitting the heading. If a visual smoke check was relevant but not run, say so directly in `Verification`.

## 5. Watch CI

After opening the PR, watch checks until completion:

```bash
gh pr checks <PR_NUMBER> --watch --fail-fast
```

Required checks:

- `fmt + clippy + test` (may appear prefixed by the PR title, e.g. `<title> / fmt + clippy + test`)
- `build-release (macos-14)`

If CI fails, inspect the failing job logs, fix the issue, commit, push, and watch again. Do not merge past red or pending checks.

## 6. Merge and clean up

When all checks are green:

```bash
gh pr merge <PR_NUMBER> --squash --delete-branch
git fetch origin main --prune
```

If this is a normal checkout, return to an updated `main`:

```bash
merged_branch="$(git branch --show-current)"
git switch main
git pull --ff-only origin main
git branch -d "$merged_branch"
```

If this is a linked worktree where switching to `main` is blocked, detach at the updated remote instead:

```bash
git switch --detach origin/main
```

## Guardrails

- Do not call `cargo` directly for PeriClaw's standard gates; use `./dev`.
- Do not skip `./dev ci` before pushing unless Chris explicitly asks for a draft or WIP PR.
- Do not merge a draft PR; stop after creating it.
- Do not merge with failed, pending, or missing required checks.
- Do not force-push `main` or rewrite merged history.
- Do not hide verification gaps. If `demo-smoke` or another relevant manual check was not run, record the reason in the PR body.
