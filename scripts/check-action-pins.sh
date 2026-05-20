#!/usr/bin/env bash
# check-action-pins.sh — audit GitHub Action SHA pins in .github/workflows/.
#
# Walks every `uses: org/repo@<sha> # <version>` line, queries the current
# upstream SHA for that version via `git ls-remote`, and reports drift.
#
# Pass --update to rewrite drifted SHAs in place (review the diff before
# committing).
#
# Exit codes:
#   0  all pins current (or --update succeeded)
#   1  drift detected (without --update) or upstream lookup failed

set -euo pipefail

WORKFLOW_DIR=".github/workflows"
UPDATE=false

case "${1:-}" in
    --update) UPDATE=true ;;
    --help|-h)
        sed -n '2,12p' "$0" | sed 's/^# //; s/^#//'
        exit 0
        ;;
    "") ;;
    *)
        echo "unknown arg: $1" >&2
        echo "usage: $0 [--update]" >&2
        exit 2
        ;;
esac

if [[ ! -d "$WORKFLOW_DIR" ]]; then
    echo "error: $WORKFLOW_DIR not found (run from repo root)" >&2
    exit 1
fi

# Matches lines like:
#   - uses: org/repo@<40-char-sha> # version
#       uses: org/repo@<40-char-sha> # version
PIN_RE='uses:[[:space:]]+([A-Za-z0-9._-]+/[A-Za-z0-9._-]+)@([0-9a-f]{40})[[:space:]]+#[[:space:]]+(.+)$'

drift_count=0
ok_count=0
lookup_failures=0

# Read each pin line. `|| true` so an empty match set isn't a fatal exit.
while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    if [[ ! "$line" =~ $PIN_RE ]]; then
        continue
    fi
    repo="${BASH_REMATCH[1]}"
    pinned="${BASH_REMATCH[2]}"
    version="${BASH_REMATCH[3]}"

    url="https://github.com/$repo.git"
    # git ls-remote prints "<sha>\trefs/{tags,heads}/<ref>". For annotated
    # tags it lists both the tag object and the peeled commit (^{}); we
    # take the first row which is the right one for our pinning use case.
    upstream=$(git ls-remote "$url" "$version" 2>/dev/null | awk '{print $1; exit}')

    if [[ -z "$upstream" ]]; then
        printf '✗  %s  %s — could not resolve upstream\n' "$repo" "$version"
        lookup_failures=$((lookup_failures + 1))
        continue
    fi

    if [[ "$pinned" == "$upstream" ]]; then
        ok_count=$((ok_count + 1))
        printf '✓  %s  %s\n' "$repo" "$version"
    else
        drift_count=$((drift_count + 1))
        printf '→  %s  %s\n   %s\n→  %s\n' \
            "$repo" "$version" "$pinned" "$upstream"

        if $UPDATE; then
            # Rewrite every workflow file that mentions this exact pin.
            # We anchor on `$repo@$pinned` so collisions on the SHA alone
            # (e.g. two repos sharing a SHA — astronomically unlikely but
            # possible in principle) don't cross-contaminate.
            for f in "$WORKFLOW_DIR"/*.yml; do
                if grep -qF "$repo@$pinned" "$f"; then
                    tmp=$(mktemp)
                    sed "s|$repo@$pinned|$repo@$upstream|g" "$f" > "$tmp"
                    mv "$tmp" "$f"
                fi
            done
        fi
    fi
done < <(grep -hE "$PIN_RE" "$WORKFLOW_DIR"/*.yml 2>/dev/null || true)

echo
if [[ $ok_count -eq 0 && $drift_count -eq 0 && $lookup_failures -eq 0 ]]; then
    echo "no SHA-pinned actions found in $WORKFLOW_DIR"
    exit 0
fi

if [[ $drift_count -eq 0 && $lookup_failures -eq 0 ]]; then
    echo "$ok_count action pin(s) current"
    exit 0
fi

if $UPDATE && [[ $lookup_failures -eq 0 ]]; then
    echo "$drift_count pin(s) updated; review with: git diff $WORKFLOW_DIR"
    exit 0
fi

if [[ $lookup_failures -gt 0 ]]; then
    echo "$lookup_failures upstream lookup failure(s)"
fi
if [[ $drift_count -gt 0 ]]; then
    echo "$drift_count pin(s) drifted (run with --update to fix)"
fi
exit 1
