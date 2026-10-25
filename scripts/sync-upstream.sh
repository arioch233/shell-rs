#!/usr/bin/env bash
# 只准备合并，推送和 PR 由工作流执行；调用前必须获取两个远程的最新引用
set -euo pipefail

branch=sync/upstream-update
output=${GITHUB_OUTPUT:-/dev/null}
summary=${GITHUB_STEP_SUMMARY:-/dev/null}

if [ -n "$(git status --porcelain)" ]; then
  echo 'Working tree must be clean' >&2
  exit 1
fi

upstream_sha=$(git rev-parse --verify refs/remotes/upstream/main)
git rev-parse --verify refs/remotes/origin/main >/dev/null
printf 'upstream_sha=%s\n' "$upstream_sha" >> "$output"

if git merge-base --is-ancestor upstream/main origin/main; then
  echo 'needs_pr=false' >> "$output"
  echo 'Upstream is already included in main' | tee -a "$summary"
  exit 0
fi

# 从远程同步分支接续人工提交，禁止重置或强推已有分支
if git show-ref --verify --quiet "refs/remotes/origin/$branch"; then
  git switch --detach "origin/$branch"
else
  git switch --detach origin/main
fi

for source in origin/main upstream/main; do
  if ! git -c core.hooksPath=/dev/null merge --no-ff --no-edit "$source"; then
    {
      echo 'Upstream sync stopped; remote branch was not changed.'
      echo
      echo 'Conflicting files (or inspect the log for another merge failure):'
      git diff --name-only --diff-filter=U
      echo
      echo 'Resolve locally, then push without force:'
      echo '```sh'
      echo 'git fetch origin'
      echo 'git fetch upstream main'
      if git show-ref --verify --quiet "refs/remotes/origin/$branch"; then
        echo 'git switch -c sync/upstream-update --track origin/sync/upstream-update'
      else
        echo 'git switch -c sync/upstream-update origin/main'
      fi
      echo 'git merge --no-ff origin/main'
      echo 'git merge --no-ff upstream/main'
      echo '# Resolve each conflict and complete that merge before continuing'
      echo 'git push -u origin sync/upstream-update'
      echo '```'
    } | tee -a "$summary"
    git -c core.hooksPath=/dev/null merge --abort || true
    exit 1
  fi
done

echo 'needs_pr=true' >> "$output"
echo 'Merge prepared; CI and manual review are required' | tee -a "$summary"
