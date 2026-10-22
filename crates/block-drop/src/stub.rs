//! Inert drop-target stand-in for platforms where the launcher has no OLE.

use std::fmt;

use anyhow::Result;
use async_channel::Sender;

use crate::DropZone;

pub(crate) struct NativeTarget;

impl fmt::Debug for NativeTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeTarget(inert)")
    }
}

impl NativeTarget {
    pub(crate) fn attach(
        _parent: isize,
        _zone: DropZone,
        _payloads: Sender<String>,
    ) -> Result<Self> {
        Ok(Self)
    }

    pub(crate) fn set_zone(&self, _zone: DropZone) {}
}
