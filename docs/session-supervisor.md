# Session supervision and crash help

Native Profile launches use one independent supervisor per game session. The same `block` executable has three modes: the launcher, headless `--supervise <run-id>`, and `--crash <run-id>`. Internal modes require `--data-dir`; they are not saved launch recipes.

## Ownership and authentication

The launcher refreshes the selected Microsoft account, then transfers only the current game session and its expiry over the supervisor's inherited stdin pipe. Supervisor arguments contain the run ID and data directory, not credentials. The bootstrap is not written to disk or placed in environment variables. The supervisor does not sign in, access refresh-token storage, or refresh credentials. It checks authorization expiry again before binding the Java command. The eventual Java command still uses Minecraft's authentication arguments; command previews and captured output are redacted.

The supervisor constructs the transient LaunchPlan and owns the Java child, log pipes, installation/game-directory locks, and sticky directory reservation. Closing the launcher does not request Stop. Reopening it restores active sessions from the registry and reconnects through local IPC. **Stop** explicitly requests cancellation from the owning supervisor. Development recipes retain their existing execution path.

Public session metadata lives under `<data-dir>/sessions/<run-id>/session.json`: IDs, version/loader, game directory, process IDs, state, revision, bounded redacted output, and capture warnings. It contains no account credentials or serialized LaunchPlan. All active records remain observable; only terminal history shown by the observer is capped at 128 records. Live status requests have bounded concurrency and deadlines.

## Exit facts and local help

Exit facts distinguish normal exit, explicit Stop, failed start, abnormal termination, and interrupted monitoring. Available Unix signals and Windows status values are retained. A Stop request alone does not turn an unrelated crash into a successful stop. Normal exits and confirmed stops do not open crash help.

For a failed start or abnormal exit, the supervisor saves `crash.json`. An open launcher can claim and show it inline; otherwise the supervisor attempts to open a separate help window. A file lock prevents competing presenters. The report is acknowledged only after the help view has been constructed. If no window can open, the unread report remains available on the next launcher start.

The help page presents profile/version context, possible explanations, matching evidence, and suggested next steps. Local rules recognize Java heap exhaustion, native-memory exhaustion, incompatible Java, mod dependencies, and graphics initialization failures. These are heuristics, not confirmed root causes. Unrecognized output remains explicitly unknown; an exit code alone is insufficient. The page provides logs/details, a summary clipboard action, a user-selected JSON export, and a return to the instance without automatic relaunch or configuration changes.

## Diagnostic bounds and privacy

Collection compares the current run against a pre-launch report baseline, excluding unchanged old reports. It considers Minecraft crash reports and JVM fatal logs, rejects symlinked or escaped paths, and limits directory scans. Portable filesystem checks are not an atomic guarantee against an adversarial concurrent writer.

| Content | Limit |
| --- | --- |
| Captured output line | 64 KiB; an oversized line is discarded in full |
| Captured output stream | 16 MiB each for stdout and stderr |
| Live registry output | 48 lines, up to 384 characters each |
| Diagnostic output tail | 64 KiB |
| Report attachments | 8 sanitized text copies, up to 128 KiB each |
| Matching evidence | 3 excerpts per rule, up to 400 bytes each |
| Serialized report reader | 8 MiB, allowing for JSON control-character expansion |

Output continues draining after a stream reaches its capture limit or a log write fails. Credential redaction precedes display and export; the defensive sanitizer also drops generic credential-bearing lines and HotSpot command/environment sections. Only the sanitized report payload is exported; the report folder is not recursively bundled. Capture, indexing, and persistence failures are recorded when possible and do not replace the observed exit result. Storage failures can still prevent saving a report.

Reports stay local and are not automatically uploaded. Redaction is not a complete anonymizer: player names, paths, chat, server addresses, and arbitrary mod output may still be personal. Review exported reports before sharing. Original game/JVM log files are not rewritten and may contain sensitive information.

## Interrupted monitoring and recovery

A missing supervisor does not prove that Minecraft ended. Block displays interrupted monitoring, preserves the sticky reservation, and refuses a second writer to that game directory. It does not kill a process using a saved PID, automatically clear stale reservations, or automatically restart Minecraft.

The low-level `GameDirectoryGuard::clear_session_reservation(directory, run_id)` API supports operator-controlled cleanup of a matching reservation. It rejects invalid paths, malformed markers, and mismatched run IDs, but does not verify process death. Clearing the marker alone does not restore Play: the orphaned run record still appears active.

Operator recovery requires independently verifying that both the previous game and supervisor have ended, then establishing exclusive access. Clear only the matching reservation and archive that run's directory outside `<data-dir>/sessions` before reopening Block. Preserve the original diagnostic facts; do not manually rewrite the record as a normal exit or kill a process using a saved PID. There is currently no automatic recovery or GUI/CLI recovery workflow. Removing a marker while a game may still be running is not supported.

## Validation scope

`crates/block-app/tests/session_supervisor.rs` launches the actual supervisor executable and a compiled synthetic Java fixture. It exercises process detachment, reconnect/Stop, normal and abnormal exits, launch failures, redaction, fresh-report selection, unread presentation, and retained reservations after monitor death. GPUI tests cover help interaction/export, inline return, session restoration, stale updates, and requested-profile selection. The observer regression includes an older active session behind more than 128 newer terminal runs.

These tests use real operating-system processes with synthetic game data and credentials. They do not establish real Minecraft gameplay or Microsoft authorization. The additional native acceptance below remains separate from the synthetic suite.

### Native acceptance on 2026-10-21

The complete verification sequence is formatting, workspace type checking with all targets/features, strict Clippy, then all workspace tests. A frozen source snapshot was verified independently on each environment; private logs retain phase exit codes, source fingerprints, failures, and subsequent complete reruns. Documentation and API-comment updates followed the verified implementation snapshot.

| Environment | Complete verification | Passed tests | Process integration tests |
| --- | --- | --- | --- |
| macOS ARM64 | Passed | 153 | 8 |
| Windows AMD64 | Passed | 143 | 7 |
| Debian ARM64 | Passed | 153 | 8 |
| Debian AMD64 | Passed | 153 | 8 |

The implementation snapshot fingerprint is `e4cfa11122f7701348015da414a9d363e3c73a0518d04c6698e3c2d7d9743c21`. Unix-only lifecycle/symlink cases account for platform-specific test counts. Windows execution exposed Win32 lock contention returning error 33; presentation and polling now use the existing portable `fs2::lock_contended_error` comparison. The ARM64 synthetic fixture explicitly selects Official compatibility because its fake child loads no native code; product ARM64 compatibility checks remain intact. Earlier fixture failures, a stale cached UI test, and an assertion updated for stronger sanitization were retained separately from successful final runs.

Computer Use verified macOS native session restoration, window closure with both owned processes surviving, standalone crash help, technical details, a system save dialog and sanitized export, and return without relaunch. The synthetic heap-failure case displays matching evidence; unknown failures remain unknown.

A separate developer-offline acceptance launched real Minecraft 1.20.6 with the original managed Java binary and a new Isolated Profile. After render-thread initialization, Block was closed natively; the identified Java process and supervisor remained alive. A controlled SIGABRT in that owned, world-free Profile produced an abnormal outcome with signal 6 and automatically opened independent Unknown help. Returning selected the same Profile without restarting Minecraft. No Minecraft/JVM report attachment was generated for that signal, so no attachment collection is claimed from this scenario.

The real-game run used the preceding macOS implementation build, before the correction for Windows lock contention; the final portable comparison was covered by the complete macOS suite. Four original Profiles, three Instance records, and 55 existing save files retained their settings or exact bytes and modification times. One generated preparation receipt changed as expected. The old temporary Java wrapper was killed with signal 9 during probing; that failed startup remains separate from the passing direct-managed-Java run. All owned game, supervisor and acceptance windows were retired.

This does not establish real-game crash or native graphical acceptance on Windows/Linux, every Loader crash type, natural JVM fatal-report generation, or fresh Microsoft authorization. Those are separate acceptance gates. See the [MVP acceptance record](archive/mvp-progress.md) for earlier gameplay and authentication results.
