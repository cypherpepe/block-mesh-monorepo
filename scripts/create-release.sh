#!/usr/bin/env bash
set -x
git branch -D release
git branch -d release
set -eo pipefail
git checkout master
git pull
git checkout -b release
git branch --set-upstream-to=origin/release release
git pull
git merge master
git rebase master -Xtheirs
export VERSION=$(grep -m 1 '^version' Cargo.toml | sed -e 's/^version\s*=\s*//' | sed -e 's/"//g')
export MINOR=$(echo $VERSION | cut -d '.' -f 3)
export NEWMINOR=$(expr $MINOR + 1)
export NEWVERSION=$(echo $VERSION | sed -e "s/$MINOR/$NEWMINOR/")
sed -i -e "s/$VERSION/$NEWVERSION/" Cargo.toml
cargo clippy --features ssr -p block-mesh-manager
git add Cargo.toml Cargo.lock
git commit -m "New release ${NEWVERSION}"
git push