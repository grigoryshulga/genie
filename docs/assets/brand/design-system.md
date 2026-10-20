# Genie design system

Status: **draft for review.** This is the first pass at turning the "ink and paper" visual language board into rules, tokens, and a specimen. It does not change the application yet.

- Tokens: [`web/src/shared/ui/tokens.css`](../../../web/src/shared/ui/tokens.css) (prefix `--g-`; two themes).
- Specimen: [`design-system.html`](design-system.html). Open it from a checkout; it reads the real tokens and has a theme toggle.
- Accessibility contract: `test/design-tokens.test.ts` fails when a documented text, control, or focus pairing drops below its WCAG ratio in either theme.
- Motif asset: [`arcs.svg`](arcs.svg).

## Principles

1. **Flat and massive.** Solid colour fields, large geometric type, no gradients, no decorative shadows. Depth comes from a surface step plus a 1px border.
2. **Two surfaces, two voices.** Ink (dark) and paper (light) are equals. Every component is specified for both.
3. **Colour carries meaning.** Green is the system's state; orange is the user's move. Neither is decoration.
4. **Big type speaks, small type works.** Unbounded is for display; Manrope does everything the user reads for more than a few words.
5. **One closing full stop.** A slogan ends with an orange full stop. It is the only place orange punctuation appears.

## Colour

| Token | Value | Role |
|---|---|---|
| `--g-green` | `#008F6B` | System state: progress, selection, links, focus, outlines |
| `--g-orange` | `#FF873C` | The user's move: the primary action, "needs your decision", the slogan stop |
| `--g-ink` | `#09090D` | Dark surface; text on paper and on every filled brand colour |
| `--g-paper` | `#F4F1EA` | Light surface; text on ink |

Neutrals are an ink-to-paper ramp (`--g-n-950` to `--g-n-0`), so greys stay warm. Components never use layer 1 values directly; they use the semantic layer (`--g-bg`, `--g-surface`, `--g-text`, `--g-accent`, `--g-signal`, and so on), which each theme maps.

### Contrast rules

Measured with the WCAG relative-luminance formula:

| Pair | Ratio | Rule |
|---|---|---|
| ink on paper | 17.62 | Body text |
| orange on ink | 8.31 | Text and accents on dark |
| ink on orange | 8.31 | Orange button label is always ink |
| ink on green | 4.87 | Filled green label is ink |
| paper on green | 3.62 | Large text only (24px or 18.66px bold) |
| green on paper | 3.62 | Not text. Use `--g-green-700` (4.73) |
| orange on paper | 2.12 | Never text or thin lines. Use `--g-orange-700` (5.32) for text; use orange only as a fill |

Consequences worth knowing: a white label on a green button is not allowed, and the brand green cannot be a link colour on paper. Danger is the one hue outside the palette (`--g-red-500` on ink, `--g-red-700` on paper) because a tracker needs a failure colour.

### Semantic mapping to the tracker

| Meaning | Colour |
|---|---|
| In progress, selected, link, focus | green |
| Waiting on you, primary action, needs a decision | orange |
| Blocked, destructive, error | red |
| Draft, queued, done-and-quiet | neutral |

The current app's `--violet` and `--yellow` have no place in this system and retire during migration.

## Typography

| Role | Family and weight | Size / line / tracking |
|---|---|---|
| Display XL (covers, slogans) | Unbounded 900 | `clamp(32px, 8vw, 96px)` / 0.95 / -0.04em |
| Display | Unbounded 900 | `clamp(28px, 5.5vw, 64px)` / 0.95 / -0.04em |
| H1 | Unbounded 700 | `clamp(26px, 4vw, 44px)` / 1.12 / -0.025em |
| H2 | Unbounded 700 | `clamp(24px, 3vw, 30px)` / 1.12 / -0.025em |
| H3, H4 (subheads) | Manrope 600 | 22px and 17px |
| Button, field, tab | Manrope 600 | 14px (13px dense) |
| Body, documentation | Manrope 400 and 500 | 15px; 17px for long reading; line 1.65 |
| Caption | Manrope 500 | 12px |
| Eyebrow | Manrope 600, uppercase | 11px / 0.14em |
| IDs and code | Geist Mono 400 | 12-13px |

Rules:

- **Unbounded never goes below 24px.** It is very wide; at small sizes it wastes space and loses Cyrillic legibility. Anything smaller is Manrope.
- Unbounded display text uses `text-wrap: balance` and a maximum width in `ch` so slogans break on the line the design intends.
- The minimum sizes in the clamps exist because the longest Russian words, such as the ones in the board's headline, overflow a 390px screen in Unbounded 900 above roughly 32px.
- Turn on tabular figures (`font-feature-settings: "tnum"`) wherever digits align in columns: counts, durations, IDs. Manrope ships `tnum` in its Latin subset.
- Prose line length is at most 70 characters.

## Shape, space, motion

- **Radii:** 8 (small controls), 12 (inputs, list rows), 20 (cards), 28 (hero panels), 999 (buttons, chips, tags). Buttons and chips are always pills.
- **Space:** 4px base: 4, 8, 12, 16, 24, 32, 48, 64, 96.
- **Elevation:** none for in-flow surfaces. Menus, popovers, and dialogs use `--g-shadow-overlay`, the only shadow in the system.
- **Motion:** 150ms, `cubic-bezier(0.2, 0, 0, 1)`, on colour and border changes. Respect `prefers-reduced-motion`.
- **Focus:** a 2px `--g-focus` ring with a 2-3px offset on every interactive element, never removed.

## Components (first set)

- **Button.** Primary is an orange pill with an ink label, one per view. Secondary is a 1.5px green outline pill. Ghost is text on hover fill. Danger is a red pill. Heights 40px, or 32px dense.
- **Chip.** Green outline pill with an icon; the selected state fills green with an ink label. Neutral chips use the strong border.
- **Tag.** Small status pill on a soft tint with the matching text colour (`accent`, `signal`, `danger`, `neutral`). Every tag text colour is contrast-tested against its tint.
- **Field.** 40px, 12px radius, 1.5px `--g-control-border` (3:1 against the surface), a visible label above, focus ring on focus.
- **Card.** `--g-surface`, 1px border, 20px radius, 24px padding.
- **List row.** 44px, 12px radius, hover and selected fills; selected uses `--g-accent-soft`.

## The arcs motif

Concentric quarter rings anchored to a corner of the frame and cropped by it: a green outer band, an orange band inside it, and (on paper) a solid green core. One composition per surface, never behind text or controls, always bleeding off two edges. The canonical asset is [`arcs.svg`](arcs.svg). Its radii (500 / 360, 250 / 140, 105 in a 500 viewBox) are measured from the board's raster and should be replaced by the designer's vector source when it exists.

## Logo

The board shows a new lockup (a green "g" with an orange arc, plus a lowercase wordmark) that differs from the lettermark currently in [the brand README](README.md). No vector source for it is in the repository, so this system does not redraw it. The board implies these rules, which carry over once the vector exists:

- On ink: green mark, paper wordmark, orange arc.
- On paper: the mark steps down to the deep green (`--g-green-800`) and the wordmark is ink, for the same contrast reason as text.
- Clear space of at least the height of the arc on every side.

## Open decisions

1. **Monospace.** The board defines no mono face. Task IDs and code keep Geist Mono for now. Alternatives: JetBrains Mono, or tabular Manrope for IDs only.
2. **Default theme.** The app is dark today. Tokens default to ink and switch with `data-theme="paper"`. Decide whether the product should follow `prefers-color-scheme`, and whether the docs site stays force-dark.
3. **Font delivery.** The app loads fonts from Google Fonts. Genie is self-hosted and often offline, so bundling Unbounded and Manrope (both OFL, variable) with the build is the safer default.
4. **Orange as "needs you".** The app's amber already means "waiting on the owner". Mapping it to brand orange keeps that meaning but makes orange both the brand highlight and an alert colour. The rule that the primary button and the attention state share orange is deliberate; confirm it.
5. **Density.** The product UI is 13px and compact. Pill rows and 40px controls are roomier than today's 32px controls. The dense variants (32px, 13px) are specified for lists and toolbars.

## Migration plan (not started)

1. Import `tokens.css`, load the fonts, and alias the legacy variables in `app/styles.css` to the new semantic tokens.
2. Convert shared UI primitives (button, menu, modal, pill) and settings.
3. Convert pages by area; retire the indigo accent, violet, and yellow.
4. Re-skin the documentation theme and README hero.
