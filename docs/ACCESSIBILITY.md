# Accessibility notes

## Appearance

PortableForge defaults to Dark High Contrast for new settings. **Appearance**
also offers System, GitHub Dark and GitHub Light. The wizard
remembers explicit choices locally. System uses the
desktop scheme reported by egui and falls back to GitHub Dark.

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
  Commands, Appearance, and Package artwork restore focus to their opener.
  Task navigation focuses the heading so the next controls follow in reading order.
  Back and Escape return from captured findings to **Review Your Package**.
- **Needs attention** shows unresolved required warnings. Review choices
  remain with their findings when you page, filter, or go back.

PortableForge exposes native accessibility information through egui and
AccessKit. Screen-reader behavior, including announcements of progress and
validation, needs a separate Windows assistive-technology pass.

## Activity and file selection

**Activity** stays on the right throughout the guided workspace, across from
the roadmap. F6 moves focus between them and the other workspace regions.
**Technical details** shows available diagnostic messages. Windows contrast settings take precedence
over the app appearance. File pickers stay inside the current window and
accept keyboard navigation and typed paths.

## Text and layout

Drag a window corner to scale the window, text, and controls together.
Long paths stay available in full. A long path can wrap in the field and
opens in a full-value dialog when more room is needed. A copy action is
available in that dialog.

The left roadmap shows the current phase and available navigation. The two
side panes have matching widths. Available fields and artwork
controls send plain-language help to the fixed status bar on pointer hover or
keyboard focus. The artwork dialog shows both previews when it fits and uses
tabs in the compact layout. The charcoal work surface uses subtle circuit
traces in Dark appearance. Light and Windows contrast appearances use
plain backgrounds.

The interface uses flat task surfaces and a fixed action strip. Task
progress appears as status text. Geometry tests establish layout fit for
the exercised states. Physical DPI transitions, Windows contrast theme switching, and
Narrator interaction require separate live checks.

## Build from a ZIP, app folder, or selected app files

**Choose App Folder** and **Choose App Files** provide keyboard alternatives
to dragging a source onto the first screen. **Choose App Files** accepts
one ZIP or several files for the same app. Review the suggested choices and choose **Build Installer** once the required decisions are complete. Use **Cancel Preparation** to stop a ZIP
or file selection while it is being prepared. **Save for Later** returns to
the source screen. **Resume App Package** reopens the saved selection for
review.

App details, build tools and output, and folder
findings use compact pages with a shared **Back** action. **F1** opens
findings. Start-file choices and findings are paged, and **View Path**
opens long paths in the full-value dialog. The wizard keeps **Build
Installer** in the action strip on the review screen, and displays
**Stop After This Step** while a running build can still be stopped.
Geometry tests exercise the populated folder pages at 640 × 480 for
Standard, Large, and Extra large text.

**Edit Package Artwork** opens the same icon and installer pane controls
from folder review. **Apply Artwork** keeps edits for the current review,
and **Cancel** or Escape discards unapplied edits. Closing the editor
returns focus to **Edit Package Artwork**.

**Test Installer** opens a confirmation from the built result. **Cancel**
has initial focus, and **Start Test** opens the installer in a fresh
Sandboxie-Plus box after checking the original build receipt. The running
screen shows the setup and two-launch instructions. **Cancel Test**
requests cancellation, and the wizard waits for the cleanup result.
**Show Test Details** opens a paged report. The built installer remains
available after the test.
