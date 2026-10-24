# The fixture session

A tiny, made-up run of converting a program called **Widget** (by the
made-up Acme Corp), kept in the repo so the whole PortableForge
pipeline can be rehearsed offline — no Windows Sandbox, no real
program, nothing installed. The three notes in `exchange/` pretend
the test box saw this:

- installing Widget put `widget.exe` in Program Files, a settings
  file in the roaming app-data folder, and a Start Menu shortcut;
- running it changed the settings file;
- its uninstall entry says the name is *Widget*, the maker is
  *Acme Corp*, and the version is *1.2.3*.

`docs/OFFLINE_REHEARSAL.md` walks the pipeline from this fixture,
step by step, with the exact commands and what each one should say.
A test in the core library also reads this fixture, so it can never
quietly rot.
