# First run on a real Windows machine — runbook

Everything in PortableForge is built and tested off-Windows; this
runbook is the checklist that turns "built" into "verified". Work top
to bottom; each step says what *should* happen and what to send back
if it doesn't. Total time: about 30–45 minutes the first time.

## 0. One-time setup

1. Windows edition must be Pro, Enterprise, or Education (Home cannot
   run Windows Sandbox — on Home, PortableForge offers Careful mode,
   which is labelled honestly and is *not* the same promise).
2. Turn on the test box: Start → "Turn Windows features on or off" →
   tick **Windows Sandbox** → OK → restart.
3. Install Rust from https://rustup.rs if `cargo --version` fails in a
   terminal. (Once GitHub CI is publishing release builds, this step
   goes away for normal use — it is only needed while testing from
   source.)

## 1. Build and first checks (10 minutes)

```text
git clone https://github.com/SeanShaffer88/PortableForge
cd PortableForge
cargo build --release
target\release\portableforge.exe preflight
```

Expected: every line `OK`, ending with the safe test box as the way
we will use. If any line says `MISS`, its fix is written under it —
do that fix, rerun, and send back the output either way.

```text
target\release\portableforge.exe tools --cache tools-pins
```

Expected: all three tools "Ready and checked". If the cache is empty,
download the three files listed in `tools-pins/PINS.md` into
`tools-pins\` first (names must match exactly).

## 2. First real conversion (20 minutes)

Pick a small, free program whose installer you are allowed to share
(a tiny single-purpose utility is ideal for a first run).

1. Launch `target\release\portableforge.exe` with no arguments — the
   guided window opens. Take the screenshots listed in
   `docs/screenshots/CAPTURE.md` as you go.
2. **Pick:** type the setup file's full path. Continue must refuse an
   empty or wrong path with a plain message.
3. **Safety check:** the step shows the real checks. All should pass.
4. **Clean test box / Install / Try it out / Close:** run the program's
   installer inside the test box (the session folder holds the
   ready-made box settings), open the installed program, use it
   briefly, close it, then close the box.
5. **What it left behind:** the review cards appear. Fixes start on;
   any big warning needs its tick before Continue works.
6. **Name / Sharing:** fill the details; for a first test, "It is free
   for anyone to share" is the honest answer only if it is true of
   your test program — otherwise "I am not sure" exercises the safer
   path.
7. **Build:** generate the package folder, then run the two official
   builders against it (each takes the package folder as its only
   argument — the pins step already verified the right files).

## 3. The finishing test (10 minutes)

1. Open a *fresh* test box, copy the finished portable folder in, run
   the portable program, close it.
2. Back on the host:

```text
target\release\portableforge.exe leaks <package-folder-in-box> <before.json> <after.json>
```

Expected: "Perfect." Anything listed is a real finding — send the
whole output back rather than fixing by hand.

## 4. Send back

- The session folder (it contains the run record, notes, and change
  lists — everything needed to reproduce a problem).
- Any step where reality differed from "Expected" above, with the
  exact wording shown.
- The screenshots from `docs/screenshots/CAPTURE.md`.

That closes the last unverified gate: after this run, the README can
say "verified on Windows" and mean it.
