# Genie design system

The current application is the reference. This package extracts its foundations and specifies its reusable patterns without redesigning the product. Tracker: **GNE-25**.

## Source and preview

- `web/src/shared/ui/tokens.css` is the token source used by the application.
- `tokens.json` is a generated [DTCG 2025.10](https://www.designtokens.org/tr/2025.10/format/) document. Its twelve groups cover dark/light × violet/blue/teal/green/orange/rose. Colors use explicit sRGB components and alpha; dimensions use CSS pixels; shadow is a composite token. CSS names and original expressions live under `$extensions.com.genie`.
- `web/design-system.html` is a live specimen page using application styles and the actual React status/priority icons. Run `npm run dev:web -- --port 5178` and open `http://localhost:5178/design-system.html`. It requires no tracker login. Switch both appearance controls and inspect focus/disabled states.
- Browser capture, the offline Figma plugin, plugin build/test code and agent instructions live in the separate private [genie-figma-tools repository](https://github.com/grigoryshulga/genie-figma-tools).

After editing tokens, run:

```sh
node scripts/design-system/export.mjs
node scripts/design-system/export.mjs --check
npm test
```

The test suite rejects stale generated output, missing/circular references, inconsistent appearance inventories, and insufficient action foreground contrast. The exporter only supports the CSS expressions used by the source and rejects unsupported ones. Color mixes are resolved in sRGB and rounded to 8-bit values; maximum channel rounding is half a step. The browser remains authoritative for rendering.

## Transfer to Figma

Export current `tokens.json` here, then use the separate [toolkit](https://github.com/grigoryshulga/genie-figma-tools). Its [agent workflow](https://github.com/grigoryshulga/genie-figma-tools/blob/main/docs/agent-workflow.md) covers local plugin setup, importing tokens/components, capturing real screens and verifying editable native layers. The toolkit owns that implementation and its tests; Genie owns these application foundations and their export.

`--measure: 64ch` stays documented rather than exported as an inaccurate fixed dimension: `ch` depends on font metrics.

## Foundations

| Family | Contract |
| --- | --- |
| Surfaces | `bg` canvas; `bg-side` navigation; `bg-panel` form/panel; `bg-raised` elevated detail; `bg-card` cards/dialogs; `bg-hover` and `bg-active` interaction; `bg-group`/`bg-sunken` grouped content |
| Borders | `line-soft`, `line`, `line-strong`; use `accent-line` for selected states |
| Text | `text` main; `text-body` prose; `text-2` secondary; `muted` metadata; `off` inactive decoration |
| Actions | `accent` + `on-accent`; `amber` + `amber-foreground`; `red` + `on-danger`. Hover preserves the paired foreground. Disabled controls use opacity 0.5 and suppress filled-button hover effects. |
| Status | `status-*` is shared by board headings, SVG icons, and epic progress. Approved is blue; review is green. Completed status stays violet independently of action accent. Glyph shape and labels also carry status. |
| Roles | `role-*` is a fixed identity fill paired with `on-role`; `role-text-*` adapts to the theme for readable role labels. Role identity and task status are distinct concepts. |
| Charts | `chart-1` through `chart-4` plus `chart-other`; semantic success/failure charts use `green`/`red`. Legends provide the meaning. |
| Spacing | 2, 4, 6, 8, 10, 12, 16, 20, 24, 28, 32 px; `space-N` is N × 2 px. The scale covers repeated padding/gaps, not every layout coordinate. |
| Radius | xs 4; sm 6; default 8; popover 10; large 12; pill 999. Circular avatars use 50%. |
| Controls | small 28; default 32; field 36; touch minimum 40 px at the existing mobile breakpoint. Auth fields use 40 px and the login action 44 px. |
| Layout | content column 840 px, prose measure 64ch. Sidebar/detail rail and responsive breakpoints remain in application layout CSS. |

## Typography styles

Use Geist for UI/prose and Geist Mono for identifiers, code, and numerical metadata. Preserve the CSS font stacks as browser fallbacks; Figma font-family variables contain the first family. Use regular 400, medium 500, and semibold 600.

| Suggested Figma style | Size / weight / line-height | Source |
| --- | --- | --- |
| UI/Body | 13 / 400 / 150% | `font-size-body`, `line-height-body` |
| UI/Small | 12 / 400 / 150% | `font-size-small` |
| UI/Label | 12.5 / 500 / 150% | `font-size-label` |
| UI/Caption | 11 / 400 / 150% | `font-size-caption` |
| UI/Metadata | 11.5 / 400 / 150% | `font-size-meta` |
| Heading/Panel | 15 / 600 / 150% | `font-size-heading` |
| Heading/Dialog | 18 / 600 / 150% | `font-size-title` |
| Heading/Section | 20 / 600 / 130% | `font-size-section` |
| Heading/Page | 24 / 600 / 125%, tracking -1% | `font-size-page` |
| Code/Small | Geist Mono 12 / 400 / 150% | `mono`, `font-size-small` |

Micro 10 px and emphasis 14 px are also exported. One-off 10.5/16/22 px sizes and icon illustration lettering remain local; they are not additional default text styles.

## Component inventory

Use horizontal/vertical auto layout matching the existing CSS, bind fills/strokes/radii/gaps to the imported variables, and keep labels as text properties. SVG source stays editable; use `web/src/shared/ui/icons.tsx` and `web/src/entities/task/ui/StatusIcon.tsx` for the icon artwork. Keep the variant axes below separate to avoid duplicating unrelated components.

| Family | Variants and states | Code reference / construction |
| --- | --- | --- |
| Button | default, primary, ghost, amber, danger × default/small × resting/hover/focus/disabled | `.btn`; 32 px, horizontal padding 12, radius 8; small 28 px/padding 8. Filled foregrounds follow the action pair. Focus outline 2 px `accent-2`, offset 1. |
| Icon button | resting/hover/focus/disabled | `.icon-btn`; 30 × 30, radius 8; contextual compact sizes remain explicit. Icon swap property, accessible name in code. |
| Field | input/select/textarea × resting/focus | `.field`; 8 px label gap, 36 px field, strong border, panel surface, padding 12. Focus accent border. Textarea grows vertically. |
| Chip input | empty/values/focused | `ChipInput.tsx`, `.chips`; min height 36, 24 px chips with remove icon; shared field border and radius. |
| Badge | default/accent/amber/green/red/muted | `Section.tsx` / `.ag-badge`; content-sized label. Status and role labels retain their own semantics. |
| Dialog | standard/wide | `Modal.tsx`; width min(660,94vw) or min(980,94vw), radius 12, card surface, strong border, overlay shadow, header/body/footer. Mobile uses full width and square outer corners. |
| Confirmation | normal/destructive/busy | `ConfirmDialog.tsx`; composes Dialog and Button; optional confirmation checkbox. |
| Menu | closed/open, item resting/hover/danger | `Menu.tsx` / `.rp-menu`; raised/card surface, shared border, hover fill, shadow; preserve Escape/outside-click behavior. |
| Navigation | resting/hover/selected, full/compact | `.nav-item`, `.team-link`, `.seg`; shared hover/active surface, selected text; counts and status are optional slots. |
| Task status | 11 statuses, icon size property | `StatusIcon.tsx`; normalized 16 × 16 SVG viewbox, theme-aware semantic fills/strokes. Priority is a separate family, P0–P4. |
| Avatar | role/person, compact/medium | `.av`; circular, role identity fill and `on-role` glyph, separate role label colors. User images keep the existing image component. |
| Feedback | success/warning/error/neutral | `.ag-badge`, notices and toast styles; shared semantic foreground/background/border tokens; retain message text. |
| Chart | grouped/stacked | `BarChart.tsx`; common chart series colors, visible legend and textual totals. |
| Image preview | thumbnail/inline/full/lightbox | `ImagePreview.tsx`; shared surfaces/radii, image scrim, paired danger foreground for close hover. |
| Rich text | reading/editing | `Markdown.tsx`, docs editor and `.md`; shared body/secondary colors, borders and typography; content-specific block geometry remains local. |

## Audit decisions and intentional exceptions

Resolved drift: duplicate palette declarations now live in one token source; role badges and avatars share identity fills; role labels have light-theme values; task icons, board tints and progress use the same statuses; light-theme yellow is darker; charts use theme-aware series; overlay scrims and filled foregrounds are shared; compact buttons have one base definition; detail-toolbar styles no longer override filled foregrounds; disabled filled buttons do not brighten on hover; login form text keeps a fixed dark-scene palette even when workspace light mode is selected.

Literal values still allowed outside tokens:

- The `.login-*` illustration and its SVG gradients are a deliberately fixed, dark, art-directed scene. Their geometry, decorative colors and motion are not workspace semantic tokens. Form text is scoped to that dark palette. Existing reduced-motion behavior is preserved.
- Theme-picker previews show both themes simultaneously through fixed `preview-*` tokens; accent swatches use fixed `accent-choice-*` tokens.
- Telegram's QR code stays dark-on-white for scanning. Avatar JPEG compositing uses a fixed neutral background so a saved image does not depend on the viewer's current theme.
- Browser meta theme-color has fixed early-boot values for the two themes, independent of loaded CSS.
- SVG icon geometry, graph coordinates, responsive breakpoints, proportional dimensions, layout widths, uncommon spacing and component-specific radii are local geometry rather than conflicting semantic values.
