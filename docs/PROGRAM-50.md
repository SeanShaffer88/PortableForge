# The 50-Enhancement Programme — "the best self-contained portable-package maker"

Directed by SEAN 2026-10-25: *"Program should be the absolute best
self-contained solution for creating portableapps.com installers
from an archive or setup file with every config and customization
option and easy to use so anyone can use it."*

Rules, same as the 100-programme: five slices of ten; every item
lands gated (full suite, both clippy targets) and published before
the next begins; the plain-language invariant is absolute — new
surface goes behind cards and "Show details", never into primary
wording; modes and the redistribution guardrail are never weakened.
Sequencing: the programme starts once v0.2.1 (the bundled quick
box) has shipped — slices build on the bundle machinery.

## Slice 1 — Start from anything (archive & folder input)

1. **Archive as the source.** The pick step accepts a `.zip`
   (and, where Windows' own `tar.exe` can read them, `.7z`/`.rar`)
   as the program source, by Browse and by drag-and-drop.
2. **Safe unpacking.** Archives unpack into the session folder
   through `tar.exe` (orchestrated, never reimplemented) behind
   the same path-escape refusal the harvest uses; nothing in an
   archive can write outside the session.
3. **Folder as the source.** Pick a folder of an already-unpacked
   program; it is copied into the session the same way.
4. **No-install capture.** Archive/folder runs skip the install
   step: the contents are copied into the test box, the program
   is run there, and only what it *writes while running* is
   captured — the same notes, review, and build afterwards.
5. **Main-file finder.** For archive/folder sources, the likely
   main program file is ranked (place in the tree, size, its own
   name tags) and pre-filled on the naming step.
6. **Archive safety look.** Before anything runs: counts of
   programs, libraries, and script files (`.bat`/`.ps1`/`.vbs`
   named in plain words), nested archives, and anything that
   would start by itself — shown as plain warnings.
7. **Extras travel along.** Dropping several files at once keeps
   the companions (notes, license, picture) as package extras
   instead of losing them.
8. **One-line summary per kind.** The pick step says, in one
   line, what happens next for each kind of source (setup /
   archive / folder / already on this computer).

## Slice 2 — Every package option (the naming step grows up)

9. **All the name-card fields.** Category (plain-word list),
   homepage, longer description, trademarks, and language —
   an "More about the program" card, pre-filled where the
   program's own tags or the capture know them.
10. **Two versions, explained.** The program's version and the
    package's version as separate fields, with the four-number
    rule in plain words.
11. **Your own icon.** Pick a picture file as the package icon;
    the program checks which sizes it has and says what is
    missing.
12. **The finished file's name.** Choose the pattern the final
    file is named with, and see it before building.
13. **Start-menu name and group.** What the installed copy calls
    itself in the Start menu, editable.
14. **License to include.** Pick a license file from the
    captured tree (or your disk) to travel inside the package.
15. **Opening picture.** The launcher's opening-picture choice
    (on/off, how long, your own picture).
16. **"Your finished copy" card.** Before building: final file
    name, estimated size, and a plain list of what will be
    inside.
17. **Look under the lid.** A read-only preview of the two
    generated settings files, behind "Show details", with copy
    buttons — the expert escape hatch, view-only by design.
18. **Saved details.** Publisher/category/homepage patterns can
    be saved and re-applied to later runs in one click.

## Slice 3 — Every behaviour option (the review grows up)

19. **Edit any finding.** Every card on the review step can
    change its suggestion (a plain dropdown of the actions that
    make sense for it) or be removed.
20. **Add your own fix.** Plain templates — "a file that must
    travel", "a settings entry that must travel", "rewrite this
    line when the copy moves", "tidy this empty folder" — added
    as findings with the same cards.
21. **Where things go, editable.** A finding's from/to places
    can be corrected, checked against what was captured.
22. **How it starts.** Start choices card: needs an
    administrator's OK, wait until it closes, close it when
    asked, only one at a time, start in this folder — defaults
    from what the capture saw.
23. **What it needs.** Needs card: remembered-settings access,
    Java, .NET and friends, each with a version when the capture
    saw one — editable.
24. **Its own words for the system.** Name-and-value rows the
    launcher sets while the program runs (the [Environment]
    behaviour), add/remove.
25. **Words it starts with.** A field for the starting words
    (command-line arguments) the copy always uses.
26. **File kinds it opens.** Kinds the capture saw the program
    register are listed; choose which ones the portable copy
    should offer to open.
27. **Its data folder name.** Choose the name of the folder the
    copy keeps its travelling settings in.
28. **Full self-check.** The generated settings are checked
    against the complete rule set of the format (illegal
    combinations named in plain words) before anything builds.

## Slice 4 — Self-contained & build depth

29. **The makers move in.** The two official PortableApps.com
    maker programs join the bundled folder (unmodified, pinned,
    licenses included — the Sandboxie pattern), unpacked into the
    tools place on first use. Building then needs nothing from
    the internet at all.
30. **Fetch-it-for-you depth.** For pass-it-on copies that fetch
    the program at setup time: the download address, its
    fingerprint, extra files with their sizes — a small editor,
    all checked before building.
31. **A zipped copy too.** The build can also leave a plain zip
    of the finished package next to the single file.
32. **Build report.** A short report file lands in the output:
    what was made, which suggestions were used, every warning.
33. **Change and rebuild.** From the done step: copy this run,
    change the review or the details, rebuild — no recapture.
34. **A queue.** Line up several programs on the pick step;
    each is converted in turn into its own output folder.
35. **Keep the original.** Choice to keep the original setup or
    archive in an "original" folder inside the output, for
    pass-it-on copies that need it.
36. **Completeness test.** The testing step also compares the
    copy's contents against everything the install put down,
    and names anything missing in plain words — not just what
    leaked, but what was left out.

## Slice 5 — Anyone can use it

37. **A three-panel welcome.** First run only: what this does,
    what you need, pick your first program. Skippable, never
    shown again.
38. **Starting points.** The pick step opens with three plain
    starting points (a setup file / an archive or folder / a
    program on this computer) that set the flow up for you.
39. **Try the example.** A one-click harmless example program
    runs the whole flow as a demonstration, offline.
40. **"Why do you ask?" everywhere.** Every step gains the
    small explainer the sharing step already has.
41. **A plain glossary.** Behind "Show details": the twenty
    technical words the details use, each in one plain sentence.
42. **Keyboard all the way.** A full keyboard-only pass: visible
    focus everywhere, and the keys listed on a details card.
43. **Named progress, cancellable where safe.** Long jobs (open
    an archive, unpack the makers, copy big trees) show what
    they are doing and allow stopping where stopping is safe.
44. **Recovery cards.** The eight likeliest troubles (box will
    not start, setup wants an administrator, 32-bit only,
    nothing captured, maker missing, disk full, name too long,
    security program blocked it) each get a plain cause and a
    one-click next step.
45. **Past runs library.** The start of the wizard lists past
    runs with how they ended; reopen or copy any of them.
46. **One settings place.** A settings view in the window:
    text size, where finished copies go, what happens to run
    folders, ask-before-closing.
47. **The done card.** Done shows what was made, where it is,
    how big, its test score — with Open folder / Make another /
    Start another buttons.
48. **What's inside.** A details panel listing the versions of
    everything bundled (quick box installer, makers) with their
    fingerprints.
49. **Docs & parity.** `docs/PARITY.md` refreshed from the
    corpus for the whole new surface; the on-machine runbook
    gains the archive flow and the bundled makers.
50. **The 1.0.0 readiness pass.** The release-readiness list
    re-run against everything; 1.0.0 itself still waits for
    SEAN's on-machine pass — this programme does not move that
    gate, it makes what he will test complete.
