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
