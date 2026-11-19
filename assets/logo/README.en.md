# Olvin logo: what to use where

A description of every file, which style to pick for which case, and where to place it. The rules for building the mark (geometry, colours, sizes) are in [STYLE.md](../../STYLE.en.md), section 9. This file is the practical side: what to take in a given situation.

Gallery of all variants on their intended backgrounds: [variations/index.html](variations/index.html).

<p align="center"><a href="README.md">Русский</a></p>

## The idea in two lines

The mark is "O▌": a ring and a green cursor. In the `Olvin` wordmark the green letter "l" is the cursor. Green is used in the logo for that one element only.

## How to pick a file in three steps

1. **Background.** Dark background: the `on-dark`, `white`, `dark` files. Light background: `on-light`, `black`, `light`. Unknown or coloured background: take a tile (`plate-*`, `circle-*`) or mono.
2. **Size.** Under 24 px: only flat files (`flat`, `plate`, `circle`), and only the mark, without the word. From 24 to 64 px: the mark. The word from 120 px wide.
3. **Style of the surroundings.** Flat interfaces and documents: `flat`. Glass panels, splash screens, promo on dark: `glass`. Decorative places and watermarks: `line`. One colour (print, stamp, terminal): `mono`.

## Styles: what each one is for

| Style | What it is | Good for | Not for |
|---|---|---|---|
| **flat** | A white or dark O and a green cursor. The main variant. | Editor and hub interfaces, documentation, README, websites, anything small | It is the default working variant |
| **mono** | One colour, no green. | One-colour print, stamps, embroidery, engraving, black-and-white documents, backgrounds with a coloured banner, console output | Anywhere green can be shown: without it the logo loses recognisability |
| **line** | A thin outline instead of a fill. | Page backgrounds, watermarks, title screens, wallpapers, large sizes from 200 px | Under 64 px the lines disappear. Not as the main logo. |
| **glass** | Translucent shapes with a highlight and shadow, Liquid Glass style. | The editor splash screen, the hub launcher, covers, presentations, promo on dark, the "About" window | Small icons, a light background behind dark glass, print documents |
| **plate** (tile) | The mark on a rounded square. | App icon, `.exe`, desktop and taskbar icons, project card | Inside an interface that is already styled: the tile becomes "a card in a card" |
| **circle** | The mark on a circle. | GitHub and social avatars, channel badges, chat icons | App icons: round icons break the grid on macOS and Windows |
| **glass-icon / glass-circle** | A tile or circle with a coloured background and a glass mark. | Showcase: an itch.io page, the NLPA site cover, Telegram previews | System icons: SVG filters may not be supported |

## All files

### Main (in the root folder)

| File | Description | Where |
|---|---|---|
| `olvin-mark.svg` | The mark: white O and green cursor, transparent background | Dark background. Default |
| `olvin-mark-light.svg` | The mark: dark O and dark green cursor (`#1F9D3A`) | Light background. The green is darker so it reads on white |
| `olvin-mark-animated.svg` | The mark with a blinking cursor (1 second) | Splash screen, launcher, loading page. Not for icons or documents |
| `olvin-wordmark.svg` | The word `Olvin`, green "l", white other letters | Website header and README on dark, the "About" window |
| `olvin-icon.svg` | The mark on a dark tile with the NLPA site gradient | Default app icon |
| `olvin-icon.ico` | Windows icon (256, 64, 32, 16 px) | `.exe`, shortcuts, Windows windows |
| `png/olvin-icon-{512,256,128,64,32,16}.png` | The same icons as PNG | Linux, previews, raster contexts |

### `variations/`: without a backing

| File | Background | Style | When |
|---|---|---|---|
| `mark-flat-on-dark.svg` | Dark | flat | The mark in an interface, on dark pages |
| `mark-flat-on-light.svg` | Light | flat | The mark on white: documents, the light theme of a website |
| `mark-mono-white.svg` | Dark or coloured | mono | White mark over a photo, a banner, one-colour print |
| `mark-mono-black.svg` | Light | mono | Black mark: stamps, black-and-white print, embroidery |
| `mark-line-white.svg` | Dark | line | Watermark, decoration on large screens |
| `mark-line-black.svg` | Light | line | The same on light |
| `mark-glass-on-dark.svg` | Dark | glass | Splash, launcher, promo. White glass with a glowing green cursor |
| `mark-glass-on-light.svg` | Light | glass | Light presentation or page. Smoky dark glass |
| `wordmark-flat-on-dark.svg` | Dark | flat | Header, README, title pages |
| `wordmark-flat-on-light.svg` | Light | flat | Documents and light pages |
| `wordmark-mono-white.svg` | Dark or coloured | mono | The word over an image |
| `wordmark-mono-black.svg` | Light | mono | Print, black and white |
| `wordmark-line-white.svg` | Dark | line | Decoration, large sizes |
| `wordmark-line-black.svg` | Light | line | The same on light |
| `wordmark-glass-on-dark.svg` | Dark | glass | Splash and promo |
| `wordmark-glass-on-light.svg` | Light | glass | Light promo |

### `variations/`: with a backing

| File | Shape | Tile background | When |
|---|---|---|---|
| `plate-dark.svg` | Tile | Dark gradient | App icon, editor, hub |
| `plate-light.svg` | Tile | Light | Light theme, light panels |
| `plate-green.svg` | Tile | Green, black mark | Accent places: a "Download" button, banners. Rarely: it is a loud variant |
| `circle-dark.svg` | Circle | Dark gradient | GitHub, Telegram, channel avatar |
| `circle-light.svg` | Circle | Light | Avatar on a light background |
| `circle-green.svg` | Circle | Green | Accent avatar, sticker, event badge |
| `glass-icon-dark.svg` | Tile | Dark glass, green-turquoise spots | Showcase (itch.io, NLPA site), splash |
| `glass-icon-light.svg` | Tile | Light glass, green and blue spots | Light showcase, presentations |
| `glass-circle-dark.svg` | Circle | Dark glass | Avatar for promo, showcase |
| `glass-circle-light.svg` | Circle | Light glass | The same on light |

## Where to put what in a project

| Place | What to take | Why |
|---|---|---|
| Editor and hub window, taskbar icon | `olvin-icon.svg` (PNG 256/64/32/16, `olvin-icon.ico` for Windows) | The tile reads at any size and matches the theme |
| Editor and hub header | `mark-flat-on-dark.svg` at 24–32 px | There is no room for the word in a header. The mark is concise |
| Editor and hub splash screen | `olvin-mark-animated.svg` + `wordmark-glass-on-dark.svg` or `wordmark-flat-on-dark.svg` | The cursor blinks like the NLPA site logo. Glass supports the feel of the panels |
| "About" window | `wordmark-flat-on-dark.svg` and the caption `Olvin · by NLPA Collab` | The word is large, without effects: the window is a working one |
| Empty viewport when there is no project | `mark-line-white.svg`, large, with low opacity | Decoration without noise |
| README in the repository root | `wordmark-flat-on-dark.svg` centred (for GitHub's dark theme) and `wordmark-flat-on-light.svg` for the light one | GitHub shows different themes: use `<picture>` with `prefers-color-scheme` |
| GitHub avatar (social preview of `DanyaDel/Olvin`) | `circle-dark.svg` for the avatar, `glass-icon-dark.svg` for the social preview | A round avatar, dark gradient as on the site |
| Olvin page on the NLPA site | `glass-icon-dark.svg` as the project cover, `wordmark-flat-on-dark.svg` in the page header | A glass style on black: the variant fits the feel |
| itch.io (studio page) | `glass-icon-dark.svg` for the preview, `circle-dark.svg` for the avatar | Showcases like a nice picture |
| Telegram channel and chats | `circle-dark.svg` or `circle-green.svg` | Chats crop to a circle |
| Documentation (mdbook, pages) | `wordmark-flat-on-light.svg` and `wordmark-flat-on-dark.svg` via the theme | Documentation switches between light and dark themes |
| Console output of `olvin-hub` | A text version: `Olvin_` with a green underscore via ANSI | Pictures are not needed in a terminal, the cursor carries over as text |
| Installer, `.exe`, shortcut | `olvin-icon.ico` | Windows takes icons only from ICO |
| Presentations, posters | `glass-icon-*` or `wordmark-glass-*` on dark | Maximum impact, where the size is large |
| Print, stamps, embroidery, merchandise | `mark-mono-black.svg` or `mark-mono-white.svg` | One colour, no gradients |
| Watermark on screenshots and video | `mark-mono-white.svg` at 30–50% opacity | It doesn't cover the frame |
| Title of large screens (teaser, wallpapers) | `mark-line-white.svg` or `wordmark-line-white.svg` | The outline works well at large sizes |

## What not to do

- Don't change the colours or proportions by hand. If you need a new variant, add a file to `variations/`.
- Don't stretch the mark along one axis or rotate it.
- Don't put the mark on a busy background without a tile or a mono variant. A green cursor disappears on a green image.
- Don't use glass variants under 64 px: the highlights turn into noise.
- Don't use outline variants under 64 px.
- Don't put a vertical bar inside the ring: that is the power symbol.
- Don't add a second green element next to the logo: in the logo, only the cursor is green.
- Don't show the mark and the word together when the word already contains the O: the wordmark includes the mark itself.

## Clear space and minimum sizes

- Clear space: at least the thickness of the ring (`W`, 12 units out of the mark's 68-unit height, about 18% of the height) on all sides.
- Mark: minimum 16 px. Word: minimum 96 px wide.
- Icons: from 16 px. Glass and outline: from 64 px.

## Integration status

Connected:

- The window and taskbar icon of the hub (`hub/olvin_hub_gui/src/main.rs`) and the editor (`editor/olvin_editor/src/lib.rs`): `png/olvin-icon-256.png` is embedded with `include_bytes!` and passed to `ViewportBuilder::with_icon`. If you change the icon, re-render the PNGs from `olvin-icon.svg` (`resvg -w N -h N`), otherwise the window keeps the old one.
- The `olvin-hub` console logo: `Olvin_` with a green underscore in the help header (`olvin-hub --help` and running without arguments). No colour with `NO_COLOR` or when output is not a terminal.
- The `README.md` header in the root: `<picture>` with `wordmark-flat-on-dark.svg` and `wordmark-flat-on-light.svg`.
- The logo in the interface is drawn by code from the `olvin-wordmark.svg` geometry, no raster: `crates/olvin_ui/src/logo.rs` (`wordmark`, `mark`, `wordmark_intro` for the splash). Colours come from the theme: the ring `logo_ring`, the cursor `accent` (`#1F9D3A` in light). The editor header shows the 16 px wordmark with a blinking cursor.

Not done yet:

- The splash screen with the animated mark (`olvin-mark-animated.svg`).
- The `.exe` icon on Windows (via `build.rs` and a resource file, `olvin-icon.ico`).

When you connect something, add a note about it to `ARCHITECTURE.md`.
