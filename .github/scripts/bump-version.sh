#!/bin/bash

# Script to bump version in Cargo.toml
# Usage: ./bump-version.sh [patch|minor|major]

set -e

# Default to patch if no argument provided
BUMP_TYPE="${1:-patch}"

# Get current version from Cargo.toml
CURRENT_VERSION=$(grep '^version = ' Cargo.toml | head -1 | cut -d'"' -f2)
echo "Current version: $CURRENT_VERSION"

# Parse version components
IFS='.' read -r -a version_parts <<< "$CURRENT_VERSION"
major=${version_parts[0]}
minor=${version_parts[1]}
patch=${version_parts[2]}

# Bump version based on type
case "$BUMP_TYPE" in
    major)
        major=$((major + 1))
        minor=0
        patch=0
        ;;
    minor)
        minor=$((minor + 1))
        patch=0
        ;;
    patch)
        patch=$((patch + 1))
        ;;
    *)
        echo "Error: Invalid bump type. Use 'patch', 'minor', or 'major'"
        exit 1
        ;;
esac

NEW_VERSION="$major.$minor.$patch"
echo "New version: $NEW_VERSION"

# Update version in Cargo.toml
if [[ "$OSTYPE" == "darwin"* ]]; then
    # macOS
    sed -i '' "s/^version = \".*\"/version = \"$NEW_VERSION\"/" Cargo.toml
else
    # Linux
    sed -i "s/^version = \".*\"/version = \"$NEW_VERSION\"/" Cargo.toml
fi

# Update Cargo.lock if it exists
if [ -f "Cargo.lock" ]; then
    cargo update --package crabmux || true
fi

echo "Version bumped to $NEW_VERSION"
echo "new_version=$NEW_VERSION" >> "$GITHUB_OUTPUT"