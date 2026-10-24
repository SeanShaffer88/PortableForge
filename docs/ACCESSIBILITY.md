# Accessibility notes

How PortableForge's window treats eyes, hands, and keyboards — and
where it honestly falls short. The palette is the family dark theme
(`guide::theme` in the core library); there is no light theme.

## Palette and contrast

Contrast ratios below are computed from the theme's hex values with
the standard WCAG relative-luminance formula. WCAG AA asks for
4.5:1 for ordinary text and 3:1 for large text.

| Pairing | Ratio | Verdict |
|---|---|---|
| Body text `#F5F5F5` on black `#000000` | 19.26:1 | Passes AAA |
| Body text `#F5F5F5` on panel `#101010` | 17.45:1 | Passes AAA |
| Muted text `#A8A8A8` on black | 8.83:1 | Passes AAA |
| Muted text `#A8A8A8` on panel | 8.00:1 | Passes AAA |
| Accent `#E5484D` on black | 5.37:1 | Passes AA |
| Accent `#E5484D` on panel | 4.86:1 | Passes AA |
| Button text `#F5F5F5` on primary red `#6B0700` | 11.63:1 | Passes AAA |
| Faint text `#6E6E6E` on black | 4.12:1 | Below AA for small text |
| Faint text `#6E6E6E` on panel | 3.73:1 | Below AA for small text |

The one honest shortfall: **faint text** is used only for secondary
hints (file sizes, "could not be checked" asides) and never for
anything a person must read to proceed — every step's instructions,
buttons, and verdicts are body or muted text. If faint text ever
carries meaning on its own, that is a bug; the fix is to promote it
to muted. Colour is never the only signal: the safety check pairs
its red crosses with the word and a written fix, and review cards
say their decision in words ("fix on", "leave alone").

A unit test in the core library (`guide` tests) re-computes the key
pairings from the tokens, so a palette change that breaks contrast
fails the gate instead of shipping quietly.

## Keyboard and focus

- The whole wizard is keyboard-usable: Tab moves through the step's
  controls in the order they are drawn (top to bottom), Enter
  activates the focused button, and the wizard adds its own layer —
  **Enter** goes forward where the step allows it, **Escape** goes
  back (see the navigation code in the wizard; both are ignored
  while a text field is being edited, so typing is never hijacked).
- The current step, the progress bar, and the step list on the left
  are always visible, so focus position is never a mystery.
- Buttons carry hover text that repeats, in plain words, what the
  button will do — screen-reader labels come from the same strings
  through egui's AccessKit support.

## Text scaling

- The header's **Text size** control offers 1.0×, 1.25×, and 1.5×,
  applied as the window's zoom factor; the choice is remembered
  between runs. Layout is drawn with relative spacing throughout,
  so larger text reflows instead of clipping.
- Windows display scaling (125%/150% on many laptops) is respected
  through the framework's DPI handling.

## Motion and flashing

The wizard uses no animation beyond the framework's own control
feedback — nothing flashes, blinks, or auto-advances. Long work is
reported as text ("looking through C:\Program Files — 1,250 files
checked so far"), never as a spinner alone.
