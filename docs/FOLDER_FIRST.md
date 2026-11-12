# Folder-first package workspace

The Windows x64 wizard accepts an unpacked app folder through drag-and-drop or the folder picker. PortableForge inventories the supplied folder, proposes the executable when there is a clear candidate, and reads bounded metadata. The review uses the existing Rust, egui, and TesselGUI stack. The scan and build do not start the selected app or change its files.

## Supported inspection

- For a raw app folder, choose a scanned executable or use the single recommendation when the evidence identifies one. The check keeps the full path relative to the selected folder. It looks in up to 32 recognized text settings files, at most 128 KiB each, for likely Windows absolute paths and parent-folder references. A warning names the file and the issue, without copying values from the file. These checks suggest what to review before moving the app.
- For a prepared package, the check reads `App/AppInfo/appinfo.ini` and resolves `[Control] Start` relative to the package root. It checks the basic format, identity, license fields, and version. When `App/AppInfo/Launcher/<AppID>.ini` exists, the check resolves `[Launch] ProgramExecutable` and recognized alternate executable fields relative to `App`. A package with its own launcher may omit that file. The check accepts a plain executable path for each supported entry and reports paths it cannot resolve to a scanned executable. Dynamic targets, including launchers that resolve Java from an external runtime, are outside this check's supported scope.
- Optional `App/AppInfo/installer.ini` receives a bounded syntax check and checks for `[Source] IncludeInstallerSource` and `[CheckRunning]` values. Other options remain unverified. Existing files are read in place and preserved. These checks cover a subset of the format and do not establish build compatibility. [PortableApps.com Format](https://portableapps.com/development/portableapps.com_format) [Launcher launch settings](https://portableapps.com/manuals/PortableApps.comLauncher/ref/launcher.ini/launch.html) [Installer settings](https://portableapps.com/manuals/PortableApps.comLauncher/ref/paf/installer.html)

Incomplete inventory, unsafe choices, and malformed supported metadata are blocking findings. The folder workspace builds unpacked apps and preserves a nested entry point with its supporting files. The prepared-package check remains available in the core, and the trusted prepared-package compiler remains a separate entry point.

## Build transaction

Use the artwork editor from the folder review to preview an app icon or installer left pane. Only applied edits are included in a build. Choosing a new folder or a different executable clears the edits. With no changes, the build uses the selected executable’s icon or the generated default and keeps the installer’s default pane.

The wizard collects the relative entry point, app details, pinned tool locations, a new output folder, and explicit permission to include the app files. The builder rechecks the source and stages a private copy with the same layout. It generates `App/AppInfo/appinfo.ini`, `App/AppInfo/installer.ini`, and the launcher configuration before invoking the pinned official tools. The launcher starts the selected program in its packaged directory.

The installer and receipt are published together through a same-volume directory rename. An existing destination is preserved. Stop requests are cooperative and checked before publication. Temporary files are cleaned up on success, failure, or a completed stop. If Windows prevents cleanup, the error identifies the affected path.

The receipt records the build evidence and final installer hash. Fixed-path and parent-folder findings are advisory. Runtime portability and host-state restoration require an isolated app test. Settings values are preserved as supplied.
