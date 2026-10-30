# Windows Explorer replacement

This document describes the unreleased Shell Registry migration. It does not
establish Windows 10/11 interactive compatibility or replace installed-artifact
acceptance testing.

## Routes and lifetime

The preference redirects the current user's Folder `open` and `explore` commands,
the Win+E `opennewwindow` CLSID, and the Directory/Drive `Strata.Open` verbs.
The two Folder commands and the CLSID also receive an empty `DelegateExecute`
value. All writes use HKCU; HKCR is read only to inspect effective associations.
No administrator privileges, keyboard hook, resident helper or new Run entry is
required. The GUI observes registry changes without handling keyboard input.

The versioned proxy lives in `%LOCALAPPDATA%\Strata\ShellIntegration`. Commands
quote the fully expanded proxy path. `--open "%1\."` preserves a drive root's
trailing separator when Windows parses the command line. `--activate` opens the
default location through the application's existing single-instance transport.
Warm requests use the SID/session-scoped acknowledged pipe. A separate launch
mutex and a process identity marker coordinate overlapping cold requests.

## Recovery and ownership

`HKCU\Software\Strata\FileManagerV2` stores a bounded, versioned binary journal.
Before changing any handler, it flushes the complete original and intended raw
registry values, their types, the observed snapshot, created key paths, and the
frozen executable/proxy identity. Prepared or restoring transactions are recovered
before dispatch. Missing or malformed journals fail closed.

Rollback compares current values with the transaction's writes and observed
snapshot. It restores only those values, retaining unrelated changes made by
another manager. Keys are removed only when empty; no production rollback
recursively deletes Shell trees. Concurrent writes by another application are
not an atomic cross-process transaction, so conflicts remain visible rather than
being silently overwritten.

Migration imports the v1 backups, including the original Run value, into the
separate v2 state path. Turning the preference off restores the values preceding
v1, rather than restoring v1 as the previous manager. Old v1 metadata is retired
only after comparison with its imported snapshot. The old helper is stopped only
after the new routes are registered. V1 did not record every original verb label
or intermediate key's existence; unknown historical information cannot be
reconstructed, and unrelated values or children are preserved.

Settings retains the saved intent if reconciliation fails, shows the actual
health inline, and offers retry. Probing and registry notifications are read only.
Taking over another manager requires confirmation. Registry work, proxy copying
and helper shutdown run outside GTK's main thread.

The installer upgrades an already owned registration without opting in a new
installation. Uninstall runs standalone rollback before deleting application
files and stops if rollback fails. A residual InstallationId alone does not
prevent uninstall.

The standalone helper reads the same LocalAppData configuration directory as
GLib, respecting `XDG_CONFIG_HOME` when set. It bounds preference-file reads and
does not treat malformed or missing preferences as an explicit opt-in.

## Diagnostics and isolated validation

`strata-shell.exe --status-json` reports desired and actual state, route ownership,
transaction identity and legacy watcher presence without starting GTK or changing
registry values. `--integration-protocol` identifies the helper protocol. These
checks report registry health, not proof that Win+E works on a particular Windows
build. Paths and user document contents are excluded from reconciliation logs.

Native tests use randomized keys under `HKCU\Software\Strata\Tests`, never the
real Shell class keys. Test artifacts are launched using
`scripts/windows/private-test-desktop.py` with a private Win32 desktop, temporary
profile and kill-on-close process job. Do not use this runner for clipboard tests:
the window station's clipboard is shared. It refuses a zero-test result and does
not fall back to the interactive desktop.

Before promotion, exercise Win+E, folder/drive opens, cold concurrent activation,
Explorer restart, logoff/logon, upgrades and uninstall on disposable Windows 10
and Windows 11 VMs. Virtual Shell namespaces are currently rejected by the
absolute-path dispatch boundary; this remains an interactive compatibility gap,
not a verified Explorer replacement route. Do not inject synthetic Win+E or
change the owner's live Shell registrations as an automated test.

## References

- [Microsoft: merged HKCR view](https://learn.microsoft.com/en-us/windows/win32/sysinfo/hkey-classes-root-key)
- [Microsoft: low-level keyboard hook timeout](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc)
- [Files Explorer registration](https://github.com/files-community/Files/blob/main/src/Files.App/Assets/FilesOpenDialog/SetFilesAsDefault.reg)
- [Inno Setup script events](https://jrsoftware.org/ishelp/topic_scriptevents.htm)
- [GLib configuration-directory selection](https://gitlab.gnome.org/GNOME/glib/-/blob/main/glib/gutils.c)
