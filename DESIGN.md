# ClearWave visual direction

Reference: [Omarchy](https://omarchy.org/), inspected on 2026-11-18 using
its rendered desktop page, HTML, branding extraction and full-page screenshot.
The inspected page selected Tokyo Night; its colors are one theme, not a fixed
ClearWave palette. The temporary reference capture is
`/tmp/clearwave-omarchy-reference.png` (not a shipped asset).

## Observed reference

Flat rectangular panels, fine borders, generous empty space and a restrained
accent distinguish controls. The hero uses pixel artwork; supporting text uses
monospace. Primary actions have a solid accent fill; secondary actions are
outlined/darker. The capture shows little rounding or shadow.

Extracted reference colors: background `#1A1B26`, foreground `#C0CAF5`, accent
`#9ECE6A`, secondary button surface `#1F2230`, border `#414868`.
The extraction reports square buttons and no shadows. It reports both Geist
and JetBrains Mono font families, so it is not evidence that every site element
uses one font. Website photographs, logos and artwork are not app assets.

## Native application choices

- Use the installed system monospace font discovered through fontconfig. On this
  machine `fc-match monospace` resolves to **JetBrainsMono Nerd Font**, at
  `/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf`. Retain a fallback when
  fontconfig or that font is unavailable on another distribution.
- Body, control labels, meters, EQ labels, secondary text and tooltips: **14
  logical pixels**. Respect desktop scaling. App title: 28; section headings:
  18–20. These are requested app sizes, not extracted website sizes.
- Keep existing Omarchy-following and manual theme choices. Map background,
  raised surface, text, border, accent and warning roles through that palette.
- Use restrained corners, thin borders and consistent 8/16/24 spacing. Preserve
  readable contrast on light and dark themes; avoid faint essential labels.
- Put optional explanations behind small information buttons with hover help
  and keyboard/click access. Keep running/capture status, errors and consequential
  startup choices directly visible.
- Output and Input contain device, processing, preset, gain and EQ controls.
  Advanced contains monitoring, detailed processing, background and login policy.
- Keep a clear action hierarchy and compact layout that fits the laptop's
  logical 1280px width; allow vertical scrolling rather than shrinking text.

These are adaptations to egui, not a pixel-perfect reconstruction of the site.
Visual acceptance requires inspecting native app captures on the laptop's second
workspace, including light/dark themes and the Advanced page.
