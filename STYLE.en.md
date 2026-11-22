# Olvin: style

The visual and verbal style of Olvin. For people who make the interface, the logo and the materials, and for AI agents who write UI code. Inspiration: the [NLPA Collab](https://nlpacollab.neocities.org/) studio. Olvin looks like part of that family, but does not copy its site.

<p align="center"><a href="STYLE.md">Русский</a></p>

## In short

A dark glass application with a retro terminal. The interface is modern: blurred panels, thin light borders, soft shadows. The text and the cursor refer to a console. One green accent, everything else neutral.

Three rules everything else rests on:

1. **One accent.** The green `#4cd964` is used only for the selected, the active, the cursor and the "working" status. If everything is highlighted green, nothing is highlighted.
2. **Glass only in the framing.** Blur on the header, floating panels and pop-up menus. Never on the viewport, the scene or code text.
3. **Readability over atmosphere.** Minimum text contrast 4.5:1. If it looks good but cannot be read, we redo it.

---

## 1. Colours

All colours are declared as tokens. Never write a hex value directly in code, only through a token.

### Background and surfaces

| Token | Value | Purpose |
|---|---|---|
| `--bg` | `#000000` | Window background and viewport outside the scene |
| `--bg-radial` | `radial-gradient(circle at 50% 0%, #2c2c2e 0%, #000 100%)` | Launcher and splash background (only there, not in the editor) |
| `--surface` | `#0f0f0f` | Editor panels |
| `--surface-raised` | `#1c1c1e` | Pop-up menus, dialogs |
| `--glass` | `rgba(255,255,255,0.03)` | Background of glass panels |
| `--glass-strong` | `rgba(15,15,15,0.6)` | Header and floating panels |
| `--footer` | `#020202` | Bottom status bar |

### Text

| Token | Value | Contrast on `#000` | Purpose |
|---|---|---|---|
| `--text` | `#f5f5f7` | ~19:1 | Main text, headings |
| `--text-2` | `#ccc` | ~13:1 | Descriptions, field labels |
| `--muted` | `#999` | ~7.4:1 | Secondary text, labels |
| `--faint` | `#666` | ~3.7:1 | **Decorative** text only, never for information |
| `--ghost` | `#333` | ~1.7:1 | Separators, not text |

### Accents

| Token | Value | Contrast on `#000` | Purpose |
|---|---|---|---|
| `--accent` | `#4cd964` | ~11:1 | Selected, active, cursor, "ok" status |
| `--accent-glow` | `rgba(76,217,100,0.4)` | — | Glow around the accent |
| `--accent-border` | `rgba(76,217,100,0.3)` | — | Border on hover |
| `--warn` | `#ffb020` | ~11:1 | Warnings (not green, so they are not confused with it) |
| `--error` | `#ff5a5a` | ~6.6:1 | Errors, red in Problems |
| `--info` | `#4da3ff` | ~7.3:1 | Information, hints |

The amber from the logo idea is not used in the interface. The logo may contain it, the interface does not.

### External brands

Used only in link icons, not in the interface: Telegram `#0088cc`, itch.io `#fa5c5c`, GitHub, YouTube.

### Borders

| Token | Value | Purpose |
|---|---|---|
| `--line` | `rgba(255,255,255,0.08)` | Ordinary separators |
| `--line-top` | `rgba(255,255,255,0.15)` | Light top edge of glass panels |
| `--line-soft` | `rgba(255,255,255,0.05)` | Card borders |

---

## 2. Typography

### Fonts

- **Mono** (`--font-mono`): `JetBrains Mono`, fallbacks `Consolas`, `Menlo`, `monospace`. For code, the console, numeric fields, the status line, file paths, IDs.
- **Sans** (`--font-ui`): `Inter`, fallbacks `system-ui`, `sans-serif`. For menus, panel headings, buttons and labels.
- Fonts are loaded from files inside the project (`assets/fonts/`), not from the system. This way the interface looks the same on Windows, macOS and Linux.
- Font licences: `JetBrains Mono` (OFL), `Inter` (OFL). Keep the licence files next to the fonts.

Exception: the launcher and the splash screen may use mono for the heading, like the cursor logo on the NLPA site.

### Sizes

| Token | Value | Use |
|---|---|---|
| `--fs-xs` | 10px, UPPERCASE, `letter-spacing: 0.06em` | Labels, statuses, menu groups |
| `--fs-sm` | 12px | Field labels, hints |
| `--fs-md` | 13px | Main editor text |
| `--fs-lg` | 14px, bold, UPPERCASE | Main menu items |
| `--fs-xl` | 20px, bold | Window titles |
| `--fs-2xl` | 28px, bold | Headings in the launcher and "About" |

In the editor the main text is 13px, not the 15–16px of the site. Panels are dense and people look at them for hours.

### Rules

- Panel and menu headings: UPPERCASE, like the NLPA navigation.
- Text in code and the console: normal case, no UPPERCASE.
- `line-height`: 1.4 for the interface, 1.7 for descriptions in the launcher.
- Text with a shadow only in launcher headings, not in the editor interface. A shadow blurs small text.

---

## 3. Shape and corner radii

| Token | Value | Where |
|---|---|---|
| `--radius-xs` | 4px | Badges, status tags |
| `--radius-sm` | 8px | Buttons, input fields, list items |
| `--radius-md` | 12px | Editor panels, pop-up menus |
| `--radius-lg` | 20px | Dialogs, launcher cards |
| `--radius-pill` | 999px | Tabs, mode switch, developer tags |

Pills (`--radius-pill`) are only for switches, tabs and tags. Editor buttons are rectangular, because a dense interface with pills looks like a website, not a tool.

---

## 4. Glass

Glass is applied to three kinds of surfaces:

- the editor and launcher header;
- floating panels (the inspector over the viewport, the command palette);
- pop-up menus and tooltips.

Base recipe:

```css
background: var(--glass-strong);
backdrop-filter: blur(20px) saturate(180%);
border: 1px solid var(--line);
border-top: 1px solid var(--line-top);
border-radius: var(--radius-lg);
box-shadow: 0 10px 40px rgba(0,0,0,0.6);
```

Rules:

- `blur` no more than 25px. More is expensive on weak GPUs.
- Blur is not applied to the viewport, the scene, input fields with code, or data tables.
- If the GPU cannot handle it (the Compat tier from `ARCHITECTURE.md`), `backdrop-filter` is turned off and the background becomes the opaque `--surface-raised`. The interface stays readable.
- Background opacity: from 0.6 to 0.95, not lower. Below that, text behind the panel stops being readable.

---

## 5. Shadows and depth

- Depth is set by a shadow and a light top edge, not by colour.
- Normal panel: `0 10px 40px rgba(0,0,0,0.6)`.
- Pop-up menu: `0 10px 40px rgba(0,0,0,0.8)`.
- Card on hover in the launcher: `0 25px 60px rgba(0,0,0,0.6)`.
- Glow only on the selected element: `0 0 15px var(--accent-glow)`.

---

## 6. Components

### Buttons

- **Primary.** Background `--accent`, text `#000`, UPPERCASE, `--fs-lg`. One per screen.
- **Secondary.** Background `--glass`, border `--line`, text `--text-2`. Hover: border `--accent-border`.
- **Danger.** Background `--error` with `#000` text. Only for deleting and resetting.
- Press: `transform: scale(0.98)`, no animation longer than 120 ms.

### Tabs and switches

- Active tab: background `--accent` or white `#fff` with `#000` text (like `.menu__link.active` on the site). Pick one and do not change it between panels.
- Inactive: `--muted`, hover → `--text`.

### Input fields

- Background `#000` or `--surface`, border `--line`.
- Focus: border `--accent`, glow `0 0 0 3px var(--accent-glow)`.
- Mono text, `--fs-md`.

### Statuses

- A 6px dot in the status colour with a `--fs-xs` label.
- "Working" `--accent`, "error" `--error`, "waiting" `--muted`.

### Cards (launcher, projects)

- Background `rgba(30,30,30,0.3)`, border `--line-soft`, radius `--radius-lg`.
- Hover: lighter background, lift `translateY(-4px)`, denser shadow.
- Project preview: 16:9, `object-fit: cover`, GIF preview on hover (as on the NLPA site).

### Member tags

- A pill with background `rgba(255,255,255,0.05)`, a 24px round avatar with border `rgba(255,255,255,0.2)`.
- Hover: avatar border `--accent`, glow `0 0 10px var(--accent-glow)`.

### Logo cursor

- A block 8×20px (6×14 in the header), background `--accent`, glow `0 0 8px var(--accent)`.
- Blinks `step-end` over 1 second.
- In the wordmark the cursor is the letter "l", so in the "off" phase it does not disappear but dims to 30% (`olvin_ui::logo`). Otherwise the word reads "O vin".
- Text is typed character by character: 50–150 ms per character, 500 ms start delay. Only in the launcher and the splash screen.

---

## 7. Motion

Two curves, no more are needed:

| Token | Value | Purpose |
|---|---|---|
| `--ease-out` | `cubic-bezier(0.16, 1, 0.3, 1)` | Panels sliding in, appearing |
| `--ease-pop` | `cubic-bezier(0.175, 0.885, 0.32, 1.275)` | Pop-up menus (with a slight overshoot) |
| `--ease-std` | `cubic-bezier(0.25, 0.8, 0.25, 1)` | Hover, colour change |

Durations:

- Hover and colour change: 200–300 ms.
- Panel and menu sliding: 300–400 ms.
- Launcher (cards, slider): up to 500 ms.

Rules for the editor:

- **Do not jump.** Tool panels do not change size or position on hover.
- **Do not animate what changes often.** Dragging objects, typing in the inspector, the timeline: no animation, only instant feedback.
- **Respect the user's setting.** If "reduce motion" is enabled in the system, all transitions are turned off except colour changes.
- Hover lift is not used in the editor. It is only in the launcher.

---

## 8. Iconography

- Monochrome icons, 1.5–2px stroke, no fill, in `--muted` colour, selected ones in `--accent`.
- One library for the whole project, for example `Lucide` (ISC licence). Do not mix several sets.
- Social network icons with brand logos are used only in the launcher and "About".

---

## 9. Logo and brand

The files are in `assets/logo/`. Which file goes where, which style to pick and what not to do: [assets/logo/README.en.md](assets/logo/README.en.md).

**Idea.** The mark is the "O" and a cursor: the first letter of the name and a blinking green cursor, as in the NLPA site logo. The vertical bar is at the same time the "l" of Olvin and the terminal cursor. The engine has just started, everything is ahead.

**Geometry.** The whole system is built on one stroke thickness `W` and one gap `GAP`:
- the thickness of the O ring = the width of the cursor = the stroke thickness of the wordmark's letters = `W` (12 units at a height of 68);
- the gap between the O and the cursor = `GAP` = `W`;
- the cursor height = the diameter of the O = the cap height.

**Files.**
- `olvin-mark.svg`: the mark on a dark background (white O, green cursor).
- `olvin-mark-light.svg`: the mark on a light background (dark O, `#1F9D3A`: on white `#4cd964` gives weak contrast).
- `olvin-mark-animated.svg`: the cursor blinks (1 second, `step-end`). For the launcher and splash screen, not for icons.
- `olvin-wordmark.svg`: `Olvin` made of strokes of one thickness, no font. The letter "l" is green: it is the cursor itself, there is no separate bar at the end, otherwise it would read as a second "l". The letters are drawn with geometry, so they look the same everywhere.
- `olvin-icon.svg`, `png/olvin-icon-{512,256,128,64,32,16}.png`, `olvin-icon.ico`: the app icon, the mark on a dark tile with the NLPA site gradient.
- `variations/`: 26 variants in SVG, gallery `variations/index.html`. File name: `<what>-<style>-<background>`.
  - What: `mark` (the mark), `wordmark` (the word), `plate`/`circle` (the mark on a tile or circle backing), `glass-icon`/`glass-circle` (an icon in the Liquid Glass style).
  - Style: `flat` (flat, with a green cursor), `mono` (one colour, no green), `line` (outline), `glass` (glass).
  - Background: `on-dark` for dark backgrounds, `on-light` for light ones. `mono-white`/`line-white` have a dark background, `mono-black`/`line-black` a light one.
  - Backings: `plate-dark`, `plate-light`, `plate-green` and the same `circle-*`. Circle for avatars, tile for app icons.
  - Liquid Glass without a backing (`*-glass-on-dark`, `*-glass-on-light`) are gradient translucent shapes with a highlight and a shadow, with no background blur: the SVG file has none, it works on any background. `glass-icon-*` and `glass-circle-*` have the coloured background drawn inside the file.

**Rules.**
- The mark and the wordmark always have the green cursor. Green is used only for it, nowhere else in the logo.
- Minimum mark size 16px, icons 16px. Checked: the ring and the cursor are readable.
- Clear space: at least `W` on all sides.
- Do not put a ring with a vertical bar inside: that is the power symbol.
- Caption on the splash screen and in "About": `Olvin · by NLPA Collab`.
- Do not change or redraw the NLPA Collab logo.
- Early sketches (cubes, voxels, a ring with a cube) are in `assets/logo/variant-*.svg` and do not ship with the product.

---

## 10. Voice and copy

### Language policy

| What | Language |
|---|---|
| Editor and hub UI (windows, menus, dialogs, toasts, `olvin-hub` CLI messages) | English. There is no localisation yet: strings live in the code as they are |
| Documentation (`README`, `ARCHITECTURE`, `STYLE`, `ROADMAP`, `docs/`) | Russian is the primary; the English twin sits next to it as `*.en.md` and is updated together with it |
| Code: identifiers, comments, doc comments, log and error messages | English |
| Commit messages | English, `Area: what was done (#number)` |
| Issues and repository discussions | Russian |

The Russian sample phrases in the Russian version of this file illustrate tone; in the interface the same phrases are written in English ("Save scene"). When localisation arrives, string keys will be English and Russian will be the first translated locale.

### Interface

- Short phrases, the verb first: "Save scene", "Open project".
- No exclamation marks in the interface.
- Errors: what happened and what to do. "Could not open the scene: the file is corrupted. Open the backup in `scene.bak`."
- Empty states: one line and one action. "The scene is empty. Add an object (Shift+A)."

Studio voice (launcher, "About", devlogs, release notes):

- Conversational, lowercase is allowed in subheadings.
- Emoticons rarely and purposefully: `:3`, `(⌐■_■)`. No more than one per text.
- Honesty: if something does not work or the project is idle, we say so.
- Signature phrases are allowed, but not in every text.

What we do not do:

- Swearing and irony at the user's expense in the interface.
- Marketing adjectives: "revolutionary", "the best".
- Smileys in error messages.

---

## 11. Accessibility

- Text: at least 4.5:1 (normal) and 3:1 (large, from 18px or 14px bold).
- `--faint` (`#666`) only for decorative text that is duplicated some other way.
- Keyboard focus is always visible: a 2px `--accent` border.
- Colour is not the only carrier of meaning: statuses have text labels, errors have an icon.
- Clickable areas in the launcher at least 32×32px.

---

## 12. What to avoid

- Gradients, except for the launcher and splash background.
- Several accent colours on one screen.
- Shadows on small text.
- Blur on the viewport and in tables.
- Pills in the editor, except for switches.
- System fonts instead of the bundled ones.
- Animations longer than 500 ms.
- Copying the NLPA site one to one: the same palette and feel, but its own interface.

---

## 13. Tokens in code

The token names from this file must match the theme fields in code. The themes live in the shared crate `crates/olvin_ui` (`Palette`, `ThemeKind`), used by the editor and the hub: `--text-2` → `palette.text_2`, `--fs-md` → `FS_MD`, `--radius-sm` → `RADIUS_SM`. Curves and durations from section 7: `olvin_ui::anim` (`ease_out`, `ease_pop`, `ease_std`, `HOVER`, `PANEL`). The logo is drawn by code from the `olvin-wordmark.svg` geometry: `olvin_ui::logo`.

If a value in code differs from this file, this file is correct and the code is brought in line with it.

---

## 14. Themes

Four themes, switched in the settings of the editor and the hub. **All themes share the same tokens** (`--bg`, `--surface`, `--text`, `--accent`, …): the interface takes a colour by token name from the current theme and never writes a hex value or checks which theme is on. A new theme fills in every token.

| Theme | Idea | `--bg` / `--surface` | `--text` / `--muted` | `--accent` |
|---|---|---|---|---|
| **Dark** (default) | The values from sections 1–5, exactly | `#000` / `#0f0f0f` | `#f5f5f7` / `#999` | `#4cd964` |
| **Light** | Almost white panels on a light grey background, dark text | `#eeeef1` / `#fbfbfc` | `#1b1e24` / `#5e626b` | `#1F9D3A` |
| **Graphite** | Dark, but on soft grey instead of black: for long sessions and monitors where black "sinks in" | `#1a1a1c` / `#232326` | `#ededf0` / `#a1a1a8` | `#4cd964` |
| **High Contrast** | Pure black and white, dense borders (`--line` ≈ 0.43), bright green | `#000` / `#050505` | `#fff` / `#d0d0d0` | `#6bff85` |

Rules:

- One accent in each theme. In the light theme it is `#1F9D3A`, as in `olvin-mark-light.svg`: `#4cd964` on white is too weak.
- The text tokens (`--text`, `--text-2`, `--muted`, `--warn`, `--error`, `--info`) give at least 4.5:1 on `--bg`, `--surface`, `--surface-raised` and `--footer` of each theme. The accent, as a state marker: at least 3:1. A test in `olvin_ui` checks this, and a new theme without it is not accepted.
- `--warn`, `--error`, `--info` are darker in the light theme (`#8a5300`, `#b3261e`, `#0b5cbf`), otherwise they are unreadable on white.
- Borders in the light theme are black with transparency (`--line` = `rgba(0,0,0,0.10)`), in the dark themes white.
- The logo takes the ring and the letters from a theme token (`logo_ring`: the `--text` colour), and the cursor from `--accent`.
- The viewport and the scene do not depend on the theme: they have their own colours, and panels over the viewport are always in the dark theme.
- "Reduce motion" is set separately from the theme and applies to any of them.

---

## History

- Version 1, based on an analysis of the NLPA Collab site (`css/style.css`, `css/indexstyle.css`, `js/main.js`).
- Version 2: four themes (section 14), tokens and the logo in the shared crate `olvin_ui`.
