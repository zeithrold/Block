# xattr 0.2.3 compatibility patch

Source: the unmodified `xattr` 0.2.3 package from crates.io, with its MIT and Apache-2.0 license files preserved.

GPUI 0.2.2 depends on `zed-async-tar`, which uses this older xattr API. Current libc releases no longer expose `libc::ENOATTR` on Linux/Android, causing the Linux desktop build to fail. The only code change uses the platform's equivalent `libc::ENODATA` for missing extended attributes on Linux/Android. Other platforms retain their original constants and API behavior.

The override is versioned through `[patch.crates-io]`. It avoids changing the global libc version or editing a registry cache. The Linux desktop CI job runs this crate's original tests before checking the application.
