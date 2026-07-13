# button

2026-07-10, golden pair via CLI, migrated to the Base UI base-nova wrapper.

## Changed

src/components/ui/button.tsx:1 now uses @base-ui/react/button and the base-nova variants. package.json and pnpm-lock.yaml add @base-ui/react.

grep -n "radix-ui\|@radix-ui" src/components/ui/button.tsx is clean.

## Left alone

src/App.tsx keeps its domain-specific layout classes; Button consumers remain native buttons.

## Behavior changes

None.

## Verify by hand

Tab to a primary, ghost, destructive, and disabled button. Confirm focus rings, clicks, and disabled state work.
