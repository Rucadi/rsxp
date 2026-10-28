# Fanplay Design System — Adventure Journal

Theme presets live in [`themes/`](themes/). Each preset has its own design
brief: [Adventure Journal](themes/adventure-journal/DESIGN.md), [Arcade
Neon](themes/arcade-neon/DESIGN.md), [Moonlit Grove](themes/moonlit-grove/DESIGN.md),
and [Field Guide](themes/field-guide/DESIGN.md).
The launcher ships all four and can load additional definitions from
`config/themes.json`.

## Direction

FANPLAY should feel like opening a well-loved handheld RPG: welcoming, colorful,
and ready for another journey. Its main audience plays Pokémon fangames and
RPG Maker XP adventures. Use the visual language of route maps, party menus,
save screens, and framed dialogue boxes to make that purpose recognizable.

The shell is an adventure journal around each game's own world. Warm paper,
forest green, water blue, and small golden details replace the neon arcade
palette. Crisp frames and restrained pixel accents evoke the handheld and
RPG Maker era; readable typography and straightforward controls keep the
library comfortable for everyday use.

These are target design guidelines, not a claim that every described UI state
or feature is implemented. Apply them consistently across the desktop,
with touch-friendly adaptations where supported.

## Principles

1. **The game is the destination.** Artwork and titles lead the library. During
   play, let the game occupy the screen; reveal the workspace on request.
2. **Adventure over arcade.** Prefer a journal tab, map marker, or menu cursor
   to glowing scoreboards, scanlines, and high-score language.
3. **Familiar without imitation.** Use original landscape motifs and RPG menu
   framing. FANPLAY's identity should not depend on Pokémon logos, Poké Balls,
   official characters, or a replica of a particular game's interface.
4. **Room for every world.** Pokémon fangames inform the character of the shell,
   but other RPG Maker games and supported cores must feel equally at home.
5. **Honest, useful information.** Distinguish engine, compatibility, and session
   state. A game appearing in the library does not mean it is known to run.

## Colors

### Day palette — default library and settings

| Token | Value | Use |
| --- | --- | --- |
| Background | `#EAE6D8` | Warm parchment page background |
| Surface | `#FFF9EC` | Cards, menus, dialogue panels |
| Surface inset | `#E2E8DC` | Grouped settings, recessed areas |
| Text | `#263C36` | Titles and body text |
| Text muted | `#52645C` | Descriptions and metadata |
| Border subtle | `#BCC6B4` | Decorative dividers and card edges |
| Border strong | `#6B7D6A` | Input boundaries and menu frames |
| Primary | `#28654D` | Play actions, selected controls |
| Primary hover | `#1E503D` | Hovered primary actions |
| Secondary | `#285F86` | Links, information, focus rings |
| Accent | `#D4A43B` | Small chapter markers and featured details |
| Success | `#28654D` | Completed operations |
| Warning | `#815600` | Compatibility caveats and attention |
| Error | `#A9363E` | Failures and destructive actions |
| On solid | `#FFF9EC` | Text on primary, secondary, or error fills |

Gold is decorative or paired with dark text; it is not a small-text color on
paper. Reserve red for errors and destructive actions, not the primary Play
button. Use explicit labels and icons alongside status colors.

### Night palette — optional theme and quiet player chrome

| Token | Value |
| --- | --- |
| Background | `#172722` |
| Surface | `#22382F` |
| Surface inset | `#1C3028` |
| Text | `#F4F0E3` |
| Text muted | `#B5C5B7` |
| Border subtle | `#40594B` |
| Border strong | `#809787` |
| Primary / Success | `#8BCDAB` |
| Primary hover | `#A3DCBD` |
| Secondary | `#95C9ED` |
| Accent / Warning | `#E7BE65` |
| Error | `#F39A9A` |
| On solid | `#172722` |

Keep the same component hierarchy in both themes. Use the night palette for
chrome immediately around a running game and a neutral `#101614` letterbox.
Never tint the game's output to match the shell.

## Typography

- **Headings:** Nunito Sans, bold (700–800), with a system sans-serif fallback.
  Friendly, compact headings should recall menu labels without looking like an
  arcade marquee.
- **Body:** Work Sans, regular or medium, with a system sans-serif fallback.
- **Technical values:** Space Mono, with a monospace fallback, for paths, IDs,
  key bindings, and diagnostics. Ordinary dates and play metadata use body text.
- **Pixel lettering:** Optional for a short decorative wordmark or chapter
  marker only. Never use it for descriptions, settings, errors, or long titles.

| Style | Size / line height | Weight |
| --- | --- | --- |
| Display | 36px / 1.15 | 800 |
| H1 | 28px / 1.2 | 800 |
| H2 | 22px / 1.25 | 700 |
| H3 | 18px / 1.3 | 700 |
| Body | 16px / 1.5 | 400 |
| Small / labels | 14px / 1.45 | 400 / 600 |
| Caption | 12px / 1.4 | 500 |
| Code | 14px / 1.5 | 400 |

Use sentence case. Allow long fangame titles to wrap; never shrink a title into
pixel-sized text. Respect text scaling and provide full text when truncation is
unavoidable.

## Spacing, frames, and depth

- **Spacing:** 4px micro gaps, 8px icon gaps, 16px standard padding, 24px card
  padding, 32px section gaps, and 48px page-level separation.
- **Corners:** 4px for badges and controls, 8px for cards and panels, 12px for
  large dialogs. Reserve pills for compact tags; avoid pill-shaped everything.
- **Frames:** 1px subtle borders for cards; 2px strong borders for prominent
  menus and dialogue panels. A single inset highlight can suggest a classic
  RPG window without copying a game windowskin.
- **Depth:** Flat surfaces by default. Raised panels may use a short 2px solid
  downward shadow; dialogs may use a soft dark shadow at low opacity. No neon
  glows, glass blur, bevel stacks, or pulsing outlines.
- **Decoration:** Sparse tile-like corner marks or a faint route-map motif in
  empty space. Never place textures behind reading areas or the game viewport.

## Artwork and iconography

Use a game's own available cover, title screen, or screenshot as its visual
identity. Preserve aspect ratio; contain title screens and screenshots instead
of cropping away text. Use a consistent media area with a neutral backing.
When artwork is missing, show an original landscape tile and the game's title.

Keep pixel art sharp with nearest-neighbor scaling and integer enlargement
where practical. Smooth illustrations use normal image sampling. Do not apply
pixelation, CRT filters, scanlines, or color grading to every image.

Use one consistent set of simple 16–24px icons: folder for importing, triangle
for Play, journal for the library, and gear for settings. Small stepped pixel
motifs can accent illustrations; functional icons must remain recognizable.
Pair unfamiliar icons with text. Pokémon-specific symbols and type labels belong
in relevant game tools, not in the global navigation for every game.

## Layout and hierarchy

### Library

The first screen should answer: **What do I want to play next?** Lead with the
recently played game when available, followed by a searchable library. Keep
importing a game easy to find and technical tools secondary.

A game card presents artwork, title, engine/core label, and available recent-play
metadata in that order. Give Play a clear primary action; place settings and
folder actions in a secondary area. Use Resume only when it truly resumes an
existing session; launching a game with save files is still Play.

Use a responsive grid that becomes a single column on narrow screens. Do not
compress titles and action labels to preserve a fixed column count. List views
should retain the same information hierarchy for larger collections.

### Player and workspace

The game is the visual focus. Preserve its native aspect ratio, including
RPG Maker XP's usual 4:3 presentation; use the actual output dimensions for
other resolutions and cores. Prefer integer scaling when space permits, with
aspect-preserving fitting on smaller screens. Never stretch or crop gameplay.

Keep workspace controls hidden during normal play, matching the existing
show/hide behavior. When revealed, group session actions together and keep
inspectors, runtime output, and performance tools in secondary panels. Avoid
permanent decorative device frames, fake console buttons, or overlays across
in-game dialogue.

### Game tools and saves

Use compact journal-like tabs and framed detail panels for game-specific tools.
Show Pokémon Essentials sections only when applicable. Species, items, moves,
and their artwork come from the selected game's data; allow custom content and
unknown values without inventing canonical replacements.

Save management should look dependable: clear names, timestamps, and explicit
Backup, Export, Import, or Restore labels where supported. Distinguish file
backups from emulator save states. Restoration prompts name the affected game
and explain what will be replaced; decoration never obscures the consequence.

## Components

### Buttons and selection

- **Primary:** Primary fill, On solid text, 1px matching border. Use Primary
  hover on hover and a slight inset treatment on press. One dominant action
  per card or dialog.
- **Secondary:** Surface fill, Text label, Border strong outline. Hover uses
  Surface inset; suitable for settings and browsing actions.
- **Ghost:** Transparent with Text muted label; Surface inset on hover.
- **Destructive:** Error fill with On solid text, or an Error label and outline
  for lower emphasis. Always use a specific verb such as Delete backup.
- **Selected:** A visible border plus a small menu-cursor marker or checkmark.
  Do not rely on a green background alone.
- **Disabled:** Surface inset fill, Text muted label, no hover or press effects.
  Keep the label readable and explain unavailable actions nearby.

Default controls are at least 44px high with 12–16px horizontal padding. Dense
desktop toolbars may use 32px controls when their labels remain legible; touch
layouts retain at least 44 × 44px targets.

### Cards and lists

Cards use Surface, a subtle 1px border, 8px corners, and 16–24px padding. Hover
strengthens the border without moving the layout. Keyboard focus adds the focus
ring. A selected card has a persistent marker independent of hover and focus.

Lists use at least 48px rows, 8px × 16px padding, and subtle dividers. Let rows
grow for wrapped titles or descriptions. Avoid leaderboard ranks, score orbs,
and ornamental XP meters in the library.

### Inputs and choice controls

Inputs use Surface, Text, a 1px Border strong outline, 4px corners, and persistent
14px labels. Default height is 44px. Helpers sit below the field; errors add an
Error border, an icon, and a specific message. Placeholder text is not a label.

Checkboxes and radio buttons use a 20px visible control within a 44px target.
Selected controls use Primary and On solid with a checkmark, dash, or radio dot.
Group settings under plain descriptive headings, not invented RPG statistics.

### Tags and status

Use 12–14px sentence-case labels, 4px × 8px padding, and 4px corners. Engine/core
tags use neutral surfaces. Status tags use semantic text and a matching icon on
a theme surface; reserve filled treatments for active filters.

Keep Downloaded, Running, and compatibility labels distinct. Use Unknown when
there is no compatibility evidence; use Tested only when backed by a relevant
test. Never infer compatibility from a green card or the engine name.

### Dialogs, tooltips, and feedback

Dialogs borrow the clear border and generous padding of RPG dialogue windows,
with ordinary readable text and explicit action labels. Avoid typewriter text
for confirmations, errors, or instructions.

Tooltips use Surface, Text, a strong border, and 8px × 12px padding. Show them on
keyboard focus as well as hover. Essential instructions remain visible outside
tooltips. Completion feedback is brief and quiet; errors stay available until
addressed or dismissed. Loading states describe the operation and show actual
progress when known, otherwise an indeterminate indicator.

## Interaction and accessibility

- Use 100–160ms color or border transitions. Avoid perpetual animation,
  flashing, bouncing cards, animated backgrounds, and mandatory menu sounds.
- Honor reduced-motion preferences; no information depends on animation.
- Keyboard focus uses a solid 2px Secondary ring with a 2px Surface-colored gap.
  Keep it distinct from selection and visible on every interactive surface.
- Maintain at least 4.5:1 contrast for normal text, 3:1 for large text and
  meaningful control boundaries. Validate actual foreground/background pairs
  in both themes; subtle borders are decorative, not input boundaries.
- Keep logical navigation order, accessible names, and non-color state cues.
  If controller navigation is available, use the same visible focus treatment
  and display prompts for the active input method.
- Use plain language: Play, Add game, Save files, Controls, and Settings. Save
  themed phrases for a short empty-state invitation such as “Your next
  adventure starts here,” followed by a concrete Add game action.

## Review checklist

- Does the library feel like a collection of RPG adventures at first glance?
- Are game titles and artwork more prominent than shell decoration?
- Do paper surfaces, restrained color, and crisp frames replace arcade glow?
- Are long titles, missing artwork, and non-Pokémon games handled gracefully?
- Can players distinguish focus, selection, errors, and compatibility status?
- Does the workspace stay quiet around gameplay and readable in both themes?
