#!/bin/sh
# One-time setup for a fresh clone. Idempotent - safe to re-run, and worth
# re-running after pulling changes to .githooks/.
#
#   scripts/initialize.sh                        set this machine up
#   scripts/initialize.sh --create-private-docs  build the private-docs branch
#                                                (once, on the machine that has
#                                                 the documents already)
#
# What a clone does NOT bring, and what this fixes:
#
#   .githooks/          tracked, but git runs .git/hooks - which is not tracked -
#                       so hooks never travel by themselves. Copied into place
#                       here, deliberately: see the note by the copy below.
#   private/            the standing-work documents. They live on the
#                       `private-docs` orphan branch and are checked out here
#                       as a git worktree.
#   remote.github.push  unpinned by default, so `git push github` could publish
#                       whatever branch you happen to be on.
#
# It does not fetch testdata/ - those are copyrighted game files. Use
# tools/fetch-testdata.sh MAX_DIR for those.

set -eu

root=$(git rev-parse --show-toplevel) || {
	echo 'initialize: not inside a git repository' >&2
	exit 1
}
cd "$root"

say() { printf '  %s\n' "$1"; }

# --- creating the branch (one-time) ------------------------------------------
#
# Orphan: no parent commit, so it shares no ancestor with dev or main and
# nothing can merge or fast-forward it into a published branch. Built with
# plumbing against a throwaway index, so the real index and working tree are
# never touched - the documents are ignored by .gitignore and stay that way.
if [ "${1:-}" = '--create-private-docs' ]; then
	if git show-ref --verify --quiet refs/heads/private-docs; then
		echo 'initialize: private-docs already exists - refusing to rebuild it' >&2
		exit 1
	fi
	[ -d private ] && [ -n "$(ls -A private 2>/dev/null)" ] || {
		echo 'initialize: private/ is missing or empty - nothing to capture' >&2
		exit 1
	}

	index=$(mktemp -u "${TMPDIR:-/tmp}/mme-private-index.XXXXXX")
	# -f because private/ is gitignored; --work-tree makes the paths land at the
	# tree root, which is what a worktree checked out AT private/ expects.
	GIT_INDEX_FILE="$index" git --work-tree=private add -A -f -- .
	tree=$(GIT_INDEX_FILE="$index" git write-tree)
	rm -f "$index"
	commit=$(git commit-tree "$tree" -m 'private: the standing-work documents

DESIGN.md, RULES.md and BACKLOG.md are the canonical set CLAUDE.md points at.
An orphan branch, so it shares no history with dev or main and cannot be
merged or fast-forwarded into anything that reaches the public remote.

Checked out at private/ via git worktree; see scripts/initialize.sh.')
	git branch private-docs "$commit"
	say 'created branch private-docs (orphan, no shared history)'
	say 'push it to the PRIVATE remote only:  git push origin private-docs'
	say 'then re-run scripts/initialize.sh to swap private/ over to the worktree'
	exit 0
fi

# --- per-machine setup --------------------------------------------------------

echo 'initialize: hooks'
# COPY the hooks rather than point core.hooksPath at the tracked directory.
# main and the archived branches do not contain .githooks/, so a hooksPath into
# the working tree evaporates the moment one of them is checked out - which is
# precisely when the "no commits on main" guard has to be there. .git/ belongs
# to no branch, so a copy survives every checkout. The cost is that this script
# must be re-run after .githooks/ changes.
#
# --git-common-dir, not --git-dir: private/ is a worktree, and worktrees share
# the one hooks directory.
hooks_dir="$(git rev-parse --git-common-dir)/hooks"
mkdir -p "$hooks_dir"
installed=''
for hook in .githooks/*; do
	[ -f "$hook" ] || continue
	name=${hook##*/}
	cp "$hook" "$hooks_dir/$name"
	chmod +x "$hooks_dir/$name"
	installed="$installed$name "
done
# An earlier revision set this, and it is the bug described above.
[ "$(git config --get core.hooksPath 2>/dev/null || true)" = '.githooks' ] \
	&& git config --unset core.hooksPath || true
say "installed ${installed}into ${hooks_dir#"$root/"}"
say 're-run this script after pulling changes to .githooks/'

echo 'initialize: public remote'
# Match on the URL, not the remote's name. Pinning the push refspec means
# `git push <remote>` cannot publish another branch even before the hook runs.
for remote in $(git remote); do
	case "$(git remote get-url "$remote")" in
		*github.com*)
			git config "remote.$remote.push" 'refs/heads/main:refs/heads/main'
			say "$remote: push pinned to main only"
			;;
	esac
done

echo 'initialize: private documents'
if git worktree list --porcelain | grep -qx "worktree $root/private"; then
	say 'private/ is already a worktree - nothing to do'
else
	# Local branch, else track the first remote that has one.
	if ! git show-ref --verify --quiet refs/heads/private-docs; then
		remote_ref=$(git for-each-ref --format='%(refname:short)' 'refs/remotes/*/private-docs' | head -n 1)
		[ -n "$remote_ref" ] && git branch --track private-docs "$remote_ref" >/dev/null 2>&1 || true
	fi

	if ! git show-ref --verify --quiet refs/heads/private-docs; then
		say 'no private-docs branch here or on a remote.'
		say 'On the machine that already has the documents, run once:'
		say '  scripts/initialize.sh --create-private-docs'
	elif [ -e private ] && [ -n "$(ls -A private 2>/dev/null)" ]; then
		say 'private/ exists and is not empty, so it was left alone.'
		say 'Compare it against the branch, then move it aside and re-run:'
		say '  mv private private.local && scripts/initialize.sh'
	else
		rmdir private 2>/dev/null || true
		git worktree add private private-docs >/dev/null
		say 'private/ -> worktree of private-docs'
	fi
fi

# CLAUDE.md is the project instructions. It lives with the standing work rather
# than on dev, because it names absolute local paths and the LAN remote's
# address and dev is squash-built into the published main. A symlink keeps one
# source of truth; /CLAUDE.md is gitignored, so the link is invisible here.
if [ -e private/CLAUDE.md ]; then
	if [ -L CLAUDE.md ]; then
		say 'CLAUDE.md -> private/CLAUDE.md (already linked)'
	elif [ -e CLAUDE.md ]; then
		say 'CLAUDE.md exists as a real file - left alone.'
		say 'Compare it with private/CLAUDE.md, then remove it and re-run.'
	else
		ln -s private/CLAUDE.md CLAUDE.md
		say 'CLAUDE.md -> private/CLAUDE.md'
	fi
fi

echo 'initialize: done'
