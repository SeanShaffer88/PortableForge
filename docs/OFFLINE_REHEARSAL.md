# The offline rehearsal

The whole PortableForge pipeline — read a finished capture, review
what the program left behind, build the package, judge a leak test —
can be run end to end **without Windows Sandbox, without a real
program, and without touching a real machine's settings**, using
the checked-in fixture session (`examples/fixture-session`). It
takes about five minutes and is the fastest way to see whether a
change broke the pipeline's shape.

The fixture pretends a program called **Widget** (maker: Acme Corp,
version 1.2.3) was installed in a test box: its program file landed
in Program Files, it kept settings in the roaming app-data folder,
it put a shortcut on the Start Menu, and running it changed its
settings file.

## 1. Copy the fixture somewhere writable

```text
cp -r examples/fixture-session /tmp/pf-rehearsal
```

## 2. Read the capture into a review

```text
portableforge post-capture /tmp/pf-rehearsal
```

Expect, in plain words: **1 fix on**, *6 changes from installing, 1
from trying it out*, a starting-settings seed named
`Data\Widget\settings.ini`, the shortcut find, and the program's
own tags (Widget / Acme Corp / 1.2.3). The run also saves
`review.json`, both diffs, and `specimens.json` into the session
folder.

## 3. Look at the review

```text
portableforge review /tmp/pf-rehearsal
```

One card per finding, each with its decision state. The settings
folder is the one fix that is on: it will be moved inside the
package.

## 4. Build the package from the fixture

Make a stand-in program file and a details file (the fixture's own
tags, filled in):

```text
mkdir appfiles && printf 'MZ-widget' > appfiles/widget.exe
```

`details.json`:

```json
{"name":"Widget Portable","app_id":"WidgetPortable","base_app_name":"Widget","publisher":"Acme Corp","homepage":"","category":"Utilities","description":"A made-up program used to rehearse PortableForge offline.","language":"Multilingual","shareable":true,"open_source":false,"freeware":true,"commercial_use":false,"package_version":"1.2.3.0","display_version":"1.2.3","requires_admin":false,"main_exe":"widget.exe","distribution":{"Bundled":{"redistribution_asserted":true}}}
```

```text
portableforge build details.json --out WidgetPortable --session /tmp/pf-rehearsal --app-files appfiles
```

Expect *Package made … Everything checks out*, a note that the
seed was noted but not planted (the fixture captured no file
contents — honest, not an error), a note that the stand-in program
file does not look like a real Windows program (it isn't one), and
a note that a shared package wants a homepage. Inside
`WidgetPortable\Other\Source\` you will find `build-report.json`
(with a UTC `built_at_utc` stamp) and `SHA256SUMS.txt`.

## 5. Judge a leak test

The fixture's notes can stand in for leak notes:

```text
cp /tmp/pf-rehearsal/exchange/before.json /tmp/pf-rehearsal/exchange/leak-before.json
cp /tmp/pf-rehearsal/exchange/after-runtime.json /tmp/pf-rehearsal/exchange/leak-after.json
portableforge verify details.json --out WidgetPortable2 --session /tmp/pf-rehearsal --app-files appfiles
echo $?
```

Expect exit code **2**: the fixture's run wrote its settings
outside the package folder — exactly the thing the accepted fix in
step 3 exists to prevent in the real package. (One nuance: because
these notes double as the *install* notes, the install-time
uninstall entries are listed too. In a real leak run, the
before-note is taken inside the leak box, so only the run's own
writes appear.)

## What the rehearsal proves — and what it does not

It proves the pipeline's plumbing: notes read, classification,
build, package checks, leak judgment, and exit codes all behave,
on any machine, with no sandbox. It does **not** prove anything
about a real Windows run — that is what
`docs/ON_MACHINE_RUNBOOK.md` is for, and until it passes, the
Windows runtime stays labelled Unverified.
