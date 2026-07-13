# switch

2026-07-10, golden pair via CLI, migrated to the Base UI base-nova wrapper.

## Changed

src/components/ui/switch.tsx:1 now uses @base-ui/react/switch and Base UI checked and disabled data attributes.

grep -n "radix-ui\|@radix-ui" src/components/ui/switch.tsx is clean.

## Left alone

src/App.tsx continues to own the autostart mutation.

## Behavior changes

None.

## Verify by hand

Toggle autostart with mouse and Space. Confirm the thumb moves, the setting persists, and busy state prevents repeat input.
