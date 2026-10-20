# Genie lettermark

[genie-mark.svg](genie-mark.svg) is the standalone vector version of the selected
smoke-shaped lowercase `g` concept (GNE-13). The lower loop stays open; there is
no lamp in the mark.

The SVG has a transparent background and one compound path with an even-odd
cutout. It contains no raster images, external resources, or font dependencies.
Its default fill is the UI's periwinkle `#9ca3f5`, intended for dark backgrounds.
For light backgrounds, change the path fill to indigo `#5e6ad2`. For inline SVG,
the path fill can be set to `currentColor` to inherit the surrounding text color.

Preserve the viewBox aspect ratio and leave space around the mark. The file
includes an outer margin. It was visually checked at 16, 24, 26, 32, 40, and
64 pixels, plus a large rendering on dark and light backgrounds. At 16 pixels,
the tapered smoke tip is naturally less distinct; prefer 24 pixels or larger
when space permits.

The same contour is embedded in `docs/public/logo.svg` for the documentation
navbar and favicon, and in `docs/assets/hero.svg` for the GitHub README hero.
When editing the lettermark, update those embedded paths as well. The application
UI logo is unchanged.

The broader visual language (colour, type, components, motif) is a draft in
[design-system.md](design-system.md); it will supersede the periwinkle and indigo
values above once the lettermark is redrawn for it.
