# dialog

2026-07-10, golden pair via CLI, migrated to the Base UI base-nova wrapper.

## Changed

src/components/ui/dialog.tsx:2 now composes @base-ui/react/dialog with the stock base-nova popup, backdrop, title, and close button. src/App.tsx:195 now uses that stock close button instead of a custom header close control.

grep -n "radix-ui\|@radix-ui" src/components/ui/dialog.tsx is clean.

## Left alone

Dialog form and confirmation content in src/App.tsx remains application-specific.

## Behavior changes

The stock base-nova dialog backdrop and close-control styling replace the prior custom versions.

## Verify by hand

Open each dialog, confirm initial focus, Escape and outside-click dismissal, focus return, and the top-right close button.
