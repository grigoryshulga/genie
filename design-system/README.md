# Genie design system

The application is the reference. This package extracts its foundations and specifies its reusable patterns. Tokens landed with the ink-and-paper redesign (epic GNE-40, foundations H21-H23); the cleanup that closed the migration is H33 (GNE-69).

## Source and preview

- `web/src/shared/ui/g-tokens.css` is the only token source. It declares the raw palette and the type/space/radius scales on `:root`, maps the semantic layer for ink, and overlays paper on `[data-theme="paper"]`. Components name semantic tokens only.
- `web/src/shared/ui/components.css` implements the `g-*` classes. React wrappers live in `web/src/shared/ui/kit/` and are re-exported by `web/src/shared/ui`. Prefer the wrapper: it carries the accessibility behaviour (focus trap, escape handling, roving menu focus) that a class name cannot.
- `tokens.json` is a generated [DTCG 2025.10](https://www.designtokens.org/tr/2025.10/format/) document with one group per theme (`ink`, `paper`), each fully resolved. Colors use explicit sRGB components and alpha; dimensions use CSS pixels; shadow is a composite token. CSS names and original expressions live under `$extensions.com.genie`.
- `web/design-system.html` is a live specimen of the real kit. Run `npm run dev:web` and open `http://localhost:5173/design-system.html`; it requires no tracker login. Switch the theme select to compare both sheets.
- Browser capture, the offline Figma plugin, plugin build/test code and agent instructions live in the separate private [genie-figma-tools repository](https://github.com/grigoryshulga/genie-figma-tools).

After editing tokens, run:

```sh
node scripts/design-system/export.mjs
node scripts/design-system/export.mjs --check
npm test
```

The test suite rejects stale generated output, mismatched exported theme keys, missing/circular references, and insufficient action foreground contrast. The exporter only supports the CSS expressions used by the source and rejects unsupported ones; font-relative `em`/`ch` values and fluid `clamp()` sizes are deliberately kept in CSS because they have no context-independent DTCG dimension. DTCG dimensions accept only `px` or `rem`. Motion values also stay CSS-only in this Figma bundle.

## Transfer to Figma

Export current `tokens.json` here, then use the separate [toolkit](https://github.com/grigoryshulga/genie-figma-tools). Its [agent workflow](https://github.com/grigoryshulga/genie-figma-tools/blob/main/docs/agent-workflow.md) covers local plugin setup, importing tokens/components, capturing real screens and verifying editable native layers. The toolkit owns that implementation and its tests; Genie owns these application foundations and their export.

## Foundations

Layer 1 holds the palette; layer 2 names the meaning. A component uses layer 2 and never a raw hex or a layer 1 value.

| Family | Contract |
| --- | --- |
| Surfaces | `g-bg` canvas; `g-surface` panels and cards; `g-surface-raised` elevated detail and popovers; `g-surface-hover` and `g-surface-active` interaction |
| Borders | `g-border` hairline; `g-border-strong` for edges that must survive a busy surface; `g-control-border` for the 3:1 outline of a field |
| Text | `g-text` primary; `g-text-2` secondary; `g-text-muted` metadata |
| Actions | `g-accent` + `g-on-accent`; `g-signal` + `g-on-signal`; `g-danger` + `g-on-danger`. A filled pair is never mixed across families, and hover keeps the paired foreground. |
| Themes | `ink` (default) and `paper`; the preference is system, ink, or paper. `system` follows `prefers-color-scheme`, and both sheets are specified for every component. |
| Status | Icons, board tints and epic progress share the semantic colours; glyph shape and the label also carry the status, so colour is never the only signal. |
| Charts | `g-viz-1` to `g-viz-4` for series, `g-viz-grid` for the grid; at most four series with direct labels. `paper` darkens the orange to hold 3:1 against the surface. |
| Avatars | One shape per kind of actor (`agent`, `human`) plus the name; identity is never carried by colour alone. |
| Agent roles | `g-agent-<role>` with `g-on-agent` for the nine standard roles, the same in both themes so a role is recognised anywhere: avatars (`Avatar tone`), the team graph and role lists. A custom role takes the colour of the role it extends. The hues share one lightness with the brand green and stay clear of signal orange and danger red. |
| Spacing | 4 px base: 4, 8, 12, 16, 24, 32, 48, 64, 96. Repeated padding and gaps use it; layout coordinates stay local geometry. |
| Radius | small 8 (controls, rows), medium 12 (fields, list rows), large 20 (cards), extra-large 28 (hero panels), pill 999 (buttons, chips, tags). |
| Controls | 40 px roomy, 32 px dense (`sm`), rows 44 px. The dense variant is for lists, toolbars and settings, not for touch-first screens. |
| Layout | content column 840 px, prose measure 64ch, board lane 344 px, card 288 × 144. Sidebar/detail rail and responsive breakpoints remain application layout CSS. |

Board cards use `g-shadow-board-card`, a short unblurred lift, without changing shared card or overlay shadows. Each of the five existing lanes uses its own `g-board-lane-*` colour: grey backlog, orange owner decision, blue preparation, green work and muted green completion. Lane surfaces mix 6% of that colour with `g-surface`; card surfaces mix 5% with `g-board-card-bg`. Folded and empty lanes retain their identity, and drag copies use the task's existing status-to-lane mapping. Cancelled tasks stay neutral outside the workflow.

Epic identity is independent: its stable four-tone palette remains on the compact `EpicChip` link and `EpicIcon`, without a competing card stripe. Task status is carried by the lane; comment counters stay hidden. Board-only `g-board-card-bg` and `g-board-label-bg` keep surfaces and filled labels quiet. Human circles and agent rounded squares reuse `Avatar` unchanged; role colours are not changed. A ready card with no team yet shows three skeleton tiles (`g-skeleton`, the small avatar size) where its team's avatars will appear, with the wait explained in its accessible description; the shimmer stops under reduced motion.

## Typography

Unbounded for display, Golos Text for everything else, Geist Mono for identifiers, code and numeric metadata. The display and body faces are self-hosted variable fonts (`@fontsource-variable`); the mono face is self-hosted too, so the app renders identically offline.

| Style | Size / weight / line-height | Token |
| --- | --- | --- |
| Display XL | `clamp(32px, 8vw, 96px)` / 900 / 0.95 | `g-text-display-xl` |
| Display | `clamp(28px, 5.5vw, 64px)` / 900 / 0.95 | `g-text-display` |
| H1 | `clamp(26px, 4vw, 44px)` / 700 / 1.12 | `g-text-h1` |
| H2 | `clamp(24px, 3vw, 30px)` / 700 / 1.12 | `g-text-h2` |
| H3 | 22 / 600 / 1.12 | `g-text-h3` |
| H4 | 17 / 600 / 1.45 | `g-text-h4` |
| Body large | 17 / 400 / 1.65 | `g-text-body-lg` |
| Body | 15 / 400 / 1.65 | `g-text-body` |
| UI | 14 / 500 / 1.45 | `g-text-ui` |
| Dense UI | 13 / 500 / 1.45 | `g-text-dense` |
| Caption | 12 / 500 / 1.45 | `g-text-caption` |
| Eyebrow | 11 / 600 / 1.45, uppercase, tracking 0.14em | `g-text-eyebrow` |
| IDs and code | Geist Mono 12-13 / 400 | `g-font-mono` |

Unbounded never goes below 24 px: it is very wide and loses Cyrillic legibility when small. Anything smaller is Golos Text. Turn on `font-feature-settings: "tnum"` wherever digits align in columns.

## Component inventory

The reference implementation is `components.css`; render each row from the kit wrapper in `web/src/shared/ui/kit/` and keep the states below.

| Family | Variants and states | Code reference |
| --- | --- | --- |
| Button | primary, secondary, ghost, danger × roomy/dense × resting/hover/focus/disabled/busy | `Button`; pill, 40 px (32 dense). Primary is orange with an ink label; secondary is a green outline. |
| Icon button | resting/hover/focus/disabled | `IconButton`; needs a `label` for its accessible name. |
| Tabs | pills/underline × selected/resting | `Tabs`; horizontal `g-tabs`, also usable as route links. Use `variant="underline"` for subordinate navigation. |
| Segmented | selected/resting | `Segmented`; `g-seg`, radiogroup semantics. |
| Chip | resting/pressed | `Chip`; `pressed` reflects the toggle state. |
| Tag | neutral, accent, signal, danger, solid-accent, solid-signal, outline | `Tag`; soft tint with the matching text colour. |
| Field | input/select/textarea/search × resting/focus/disabled × roomy/dense, with label, help, error | `Field`, `Input`, `Select`, `Textarea`, `SearchField`; 40 px field, 12 px radius, `g-control-border`. |
| Choice | checkbox, radio, switch | `Checkbox`, `Radio`, `Switch`; visible label, native input underneath. |
| Dialog | standard/wide, busy | `Dialog`; focus trap, `Escape`, backdrop click, restored focus. |
| Confirmation | normal/destructive/busy | `ConfirmDialog`; composes Dialog and Button. |
| Menu | closed/open, item resting/hover/disabled/checked | `ActionMenu`; roving focus, `Escape`, outside click, `separator` entries. |
| Navigation | resting/hover/selected, full/compact | `g-nav-*` and `g-tabs`; counts and status are optional slots. |
| Breadcrumbs, key caps, tooltip | — | `Breadcrumbs`, `Kbd`, `Tooltip`. |
| Task status and priority | 11 statuses, priorities P0-P4 | `StatusIcon`, `PriorityIcon`; shape carries the state too. |
| Avatar | `agent`, `human` × roomy/dense | `Avatar`; shape encodes the kind of actor. |
| Content | card, sheet, stat, progress, segments, group heading, prose | `Card`, `Sheet`, `Stat`, `Progress`, `Segments`, `GroupHead`, `Prose`. |
| Disclosure card | expanded/collapsed, hover/focus; persisted by caller-supplied key | `DisclosureCard`; compact rail surface, full-heading keyboard toggle and hidden body. |
| Feedback | neutral/accent/signal/danger banner, empty, error, skeleton, working arcs | `Banner`, `EmptyState`, `ErrorState`, `Skeleton`, `WorkingArcs`. An empty list uses `EmptyState`; a failed query uses `ErrorState`. |
| Image preview | thumbnail/inline/full/lightbox | `ImagePreview`; shared surfaces and radii, paired danger foreground for close hover. |
| Team graph | process/configuration, inline/expanded | `TemplateGraph`; configuration uses `compact` to wrap stages at readable size, with fit/zoom/expand controls. |
| Charts | grouped/stacked | `BarChart`; `g-viz-*` series colours, visible legend and textual totals. |
| Rich text | reading/editing | `Markdown`, docs editor; content-specific block geometry stays local. |

## Audit decisions and intentional exceptions

Resolved drift: one token source; the legacy `--bg`/`--text`/`--accent` palette, its compatibility aliases, the accent choices and the `Geist` body font are gone; board tints, status icons and progress share the semantic colours; charts use theme-aware series; filled foregrounds come in pairs; disabled filled buttons do not brighten on hover.

Literal values still allowed outside the token layer:

- The `.login-*` illustration and its SVG gradients are a deliberately fixed, dark, art-directed scene. Their geometry, decorative colours and motion are not workspace semantic tokens; the form text is scoped to that dark palette.
- Theme-preview swatches in Settings draw from the fixed neutral ramp (`g-n-*`), because each swatch must show the theme it names rather than the theme in use.
- Telegram's QR code stays dark-on-white for scanning, and avatar JPEG compositing uses a fixed neutral background so a saved image does not depend on the viewer's theme.
- Browser meta theme-color has fixed early-boot values for both themes, independent of loaded CSS.
- The inverted paper sheet's two hairlines in `components.css` blend with the sheet underneath them and cannot follow the theme; `test/accessibility.test.ts` records them as a budget.
- SVG icon geometry, graph coordinates, responsive breakpoints, proportional dimensions, layout widths, uncommon spacing and component-specific radii are local geometry rather than conflicting semantic values.

`--g-measure: 64ch` stays documented rather than exported as an inaccurate fixed dimension: `ch` depends on font metrics.
