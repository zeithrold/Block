# Local diagnostics

The desktop launcher, its headless supervisor/crash-help modes, and `block-launch`
write local JSON Lines diagnostics. Each process owns a separate set of files.
`--help`, `--version`, desktop `--build-info` and `--list-scenarios` do not start
logging.

## Location

Launcher diagnostics are independent of `BLOCK_DATA_DIR` and `--data-dir`:

| Platform | Base directory |
| --- | --- |
| Linux | `$XDG_STATE_HOME/block/logs`, or `~/.local/state/block/logs` |
| Windows | `%LOCALAPPDATA%\Block\logs` |
| macOS | `~/Library/Logs/Block` |

Files live under `launcher/`. Open the directory from **About → Open logs**.
Game output and sanitized crash reports remain in their existing session
directories under the application data root. Their lifetime stays with the
independent supervisor, not the launcher window.

## Events and filters

Each JSONL record carries a UTC timestamp, severity, target and structured
fields. The initial `application_started` event records the executable version,
OS, CPU architecture and role. Operation and game-session events carry their
existing identifiers; UI diagnostics use navigation's page identity, not visible
user-provided names.

The default filter enables Block `INFO` events and dependency warnings/errors.
Use `RUST_LOG` to opt into a narrower diagnostic scope, for example:

```sh
RUST_LOG=warn,block_app=info,block_cli=info,block_launcher=debug,block_ui=debug just run
```

A nonempty `RUST_LOG` replaces the application defaults, with `WARN` as the
fallback. For example, `RUST_LOG=block_launcher=debug` leaves other targets at
`WARN`; `RUST_LOG=info,block_launcher=debug` enables `INFO` globally.
This filter applies to `tracing` events. Third-party `log`-facade messages are
not bridged into persistent logs: those messages can contain arbitrary paths,
URLs and error text.

UI diagnostics describe state changes: window geometry, page/overlay changes,
motion policy and operation outcomes. A committed navigation state does not
prove a compositor displayed a frame. Physical motion capture and native
interaction remain separate checks. Ordinary logging does not record every
render, layout, download-progress update or pointer movement.

Call sites explicitly select safe fields. They do not serialize account objects,
authentication exchanges, device codes, launch commands, token-bearing plans or
arbitrary error chains. Persistent launcher failures record an event/category;
existing user-facing errors and sanitized game diagnostics retain their own
detail. Do not add `Debug` dumps of request, account or command objects when
adding an event.

## Writing and retention

A bounded background writer keeps disk I/O off UI producers. Events are submitted
without waiting for disk writes. When the queue fills, events can be dropped;
the writer reports the count and the UI surfaces a logging failure once.
Producer-side failure handling does not write to stderr, which may itself be
backpressured. Initialization failures report a safe diagnostic to stderr
before the UI starts. This is diagnostic storage, not an audit log.

The writer flushes at least once per second while running, promptly flushes error
records, and drains accepted records on normal shutdown. Force termination,
process abort or power loss can lose buffered records. There is no per-event
`fsync` guarantee. The desktop installs a GPUI quit callback to drain the writer;
native macOS termination does not unwind the Rust entry point.

Files rotate at 10 MiB. Cleanup targets 100 MiB across closed launcher log
segments, oldest first. Active files are protected from cleanup, so concurrent
processes may temporarily exceed the target. Logging failures do not stop
normal launcher operation.

## Export and sharing

**About → Export logs** asks for a new ZIP destination and exports a bounded
snapshot of launcher JSONL files. An existing destination is refused rather
than overwritten. The snapshot includes complete on-disk records; another
process's buffered events may be one flush interval behind.

While waiting for the save dialog, **Cancel export** abandons that selection and
restores the export action. A later response to the abandoned selection cannot
write a file or affect a retry. There is no deadline on a healthy file-selection
dialog. Cancellation is no longer offered once ZIP writing starts.

Only recognized launcher log files are included, with a 100 MiB input limit.
Game output, accounts, configuration, credentials and arbitrary neighboring files
are not collected. For a game failure, use **Crash help → Export diagnostics**
to export its existing sanitized report separately.

Review diagnostics before sharing, particularly when enabling third-party debug
logging through `RUST_LOG`. Files stay local unless the user explicitly shares
them.

## Linux ARM64 verification

Native checks on 2026-10-30 used the debug `0.1.0` launcher on Debian 13 ARM64,
in KDE's Wayland session with the launcher on Xwayland `:1`. The isolated
source snapshot was based on `ebd5b375a54f1794f31584b5cc68490b6f9590d4` plus the
logging working-tree changes. Disposable About scenarios and a task-owned
`XDG_STATE_HOME` kept the checks separate from real accounts and game data.

- Export opened the KDE save dialog; native keyboard confirmation produced a ZIP
  whose entries were complete, parseable launcher JSONL records. The UI logged
  `ui.logs.export_finished` with outcome `succeeded`.
- Native Escape canceled the next save dialog and logged `cancelled` at INFO.
- Confirming the system overwrite prompt for that test ZIP reached the
  application's refusal: ERROR outcome `failed`, visible recovery feedback,
  and the original ZIP preserved byte-for-byte.
- Open logs displayed the correct task directory in Dolphin. Product Close
  exited the launcher and flushed the final `application_stopped` record.
- A file in place of the log-state directory triggered the visible logging
  failure notice, while About remained usable and normal exit returned zero.

The earlier native focused checks passed 17 storage tests, four executable
logging tests and seven UI logging tests. They were reused for this UI replay;
the product code was unchanged. Captures, JSONL and the test ZIP are retained
under `target/logging-native-linux-arm64/`.

The cancellable-picker increment was checked with `just check` on macOS ARM64
and seven focused About tests on Linux ARM64. Its stalled-picker regression
failed before the change and passed afterward. Additional tests cover view
release, late responses during retry, and visual-snapshot token independence.
A native Linux replay used a private D-Bus session and a test portal that accepts
SaveFile requests but withholds their responses: Cancel export restored the
action, a retry remained pending after the old request's late success response
without creating its file, and the current response produced a valid ZIP.
The normal system portal also passed native Save and normal-exit checks with
the increment. The private portal, bus and test applications were stopped.

### Limits

The healthy-system replay used the repaired system GSettings cache and normal
portal service. These checks do not establish the launcher's native
Wayland-backend behavior or game/authentication acceptance. GPUI's detached
native picker cannot be closed through its public receiver API. Cancel export
abandons Block's wait, not the OS request: a delayed dialog may still appear and
can be dismissed through its native controls. Its result is ignored by Block.
