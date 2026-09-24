#!/bin/bash
#========================================================================================================================================
# CyborgAI CC BY-NC-ND 4.0 Creative Commons Attribution-NonCommercial-NoDerivatives 4.0 International  https://github.com/cyborg-ai-git 
#========================================================================================================================================

PACKAGE_NAME="$(basename "$(pwd)")"
DIRECTORY_BASE=$(dirname "$(realpath "$0")")

clear
echo "Usage with message: $0 'commit_message' release"

echo "🟢 RUN git $ $DIRECTORY_BASE"
current_time=$(date +"%Y.%-m.%-d%H%M")

CURRENT_DIRECTORY=$(pwd)

cd "$DIRECTORY_BASE" || exit

if [ -z "$1" ]; then
  comment="feat:$current_time"
else
  comment="$1"
fi

echo "$comment"
git config http.postBuffer 524288000
git fetch --all
git add . ;git commit -am "$comment";

git pull --rebase
git push origin main

if [ -z "$2" ]; then
  comment="$current_time"
else
  if [ -z "$3" ]; then
    #gh auth login
    git tag "v$current_time";git push origin --tags
    gh release create "v$current_time" --title ""v$current_time" Release" --notes "$PACKAGE_NAME"
  fi
fi

cd "$CURRENT_DIRECTORY" || exit