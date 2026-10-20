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

The board's lockup (a green stroked "g" with an orange arc, plus a lowercase wordmark) is a draft vector rebuilt from the board raster, because no designer source exists in the repository:

- [`logo-ink.svg`](logo-ink.svg): for dark surfaces. Green mark, orange arc, paper wordmark.
- [`logo-paper.svg`](logo-paper.svg): for light surfaces. The mark steps down to the deep green (`--g-green-800`) and the wordmark is ink, for the same contrast reason as text.
- [`logo-mark.svg`](logo-mark.svg): the mark alone, for avatars and favicons. Prefer 24px or larger.

How they were made: the mark is clean geometry (a stroked circle, handle, stadium, and loop) overlaid on the board and corrected until it matched; the wordmark is a bitmap trace of the board lettering, so its curves and small corners are approximate. The files use no fonts or external resources. They are working drafts: replace them with the designer's originals when available, and do not treat the wordmark's exact letterforms as final.

They do not replace the current lettermark in [the brand README](README.md) yet; the docs navbar, README hero, and app UI still use the old mark until the migration.

Clear space is at least the height of the arc on every side.

## Character animations

The logo mark doubles as a character. The orange arc is its eye (open "∩", closed line, sleeping "∪", full ring when startled) and the handle is an antenna that reacts on its own timing. Four looping actions ship as standalone SVGs with embedded CSS and no scripts or fonts, so they work in `<img>`, inline, or as background images:

| File | Action | What happens |
|---|---|---|
| [`genie-surprise.svg`](character/genie-surprise.svg) | Surprise | Crouch, jump, eye becomes a full ring, antenna flicks, spark lines burst, then a tremble |
| [`genie-wake.svg`](character/genie-wake.svg) | Waking up | Asleep with a drooping antenna and "z" marks, stirs, stretches, opens wide, blinks, drifts off again to loop |
| [`genie-sleep.svg`](character/genie-sleep.svg) | Falling asleep | Heavy blinks, head and antenna droop, eye closes to a thin curve, soft breathing and "z" marks, startles awake to loop |
| [`genie-think.svg`](character/genie-think.svg) | Thinking | Gaze moves up and to the side then across, squint and blink, a thought trail ends in a "..." bubble |

Open [`character/animations.html`](character/animations.html) from a checkout to see all four with replay, pause, and a theme toggle. The files are generated: edit [`character/build.mjs`](character/build.mjs) (keyframes are tables per body part) and run `node docs/assets/brand/character/build.mjs`; do not edit the SVGs by hand.

Rules:

- Every action returns to the neutral logo pose, so a loop can be dropped in place of the static mark without a jump.
- Under `prefers-reduced-motion: reduce` the SVGs show only the neutral pose. Extras that exist only during an action (sparks, "z" marks, thought trail) are hidden by default.
- The SVGs default to the ink-theme colours. On paper the mark uses `--g-green-800` and the "z" marks use ink; the preview page shows this through two CSS variables, `--g` and `--p`.
- The characters are built from the draft mark, so they inherit its approximations (see Logo) and should be regenerated when the final vector arrives.

Suggested product mapping, not yet decided: thinking for an agent that is planning or running, sleeping for an idle project, waking for resuming work or a new assignment, surprise for something that needs the owner's attention. Keep the character out of dense lists and toolbars; it belongs in empty states, onboarding, loading screens, and status panels.

## Concept: task board

[`concepts/board.html`](concepts/board.html) applies the system to the task board with sample data, deliberately without reusing the current layout. It is a concept for discussion, not product code. Open it from a checkout; the theme button switches ink and paper, and the demo control under the headline plays each character state.

What it tries:

- **The headline is the project's state.** An Unbounded slogan ("3 decisions are waiting for you") with the orange full stop, over the arcs motif, replaces a plain page title.
- **Orange means "your move".** The owner-decision lane is pulled to the front and filled orange, with answer buttons on its cards. The create-task button is secondary on purpose, so orange stays reserved for the user's turn.
- **Three stages instead of eight columns.** Preparation (inbox, draft, refining, ready), work (in progress, changes requested), and review (review, approved) are lanes; each card keeps its exact status as a tag. Done is a quiet pill strip. This is a proposal to confirm, since dragging between lanes would then have to pick a default status.
- **Cards are paper sheets on the ink canvas** (ink sheets on paper in the light theme). Acceptance criteria are a segmented bar, agents are rounded squares and people are circles, and concentric arcs pulse on a card while an agent is working on it.
- **The character reports status.** Thinking while an agent works, sleeping when idle, waking when a task is picked up, surprised when a task is sent back.
- **Navigation is a top pill rail**, not a sidebar.

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
