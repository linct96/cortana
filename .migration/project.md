# project

2026-07-10, golden pair via CLI, the complete shadcn surface now uses base-nova and Base UI.

## Changed

components.json:3 selects base-nova and supplies a valid current shadcn configuration. src/components/ui/button.tsx, input.tsx, badge.tsx, dialog.tsx, dropdown-menu.tsx, switch.tsx, alert.tsx, card.tsx, checkbox.tsx, and input-group.tsx use base-nova registry wrappers. src/App.tsx updates the menu render and click call sites, and uses stock card, alert, checkbox, input-group, and dialog-footer components. package.json and pnpm-lock.yaml replace all Radix packages with @base-ui/react.

grep -n "radix-ui\|@radix-ui" on every migrated component file is clean.

## Left alone

src/App.tsx domain layout and src/styles.css token values are intentionally retained; they are application styling, not duplicate primitive wrappers. Tauri code is unrelated to Radix and was not touched.

## Behavior changes

Dialog and dropdown visual treatment now follows base-nova. No Radix call-site behavior remains.

## Verify by hand

Open dialogs and the account menu, verify focus and keyboard navigation, toggle autostart, edit inputs, and run a profile switch. pnpm test and pnpm build pass.
