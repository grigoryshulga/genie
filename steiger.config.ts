import fsd from "@feature-sliced/steiger-plugin";
import { defineConfig } from "steiger";

// One rule on purpose (PD21, S9): the settings pages and the work pages must
// not import each other, and lower layers must not import higher ones. The
// other FSD rules are off — the codebase predates them.
export default defineConfig([
  ...fsd.configs.recommended,
  {
    rules: {
      "fsd/forbidden-imports": ["error", { allowSingularSegments: false }],
      "fsd/ambiguous-slice-names": "off",
      "fsd/excessive-slicing": "off",
      "fsd/import-locality": "off",
      "fsd/inconsistent-naming": "off",
      "fsd/insignificant-slice": "off",
      "fsd/no-cross-imports": "off",
      "fsd/no-higher-level-imports": "off",
      "fsd/no-layer-public-api": "off",
      "fsd/no-processes": "off",
      "fsd/no-public-api-sidestep": "off",
      "fsd/no-reserved-folder-names": "off",
      "fsd/no-segmentless-slices": "off",
      "fsd/no-segments-on-sliced-layers": "off",
      "fsd/no-ui-in-app": "off",
      "fsd/no-wildcard-exports": "off",
      "fsd/public-api": "off",
      "fsd/repetitive-naming": "off",
      "fsd/segments-by-purpose": "off",
      "fsd/shared-lib-grouping": "off",
      "fsd/typo-in-layer-name": "off",
    },
  },
  // The entities cross-import pure model types deeply (the node tests load the
  // files directly), and create-task uses shape-idea's hook — both predate this
  // rule. The boundary this phase guards is the pages and the widgets.
  {
    files: ["web/src/entities/**", "web/src/features/**"],
    rules: {
      "fsd/forbidden-imports": "off",
    },
  },
]);
