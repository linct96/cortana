# dropdown-menu

2026-07-10, golden pair via CLI, migrated to the Base UI base-nova wrapper.

## Changed

src/components/ui/dropdown-menu.tsx:2 now composes @base-ui/react/menu through Portal, Positioner, and Popup. src/App.tsx:185 changes Radix asChild to Base UI render and onSelect to onClick; the destructive item uses the stock variant.

grep -n "radix-ui\|@radix-ui" src/components/ui/dropdown-menu.tsx is clean.

## Left alone

No submenu, checkbox, or radio menu consumers exist, so their generated stock support is unused but retained with the standard wrapper.

## Behavior changes

None; standard items still close on click.

## Verify by hand

Open the account menu, use Arrow keys and Enter to rename and remove, then confirm Escape closes it and focus returns to the trigger.
