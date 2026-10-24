# Genie logo

[genie-mark.svg](genie-mark.svg) is the standalone vector logo: a line-art genie
(head with a single eye and nose, rounded body, and a curled neck). It replaces
the earlier smoke-shaped lowercase `g` (GNE-13). The file has no raster images,
external resources, or font dependencies, and it has a transparent background.

The strokes use `currentColor`; the root `color` attribute sets the default
`#A4A0F5`, intended for dark backgrounds. For inline SVG, set `color` or
`currentColor` to the surrounding text color. For light backgrounds, use
indigo `#5e6ad2`.

Preserve the viewBox aspect ratio and leave space around the mark. The file
includes an outer margin. The stroke width is 44 units in an 800-unit viewBox,
so the line weight scales with the rendered size: at 16 pixels the lines are
about 0.9 pixels thick, so prefer 24 pixels or larger when space permits.

The same artwork is embedded in several places. When the logo changes, update
all of them:

- `web/public/logo.svg` — browser favicon and the web app. The mark sits on the
  `#1b1d33` tile, stroked `#A4A0F5`.
- `docs/public/logo.svg` — documentation navbar and favicon, same tile.
- `docs/assets/hero.svg` — GitHub README hero, mark inside the 44-pixel tile.
- `web/src/shared/ui/icons.tsx` — `Icon.mark` for the sidebar and public pages,
  stroked with `currentColor`.

## Ink and paper design system

The visual language is specified in [design-system.md](design-system.md), with principles in [principles.md](principles.md), a component specimen in [components.html](components.html), and the redesign record in [handoff/README.md](handoff/README.md). Its tokens and components live in `web/src/shared/ui/g-tokens.css` and `web/src/shared/ui/components.css`, wrapped by `web/src/shared/ui/kit/`; the application has used them since the redesign landed (epic GNE-40). [design-system/README.md](../../../design-system/README.md) is the working reference for tokens, type, and components.

There is one token source: `g-tokens.css`. The earlier GNE-25 palette, its compatibility aliases, the accent choices, and the Geist body font were removed in H33 (GNE-69), and `geist-mono` remains for identifiers only. The draft logo lockups (`logo-*.svg`) are exploratory; the current logo is `genie-mark.svg` until the owner supplies the final vector.

## Where the logo is used (GNE-46)

The ink tile with the green mark and orange eye (`#09090d`, `#2fbf94`, `#ff873c`) is the application icon (`web/public/logo.svg`), the documentation favicon and navbar logo (`docs/public/logo.svg`), and the mark in the README hero (`docs/assets/hero.svg`). All three are drawn from the geometry of [genie-mark.svg](genie-mark.svg), which stays the source until the owner supplies the final vector; update the three files together when it changes. The documentation site uses the ink and paper palette with self-hosted Manrope, Unbounded and Geist Mono (Latin subsets in `docs/public/fonts/`, SIL OFL), mirrored by hand in `docs/.vitepress/theme/style.css`.
