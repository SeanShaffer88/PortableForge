# Accessibility notes

## Appearance

PortableForge uses the GitHub palettes from TesselGUI revision 2. **Appearance**
offers System, GitHub Dark, GitHub Light and three text sizes. The wizard
remembers these choices locally. System uses the desktop scheme reported by egui
and falls back to GitHub Dark.

On Windows, a contrast theme takes precedence. PortableForge reads native
window, control, text and highlight colors on startup, when focus returns,
and when egui reports a different system theme. A change made while
PortableForge remains focused takes effect at the next refresh.

Palette tests verify 4.5:1 contrast for foreground and muted text on task
surfaces, and for the primary label on normal, hovered and pressed fills.
Focus and selection indicators are checked independently at 3:1 against the
task surface. Warnings and outcomes include text. [TesselGUI revision 2 verification](https://github.com/SeanShaffer88/TesselGUI/blob/a689a5e3bfadf9e1e77a3acea826bea5cc717e94/docs/verification.md)

## Keyboard and focus

- Tab and Shift+Tab move between controls. Enter activates the focused action.
- Ctrl+K opens **Commands**. Type to filter, use the arrow keys to move, and
  press Enter to run the selected action. Unavailable actions remain disabled.
- Ctrl+, opens **Appearance**. F1 opens **Status and details** or shortcut help
  in the saved-profile workspace. F6 moves focus between available regions.
- Escape dismisses a dialog or goes back when the current step allows it.
  Commands and Appearance restore focus to their opener. Task navigation
  focuses the heading so the next controls follow in reading order.
- **Needs attention** shows unresolved required warnings. Review choices
  remain with their findings when you page, filter, or go back.

PortableForge exposes native accessibility information through egui and
AccessKit. Screen-reader behavior, including announcements of progress and
validation, needs a separate Windows assistive-technology pass.

## Text and layout

Standard, Large and Extra large use 100%, 125% and 150% text zoom. The
640 × 480 logical minimum is tested with populated controls, long paths,
and paged review. A long path can wrap in the field and opens in a full-value
dialog when more room is needed. A copy action is available in that dialog.

At minimum size the capture task stays in one column so file paths have the
full width. Prerequisites sit with the capture controls. The wizard uses
32-point Compact controls there and 40-point Comfortable controls when
the available layout is at least 700 × 620 logical points. Phase and data
rows use the Compact row height. Text and primary fills follow the Tessel
tokens. The app reads installed Windows fonts and uses egui's local
fallbacks when the requested font is unavailable.

The interface uses flat task surfaces and a fixed action strip. Task
progress appears as status text. Geometry tests establish layout fit for
the exercised states. The fit tests cover up to 150%. Physical DPI
transitions, 200% text scaling, Windows contrast theme switching, and
Narrator interaction require separate live checks.
