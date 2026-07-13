# input

2026-07-10, golden pair via CLI, migrated to the Base UI base-nova wrapper.

## Changed

src/components/ui/input.tsx:2 now uses @base-ui/react/input with the base-nova classes.

grep -n "radix-ui\|@radix-ui" src/components/ui/input.tsx is clean.

## Left alone

src/App.tsx form state and validation flow are unchanged.

## Behavior changes

None.

## Verify by hand

Edit each text field, tab through it, and confirm disabled save controls remain unavailable while an operation runs.
