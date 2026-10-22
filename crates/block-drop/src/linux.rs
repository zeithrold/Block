//! X11 implementation: an owned, transparent `InputOnly` child window carrying
//! the XDND target over the rendered quick-link drop zone.
//!
//! GPUI's window and event loop are never patched. The launcher creates its own
//! X connection and a child of the product window at the zone's client
//! coordinates. The child is input-only, so it never paints over the rendered
//! zone, but it is a real X window that drag sources can target through XDND.
//!
//! Wayland has no equivalent here: the pinned GPUI exposes only the raw
//! `wl_surface` pointer, so an owned, positioned drop surface would need its own
//! Wayland connection and `wl_data_device` plumbing. The caller reports that
//! limitation instead of pretending a Wayland target exists.

use std::{
    fmt,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, bail};
use async_channel::Sender as PayloadSender;
use x11rb::{
    connection::Connection as _,
    protocol::xproto::{
        Alloc, Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, Colormap,
        ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, StackMode,
        Visualid, Window, WindowClass,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

use crate::{DropZone, quick_link_payload};

/// XDND version advertised through the `XdndAware` property.
const XDND_VERSION: u32 = 5;
/// How long the event loop may sleep between polls; short enough for drag UI.
const POLL_INTERVAL: Duration = Duration::from_millis(5);
/// Upper bound for a dragged text property; quick links are far smaller.
const MAX_TEXT_BYTES: u32 = 8192;

#[derive(Clone, Copy, Debug)]
enum Command {
    Zone(DropZone),
    Shutdown,
}

struct Atoms {
    aware: Atom,
    enter: Atom,
    position: Atom,
    status: Atom,
    drop: Atom,
    leave: Atom,
    finished: Atom,
    selection: Atom,
    type_list: Atom,
    action_copy: Atom,
    action_none: Atom,
    utf8: Atom,
    text_plain: Atom,
    string: Atom,
    property: Atom,
}

struct DragState {
    source: Window,
    /// Target type the source offered and we will request on drop.
    offered: Option<Atom>,
    accepted: bool,
    pending: bool,
    drop_time: u32,
}

impl DragState {
    const fn empty() -> Self {
        Self {
            source: 0,
            offered: None,
            accepted: false,
            pending: false,
            drop_time: 0,
        }
    }

    fn reset(&mut self) {
        *self = Self::empty();
    }
}

/// Owned X11 drop target. Dropping the value shuts the event thread down and
/// destroys the child window.
pub(crate) struct NativeTarget {
    commands: Sender<Command>,
    thread: Option<thread::JoinHandle<()>>,
}

impl fmt::Debug for NativeTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NativeTarget(x11)")
    }
}

impl NativeTarget {
    pub(crate) fn attach(
        parent: isize,
        zone: DropZone,
        payloads: PayloadSender<String>,
    ) -> Result<Self> {
        let parent = u32::try_from(parent)
            .ok()
            .filter(|parent| *parent != 0)
            .ok_or_else(|| anyhow!("X11 window handle is missing"))?;
        let (commands, command_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("block-drop-x11".into())
            .spawn(move || run(parent, zone, payloads, command_rx, ready_tx))
            .map_err(|error| anyhow!("could not start the X11 drop-target thread: {error}"))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                commands,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(anyhow!("{error}"))
            }
            Err(_) => {
                let _ = thread.join();
                Err(anyhow!(
                    "the X11 drop-target thread exited before it was ready"
                ))
            }
        }
    }

    pub(crate) fn set_zone(&self, zone: DropZone) {
        let _ = self.commands.send(Command::Zone(zone));
    }
}

impl Drop for NativeTarget {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Ready {
    connection: RustConnection,
    /// The GPUI client window whose position anchors the rendered zone.
    parent: Window,
    root: Window,
    window: Window,
    colormap: Colormap,
    atoms: Atoms,
    payloads: PayloadSender<String>,
    drag: DragState,
    zone: DropZone,
    last_refresh: Instant,
}

fn run(
    parent: Window,
    initial_zone: DropZone,
    payloads: PayloadSender<String>,
    commands: Receiver<Command>,
    ready: Sender<std::result::Result<(), String>>,
) {
    let mut ready = Some(ready);
    let mut state = match open(parent, initial_zone, payloads) {
        Ok(state) => {
            if let Some(sender) = ready.take() {
                let _ = sender.send(Ok(()));
            }
            state
        }
        Err(error) => {
            if let Some(sender) = ready.take() {
                let _ = sender.send(Err(format!("{error:#}")));
            }
            return;
        }
    };
    loop {
        let mut shutdown = false;
        loop {
            match commands.try_recv() {
                Ok(Command::Zone(zone)) => configure_zone(&mut state, zone),
                Ok(Command::Shutdown) | Err(TryRecvError::Disconnected) => {
                    shutdown = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        if shutdown {
            break;
        }
        if state.last_refresh.elapsed() >= Duration::from_millis(200) {
            refresh(&mut state);
        }
        match state.connection.poll_for_event() {
            Ok(Some(event)) => {
                if let x11rb::protocol::Event::ConfigureNotify(configure) = &event
                    && configure.window == state.parent
                {
                    // The product window moved or was restacked; follow it.
                    refresh(&mut state);
                }
                handle_event(&mut state, event);
            }
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(_) => break,
        }
        let _ = state.connection.flush();
    }
    let _ = state.connection.free_colormap(state.colormap);
    let _ = state.connection.destroy_window(state.window);
    let _ = state.connection.flush();
}

fn open(parent: Window, zone: DropZone, payloads: PayloadSender<String>) -> Result<Ready> {
    let (connection, screen) =
        x11rb::connect(None).map_err(|error| anyhow!("could not open the X11 display: {error}"))?;
    let atoms = intern_atoms(&connection)?;
    let root_info = connection
        .setup()
        .roots
        .get(screen)
        .ok_or_else(|| anyhow!("the X11 display has no root window"))?;
    let root = root_info.root;
    let compositor = connection
        .intern_atom(false, b"_NET_WM_CM_S0")?
        .reply()?
        .atom;
    if connection.get_selection_owner(compositor)?.reply()?.owner == 0 {
        bail!("the X11 session has no compositor");
    }
    let visual = root_info
        .allowed_depths
        .iter()
        .find(|depth| depth.depth == 32)
        .and_then(|depth| depth.visuals.first())
        .map(|visual| Visualid::from(visual.visual_id))
        .ok_or_else(|| anyhow!("the X11 session has no 32-bit ARGB visual"))?;
    let colormap = connection.generate_id()?;
    connection.create_colormap(Alloc::NONE, colormap, root, visual)?;
    // XDND sources look up the immediate child of the root under the pointer, so
    // the target must be a top-level window, not a child of the app. A 32-bit
    // ARGB visual keeps it invisible under the compositor while it stays a real,
    // hittable InputOutput window (GTK sources skip InputOnly windows and
    // intersect the bounding and input shapes when hit-testing).
    connection
        .change_window_attributes(
            parent,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
        )
        .map_err(|error| anyhow!("could not observe the product window: {error}"))?;
    let window = connection.generate_id()?;
    let aux = CreateWindowAux::new()
        .override_redirect(1)
        .background_pixel(0)
        .border_pixel(0)
        .colormap(colormap)
        .event_mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY);
    connection
        .create_window(
            32,
            window,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &aux,
        )
        .map_err(|error| anyhow!("could not create the X11 drop window: {error}"))?;
    connection.change_property32(
        PropMode::REPLACE,
        window,
        atoms.aware,
        AtomEnum::ATOM,
        &[XDND_VERSION],
    )?;
    connection.map_window(window)?;
    let mut state = Ready {
        connection,
        parent,
        root,
        window,
        colormap,
        atoms,
        payloads,
        drag: DragState::empty(),
        zone,
        last_refresh: Instant::now(),
    };
    refresh(&mut state);
    Ok(state)
}

fn intern_atoms(connection: &RustConnection) -> Result<Atoms> {
    let intern =
        |name: &[u8]| -> Result<Atom> { Ok(connection.intern_atom(false, name)?.reply()?.atom) };
    Ok(Atoms {
        aware: intern(b"XdndAware")?,
        enter: intern(b"XdndEnter")?,
        position: intern(b"XdndPosition")?,
        status: intern(b"XdndStatus")?,
        drop: intern(b"XdndDrop")?,
        leave: intern(b"XdndLeave")?,
        finished: intern(b"XdndFinished")?,
        selection: intern(b"XdndSelection")?,
        type_list: intern(b"XdndTypeList")?,
        action_copy: intern(b"XdndActionCopy")?,
        action_none: intern(b"XdndActionNone")?,
        utf8: intern(b"UTF8_STRING")?,
        text_plain: intern(b"text/plain")?,
        string: intern(b"STRING")?,
        property: intern(b"BLOCK_QUICK_LINK")?,
    })
}

fn zone_rect(zone: DropZone) -> (u16, u16, u16, u16) {
    let (x, y, width, height) = zone.pixel_rect();
    (
        u16::try_from(x.clamp(0, i32::from(u16::MAX))).unwrap_or(0),
        u16::try_from(y.clamp(0, i32::from(u16::MAX))).unwrap_or(0),
        u16::try_from(width.clamp(1, i32::from(u16::MAX))).unwrap_or(1),
        u16::try_from(height.clamp(1, i32::from(u16::MAX))).unwrap_or(1),
    )
}

fn configure_zone(state: &mut Ready, zone: DropZone) {
    state.zone = zone;
    refresh(state);
}

/// Place the input-only overlay at the zone's absolute root coordinates and keep
/// it above the product window, following later moves and restacks.
fn refresh(state: &mut Ready) {
    let Ok(cookie) = state
        .connection
        .translate_coordinates(state.parent, state.root, 0, 0)
    else {
        return;
    };
    let Ok(origin) = cookie.reply() else {
        return;
    };
    let (x, y, width, height) = zone_rect(state.zone);
    let aux = ConfigureWindowAux::new()
        .x(i32::from(x) + i32::from(origin.dst_x))
        .y(i32::from(y) + i32::from(origin.dst_y))
        .width(u32::from(width))
        .height(u32::from(height))
        .stack_mode(StackMode::ABOVE);
    let _ = state.connection.configure_window(state.window, &aux);
    let _ = state.connection.flush();
    state.last_refresh = Instant::now();
}

fn handle_event(state: &mut Ready, event: x11rb::protocol::Event) {
    let x11rb::protocol::Event::ClientMessage(message) = event else {
        if let x11rb::protocol::Event::SelectionNotify(selection) = event {
            eprintln!("Q8DBG SelectionNotify property={}", selection.property);
            handle_selection(state, selection);
        }
        return;
    };
    let data = message.data.as_data32();
    eprintln!("Q8DBG client message type={} data={data:?}", message.type_);
    if message.type_ == state.atoms.enter {
        handle_enter(state, data);
    } else if message.type_ == state.atoms.position {
        handle_position(state, data);
    } else if message.type_ == state.atoms.drop {
        handle_drop(state, data);
    } else if message.type_ == state.atoms.leave {
        state.drag.reset();
    }
}

fn handle_enter(state: &mut Ready, data: [u32; 5]) {
    let source = data[0];
    let count = data[1] & 0x0000_0007;
    let types = if count == 0 {
        read_type_list(state, source)
    } else {
        data[2..].get(..count as usize).unwrap_or(&[]).to_vec()
    };
    let mut offered = crate::text_offer(
        &types,
        state.atoms.utf8,
        state.atoms.text_plain,
        state.atoms.string,
    );
    // Some GTK sources advertise more than three types through XdndTypeList.
    // Read the property again at enter time in that case; when it is still
    // unavailable the drop requests UTF8_STRING optimistically, and the
    // payload validation below still rejects anything that is not a quick link.
    if offered.is_none() && types.is_empty() {
        offered = Some(state.atoms.utf8);
    }
    state.drag.reset();
    state.drag.source = source;
    state.drag.offered = offered;
    state.drag.accepted = offered.is_some();
}

fn read_type_list(state: &Ready, source: Window) -> Vec<Atom> {
    let Ok(cookie) =
        state
            .connection
            .get_property(false, source, state.atoms.type_list, AtomEnum::ATOM, 0, 32)
    else {
        return Vec::new();
    };
    let Ok(reply) = cookie.reply() else {
        return Vec::new();
    };
    reply
        .value32()
        .map(|values| values.collect())
        .unwrap_or_default()
}

fn handle_position(state: &mut Ready, data: [u32; 5]) {
    let source = data[0];
    if source != state.drag.source {
        return;
    }
    let flags = if state.drag.accepted { 1 } else { 0 };
    let action = if state.drag.accepted {
        state.atoms.action_copy
    } else {
        state.atoms.action_none
    };
    send_message(
        state,
        state.atoms.status,
        source,
        [state.window, flags, 0, 0, action],
    );
}

fn handle_drop(state: &mut Ready, data: [u32; 5]) {
    let source = data[0];
    if source != state.drag.source {
        return;
    }
    state.drag.drop_time = data[2];
    let refresh = read_type_list(state, source);
    if let Some(target) = crate::text_offer(
        &refresh,
        state.atoms.utf8,
        state.atoms.text_plain,
        state.atoms.string,
    ) {
        state.drag.offered = Some(target);
        state.drag.accepted = true;
    }
    let Some(target) = state.drag.offered.filter(|_| state.drag.accepted) else {
        finish(state, source, false);
        state.drag.reset();
        return;
    };
    state.drag.pending = true;
    let _ = state.connection.convert_selection(
        state.window,
        state.atoms.selection,
        target,
        state.atoms.property,
        data[2],
    );
    let _ = state.connection.flush();
}

fn handle_selection(state: &mut Ready, event: x11rb::protocol::xproto::SelectionNotifyEvent) {
    if !state.drag.pending || event.requestor != state.window || event.property == 0 {
        state.drag.pending = false;
        finish(state, state.drag.source, false);
        state.drag.reset();
        return;
    }
    let text = state
        .connection
        .get_property(
            true,
            state.window,
            state.atoms.property,
            AtomEnum::ANY,
            0,
            MAX_TEXT_BYTES / 4,
        )
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| String::from_utf8_lossy(&reply.value).into_owned());
    let accepted = text
        .as_deref()
        .is_some_and(|text| quick_link_payload(text).is_some());
    if accepted && let Some(text) = text {
        // Bounded channel: a full queue means the UI is already handling the
        // previous drop, so the newest payload is dropped without blocking the
        // X event thread.
        let _ = state.payloads.try_send(text);
    }
    finish(state, state.drag.source, accepted);
    state.drag.reset();
}

fn finish(state: &Ready, source: Window, accepted: bool) {
    let flags = u32::from(accepted);
    let action = if accepted {
        state.atoms.action_copy
    } else {
        state.atoms.action_none
    };
    send_message(
        state,
        state.atoms.finished,
        source,
        [state.window, flags, action, 0, 0],
    );
}

fn send_message(state: &Ready, kind: Atom, destination: Window, data: [u32; 5]) {
    let event = ClientMessageEvent::new(32, state.window, kind, data);
    let _ = state
        .connection
        .send_event(false, destination, EventMask::NO_EVENT, event);
    let _ = state.connection.flush();
}
