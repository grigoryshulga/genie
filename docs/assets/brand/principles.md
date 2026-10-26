# Genie design principles

These are the rules behind the [design system](design-system.md). The system says what the pieces are; this page says how to decide when the pieces do not answer the question. Read it before designing or reviewing any screen. Every rule here is checkable in a pull request; the last section is the checklist.

## What the interface is for

Genie is where a person supervises a team of agents that does real work. The person is not the one typing the code; they decide, unblock, approve, and learn. So the interface is calm when things go well, unmistakable when they need a person, and always honest about what is happening.

## The twelve principles

**1. The owner's move comes first.** The most important thing on any screen is what is waiting for the person. It gets the strongest treatment on the page (orange, first position, a verb on the button). Everything else is quieter. If two things compete to be first, the design is wrong, not the order.

**2. Show state, not chrome.** Prefer the live fact ("3 decisions are waiting", "agent is running step 3 of 5") to decoration or empty headings. Headlines, counts, and status icons carry information. A pixel that tells nothing earns its place by structure or it goes.

**3. Calm by default, loud by exception.** Orange is a budget: at most one primary button and one signal region per view. Red is for failure only. Green is the resting accent. When everything shouts, nothing is heard. If a screen needs a second orange element, the first one is probably not the real priority. Indicators that point at the same "your move" (the waiting-count badge in the navigation, an orange dot on a card in the signal lane) do not count against the budget, because they are the same message, not a second one.

**4. Flat, massive, geometric.** Solid fields, large radii, pill controls, no gradients, one overlay shadow. Depth comes from a surface step and a 1px border. Shapes are circles, rounded squares, and quarter arcs; avoid new shape families.

**5. Type does the hierarchy.** Unbounded speaks (display, h1, h2; never below 24px). Golos Text works (everything else). Hierarchy comes from size and weight steps in the scale, not from colour or boxes. One Unbounded heading per view is usually enough.

**6. Two themes, one design.** Ink and paper are equals. Nothing is designed in one and patched into the other. If a component needs a hard-coded colour to work in a theme, the token set is missing something; add the token.

**7. Density is a mode, not an accident.** Work surfaces (lists, boards, tables, settings) use the dense step: 32px controls, 44px rows, 13px text. Reading and deciding surfaces (documents, task description, the answer page) use the comfortable step: 40px controls, 15-17px text. Do not mix steps inside one surface. See the density table below.

**8. Design every state.** Each data view has loading, empty, error, offline, forbidden (the viewer role), and long-content states, designed before the happy path is polished. Loading is a skeleton or the working arcs, never a bare "Loading...". Empty explains what to do next and offers the action. Errors say what failed and what the person can do.

**9. Keyboard and assistive technology are first-class.** Every existing shortcut keeps working. Every control has a visible focus ring (2px, offset, never removed), a label, and a semantic element (`button`, `a`, `input`), not a clickable `div`. Colour is never the only carrier of meaning: status has a shape, errors have text.

**10. Motion explains, never decorates.** Motion shows cause and effect: a card moved, a thing appeared, an agent is working. 150ms for state changes, up to 300ms for entrances, ease-out. Nothing loops except working indicators and the character. Under `prefers-reduced-motion` everything is static and still readable.

**11. The character earns its place.** The character appears where a human moment exists: empty states, onboarding, the status panel, a task returned, a long wait. It never appears in dense lists, toolbars, forms, or error details. One character per view. It reports a true state; it never fakes activity.

**12. Honest status, no false certainty.** The UI shows what the system knows. "CI running" is not "CI passed"; "approved" is not "merged". Do not show optimistic success before the server confirms. Keep the exact status visible even when the layout groups statuses.

## Layout

- **Shell.** A floating pill (primary navigation with an icon on every entry, search, create, notifications, profile) centred over a content area, with the project as a round button of its own beside it. On a phone the pill drops to the bottom edge as icons with small labels, and the rest sits in a sheet behind «Ещё». Detail opens beside or over the content, never replaces the list silently. The page title is the headline of the state, not a label.
- **Width.** Work surfaces use the full width with 32px page padding (16px on phones). Reading surfaces cap at 70 characters. Forms cap at 640px.
- **Rhythm.** 4px base. Related items sit 8-12px apart; groups sit 24-32px apart; sections sit 48-64px apart. If two gaps look similar they should be equal.
- **Alignment.** Left-align text and forms. Right-align numbers in tables. Centre only empty states and hero moments.
- **Regions.** At most three columns at once (navigation or list, main, detail). The orange region, when present, is the first column or the top band.

### Breakpoints

| Name | Width | Behaviour |
|---|---|---|
| Phone | up to 640px | One column; navigation collapses to a bottom-sheet menu; detail is a full-screen page; tables become stacked rows |
| Tablet | up to 900px | One or two columns; detail overlays; the board scrolls horizontally with snap |
| Laptop | up to 1100px | Detail replaces the right-most optional column |
| Wide | 1440px and up | Content centres or spreads; reading surfaces stay at 70ch |

### Density

| Surface | Control height | Row height | Body text |
|---|---|---|---|
| Lists, boards, tables, filters, settings tables | 32px (`--g-control-sm`) | 44px (`--g-row`) | 13px |
| Forms, dialogs, detail panels | 40px (`--g-control`) | auto | 14px |
| Documents, task description, answer page | 40px | auto | 15-17px |

## Navigation

- **Primary** (always visible): Board, Tasks, Epics, Docs, Agents. Five or fewer; the selected one is the only filled pill.
- **Secondary** live under a "More" menu or the profile menu: Proposals, Automations, Notifications history, Management, Settings, Statistics.
- **Global actions** sit at the right of the top bar in this order: search (opens the command palette), create, notifications, theme, profile. Create is a secondary button (see principle 3).
- **Project switcher** is the round project icon floating left of the bar (no name; the name is its accessible label): click it to switch.
- **Live work** ("working now") is a compact status dock fed by the character, not a sidebar list; it opens the team views.
- **Settings is a mode.** Entering Settings or Management swaps the top-bar navigation for a section rail and a "back to work" action, as today.
- Keep every existing URL and keyboard shortcut working.

## Voice and content

- **Language.** User-facing text is Russian, as today. Use the established terms and the [glossary](../../../GLOSSARY.md). Do not rename domain words in the redesign.
- **Tone.** Direct, short, active. Buttons are verbs ("Accept", "Reply", "Create task"). Statuses are nouns or short states. No exclamation marks, except the surprised character's line.
- **Case.** Sentence case everywhere, including buttons and headings.
- **Slogans.** The orange full stop ends a slogan or headline of at most six words. It appears on hero headlines and the login page, not on ordinary headings or buttons.
- **Numbers.** Use tabular figures wherever digits align. Format dates relative for recent ("2 minutes ago"), absolute with a tooltip for older.
- **Errors.** Say what failed, why if known, and what to do. Never show a raw stack or status code as the only message.
- **No emoji in the interface.** Use the icon set. Emoji render differently per platform and carry unintended colour.

## Iconography

- 16px grid, 1.6px stroke, round caps and joins, `currentColor`. Filled shapes only for state (approved, needs owner, done).
- Statuses differ by shape as well as colour; see the status table in [components.html](components.html). New statuses need a new shape.
- Icons never replace a label on a primary or destructive action. Icon-only buttons need an accessible name and a tooltip.
- The arcs motif is decoration and a symbol for "epic" and "agent working"; do not use it for anything else.

## Data visualisation

- At most four series, using `--g-viz-1` to `--g-viz-4`, direct-labelled at the data. More series: small multiples.
- Grid lines use `--g-viz-grid`; axes and labels use `--g-text-muted`. Bars have 6px radius and a minimum 3:1 contrast against the surface.
- Show the number. A bar without its value is decoration.
- Never use red and green as the only difference; add a label or a shape.

## The character

| State | When | Never |
|---|---|---|
| Thinking | An agent is planning or running | Without a real running agent |
| Sleeping | The project is idle | While anything is running |
| Waking | A task is picked up or a session resumes | As a loading spinner |
| Surprised | A task was returned, CI failed, or something unexpected needs the owner | For routine events |

Size: 72-140px in panels, 160-240px in empty states. One per view. Static pose under reduced motion. Never overlapping text or controls.

## Accessibility baseline

- Text contrast 4.5:1, large text and graphics 3:1, in both themes. `test/design-tokens.test.ts` enforces the token pairs; new colour tokens add their pairs to it.
- Touch targets are at least 32px (24px minimum for dense desktop rows with spacing); phone controls are 40px.
- Focus is always visible and never trapped except in modal dialogs, which restore focus on close.
- Dialogs, menus, and tabs follow their WAI-ARIA patterns; `role` and `aria-*` ship with the component, not per screen.
- Live regions announce toasts and agent state changes politely.

## Anti-patterns

- A second orange button or a second orange region.
- Unbounded below 24px, or Unbounded for body text.
- Hard-coded hex values or font stacks outside `g-tokens.css`.
- Shadows on in-flow surfaces; gradients; blur.
- A clickable `div`; a link styled as a button or a button that navigates.
- A loading state that is only the word "Loading".
- A new status or colour without a shape and a token.
- Light text on the brand green below 24px; brand orange as text on paper.
- The character in a list, a toolbar, or an error.

## Pull request checklist

1. Both themes checked at 1440px and 390px, screenshots attached.
2. No hard-coded colours, sizes, or font families; tokens and `g-` components only.
3. At most one orange primary action and one signal region.
4. Loading, empty, error, and forbidden states exist and were seen.
5. Keyboard: tab order, focus ring, existing shortcuts, escape closes overlays.
6. Reduced motion: nothing loops except working indicators, and those are static.
7. Contrast tests pass; new colours added their pairs.
8. Terminology matches the existing UI and the glossary.
9. Behaviour is unchanged: URLs, query parameters, shortcuts, drag and drop, live updates.
