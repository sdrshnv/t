#!/usr/bin/env bash
set -euo pipefail
: "${RELEASE_TAG:?release tag is required}"
: "${GH_REPO:?GitHub repository is required}"

if draft=$(gh release view "$RELEASE_TAG" --json isDraft --jq .isDraft 2>/dev/null); then
    if [[ "$draft" != true ]]; then
        echo "Refusing to overwrite published release $RELEASE_TAG" >&2
        exit 1
    fi
else
    gh release create "$RELEASE_TAG" --verify-tag --draft \
        --title "$RELEASE_TAG" --generate-notes
fi

# Retrying a failed upload may replace assets only while the release is a draft.
gh release upload "$RELEASE_TAG" dist/*.tar.gz dist/SHA256SUMS dist/install.sh --clobber
gh release edit "$RELEASE_TAG" --draft=false --latest
