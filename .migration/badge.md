# badge

2026-07-10, golden pair via CLI, migrated to the Base UI base-nova wrapper.

## Changed

src/components/ui/badge.tsx:1 now uses Base UI useRender and the stock base-nova variants.

grep -n "radix-ui\|@radix-ui" src/components/ui/badge.tsx is clean.

## Left alone

src/App.tsx keeps its status-specific semantic color classes.

## Behavior changes

None.

## Verify by hand

Confirm status and active-account badges remain readable at normal and keyboard-focus states.
