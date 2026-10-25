# ArcadeNeon Design System

## Overview

ArcadeNeon is a neon-soaked, CRT-glow, retro-arcade design system for retro gaming platforms and arcade-style applications. Its electric palette of hot pink, cyan, and lime pulses against a deep navy-black background, channeling the energy of dimly-lit arcades and glowing CRT screens. Bold display typography and colored glow shadows complete the immersive throwback aesthetic.

---

## Colors

- **Primary Hot Pink** (#FF006E): Primary actions, key accents
- **Secondary Electric Blue** (#00F0FF): Links, highlights, P2 accents
- **Tertiary Lime** (#39FF14): Success, scores, XP
- **Background** (#0A0A14): Page background
- **Surface Default** (#12121F): Card backgrounds
- **Success** (#39FF14): High score, win, level up
- **Warning** (#FFD600): Low lives, time running out
- **Error** (#FF006E): Game over, critical failure
- **Info** (#00F0FF): Tutorials, hints, power-ups

## Typography

- **Headline Font**: Bungee
- **Body Font**: Work Sans
- **Mono Font**: Space Mono

- **Display**: Bungee 48px regular, 1.1 line height
- **H1**: Bungee 36px regular, 1.15 line height
- **H2**: Bungee 28px regular, 1.2 line height
- **H3**: Bungee 22px regular, 1.25 line height
- **H4**: Bungee 18px regular, 1.3 line height
- **Body LG**: Work Sans 18px regular, 1.6 line height
- **Body**: Work Sans 16px regular, 1.6 line height
- **Body SM**: Work Sans 14px regular, 1.5 line height
- **Caption**: Work Sans 12px medium, 1.4 line height
- **Code**: Space Mono 14px regular, 1.5 line height

---

## Spacing

Base unit: **8px**
- **xs**: 4px — Inline icon gaps
- **sm**: 8px — Tight component padding
- **md**: 16px — Default padding
- **lg**: 24px — Card padding
- **xl**: 32px — Section gaps
- **2xl**: 48px — Layout sections
- **3xl**: 64px — Page-level spacing

## Border Radius

- **sm** (4px): Chips, small badges
- **DEFAULT** (8px): Buttons, cards, inputs
- **md** (12px): Modals, panels
- **lg** (16px): Large containers
- **full** (9999px): Avatars, score orbs

## Elevation

Neon-glow colored shadows for maximum arcade energy.
- **sm**: 6px glow #FF006E at 30%. Pink glow, buttons.
- **DEFAULT**: 12px glow #FF006E at 35%. Cards, panels.
- **md**: 24px glow #FF006E at 40%. Elevated elements.
- **lg**: 48px glow #FF006E at 50%. Hero/featured items.
- **glow-cyan**: 20px glow #00F0FF at 45%. Cyan-themed elements.
- **glow-lime**: 20px glow #39FF14 at 45%. Score, success states.

## Components

### Buttons
#### Variants
- **Primary**: #FF006E fill, #FFFFFF text, no border. Hover: glow sm, brightness 1.15.
- **Secondary**: transparent fill, #00F0FF text, 1px #00F0FF border. Hover: glow-cyan sm, bg #00F0FF15.
- **Ghost**: transparent fill, #9999AA text, no border. Hover: bg #1A1A2E.
- **Destructive**: #FF006E fill, #FFFFFF text, no border. Hover: bg #CC0058, glow md.
#### Sizes
Sizes: sm (6px 14px, 14px, 32px), md (8px 22px, 16px, 40px), lg (12px 30px, 18px, 48px).
#### Disabled State
0.3 opacity.
- disabled cursor
- All glow effects removed; flat appearance
---

### Cards
- **Default**: #12121F fill, 1px #2A2A3E border, no shadow, 8px radius.
- **Elevated**: #1A1A2E fill, 1px #FF006E40 border, DEFAULT shadow, 8px radius.
** 24px **padding, ** optional 2px left border in any brand color for category coding **neon accent, ** border color increases to full opacity, glow intensifies **hover (elevated).
---

### Inputs
- **Default**: 1px #2A2A3E border, #12121F fill, no shadow.
- **Hover**: 1px #FF006E border, #12121F fill, no shadow.
- **Focus**: 2px #FF006E border, #12121F fill, 4px ring #FF006E at 20% shadow.
- **Error**: 2px #FF006E border, #12121F fill, 4px ring #FF006E at 30% shadow.
- **Disabled**: 1px #1A1A2E border, #0A0A14 fill, no shadow.
** 40px | **Padding:** 8px 12px | **Radius:** 8px **height, ** Work Sans 14px/500, color #EEEEF0, bottom margin 6px **label, ** Work Sans 12px/400, color #9999AA, top margin 4px **helper text, ** Work Sans 12px/400, color #FF006E, top margin 4px **error text.
---

### Chips
- **Filter**: #12121F fill, #9999AA text, 1px #2A2A3E border.
- **Filter Active**: #FF006E fill, #FFFFFF text, no border.
- **Status Success**: #39FF1420 fill, #39FF14 text, no border.
- **Status Warning**: #FFD60020 fill, #FFD600 text, no border.
- **Status Error**: #FF006E20 fill, #FF006E text, no border.
** 4px 12px | **Radius:** 4px | **Font:** Space Mono 11px/400, uppercase, tracking 1px **padding.
---

### Lists
** 48px **row height, ** 8px 16px **padding, ** 1px #1A1A2E **divider, ** #1A1A2E **hover background, ** #FF006E0D **active background, ** Work Sans 16px/400 #EEEEF0 for label, 14px/400 #9999AA for description **font, ** Space Mono, color #FF006E for leaderboard lists **rank numbers.
---

### Checkboxes
** 20px x 20px | **Radius:** 4px **size, ** border 2px #2A2A3E, background #12121F **unchecked, ** background #FF006E, border none, checkmark #FFFFFF, box-shadow glow sm **checked, ** background #FF006E, dash #FFFFFF **indeterminate, ** 30% opacity, disabled cursor, no glow **disabled, ** 8px left of label text **label spacing.
---

### Radio Buttons
** 20px x 20px | **Radius:** full (circle) **size, ** border 2px #2A2A3E, background #12121F **unchecked, ** border 2px #00F0FF, inner dot 10px #00F0FF, box-shadow glow-cyan sm **selected, ** 30% opacity, disabled cursor, no glow **disabled, ** 8px left of label text **label spacing.
---

### Tooltips
** #1A1A2E **background, ** #EEEEF0, Work Sans 12px/400 **text, ** 1px #2A2A3E **border, ** 6px 12px | **Radius:** 8px **padding, ** 6px triangle matching background **arrow, ** 240px **max width, ** 150ms show, 0ms hide **delay.
---

## Do's and Don'ts

1. **Do** use neon glow shadows on interactive elements to reinforce the arcade identity.
2. **Do** keep the deep navy-black (#0A0A14) background consistent; it is the stage for all neon elements.
3. **Do** use Bungee exclusively for headings; its blocky display style sells the retro-arcade mood.
4. **Don't** apply glow effects to body text or large blocks -- glows are for interactive and highlight elements only.
5. **Don't** use more than two neon colors in a single component; pick one primary and one accent per context.
6. **Do** use Space Mono for scores, countdowns, and any numerical display for that authentic arcade feel.
7. **Don't** use subtle, muted interactions; ArcadeNeon should feel responsive with visible hover state changes.
8. **Do** leverage color-coded glow shadows (pink, cyan, lime) to differentiate functional areas.
9. **Don't** use light backgrounds for any surface; the dark-first design is non-negotiable.
10. **Do** animate neon glows with CSS pulse or breathe keyframes for featured or active states.