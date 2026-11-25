# cadcraft-render-server wire protocol, version 1

The render server runs CADCraft's editing UI (`CadApp`, set up as a pane of the host) with no window,
draws each frame into an offscreen texture, and streams it to an embedding host. The host sends the
user's pointer, keyboard and IME input back as UI input. Files go through the host: every file dialog
of the UI becomes an `ask_open` or `ask_save`.

This is the same protocol, message for message and field for field, as `solidcraft-render-server`
v1 (https://github.com/jianzhichun/solidz-render-server, `apps/solidcraft-render-server/PROTOCOL.md`),
so a host's stream code (leg A reader and writer, the page's decoder and input capture) serves both.
What differs is how the process is started and what the editor does with a few messages; each
difference is listed in [Differences from solidcraft-render-server](#differences-from-solidcraft-render-server).

## Legs and framing

| Leg | Between | Transport | Framing |
|---|---|---|---|
| A | render server and host process | one loopback TCP connection; the render server connects to `--stream 127.0.0.1:PORT` (a listener the host made for this one process) | `u32 LE length` + envelope, repeated |
| B | host process and the page | one WebSocket (binary messages) on the host's own origin | one WebSocket message = one envelope |

**Envelope** = `u32 LE header_len` + `header_len` bytes of UTF-8 JSON (an object with `"k"`, the kind)
+ payload bytes (the rest of the envelope). Only `frame` has a payload; every other kind has none.

Limits: header at most 1 MiB (`1 << 20`); envelope at most 64 MiB (`64 << 20`). A peer that receives
more, a header that is not a JSON object, a kind it does not know for that direction, or `v` other
than its own sends `error` and closes the connection.

Messages on one leg arrive in the order they were sent. The host relays to the page exactly
`frame`, `cursor`, `ime`, `copied` (unchanged, payload included), and to the render server exactly
the page's `input`, `resize`, `visible`, `ack` (unchanged). Everything else is the host's own.

## Units

- Positions, sizes and rectangles exchanged with the UI: **CSS pixels of the page's pane = egui
  points**. The render server runs the context at zoom 1 with `pixels_per_point = dpr`, so
  `RawInput.screen_rect = Rect(0, 0, w, h)` in points and
  `viewports[ROOT].native_pixels_per_point = Some(dpr)`.
- A frame's `w`, `h` and its rectangles: **device pixels** (`round(w_css * dpr)`), at most 4096 per
  side (a larger pane is drawn at a lower dpr, which `frame.dpr` reports). The pane itself may be up
  to 16384 CSS px a side (the host refuses more); the UI is laid out over all of it.
- Times: milliseconds, as durations only. The two processes' clocks are never compared.

## Starting (leg A)

```
cadcraft-render-server --stream 127.0.0.1:PORT [--session ID] [--size WxH] [--dpr N] [--lang zh] [--theme dark|light] [--open FILE]
```

`--lang` is an interface language code (`en`, `zh`, `de`, `fr`, … as CADCraft's Edit ▸ Interface
Language offers; Chinese needs a CJK font installed on the computer, else the interface stays English).
`--open FILE` opens a drawing before the first frame (read in the background, as the desktop app
reads one; the host may also open drawings over the control channel, `app.open {path}`).

The process has **no settings folder**: nothing of a desktop CADCraft's per-user files (profile,
preferences, keyboard shortcuts, command aliases, autosave and recovery, LISP support files, logs) is
read or written. A preference changed in the pane lasts as long as the process.

Environment: `CADCRAFT_RENDER_TOKEN` (64 lowercase hex, required: sent back in `hello`, proves the
connection is the process the host started) and `CADCRAFT_CONTROL_TOKEN` (64 lowercase hex, required:
the control channel's token). The render server listens for the control channel on a free loopback
port (`ready.controlPort`) and writes the control token where CADCraft's MCP bridge reads it,
`cadcraft_mcp::control_token::token_path(port)` (`$XDG_RUNTIME_DIR` or `~/.cache`, or
`%LOCALAPPDATA%`: `cadcraft-control-<port>.token`, owner-only), and removes it when it exits. So the
host starts the unmodified bridge as `cadcraft-cli mcp --connect 127.0.0.1:<controlPort>` (the
`cadcraft-cli` of the same release, built from the same upstream commit) and the model's calls drive
the session the user sees. At this commit the bridge refuses `--automation-read-root` /
`--automation-write-root` together with `--connect` (upstream issue #1585): the files the model's
commands open or write over the control channel are not confined by the bridge. CADCraft has no port discovery file; a user's own
`cadcraft-cli mcp` never finds this process unless given its port. `CADCRAFT_RENDER_LOG=error|warn|info|debug`
sets stderr's level (default `warn`).

Snapshot mode, no host: `--snapshot DIR --frames N [--open FILE] [--script FILE] [--hold-ms N] [--gap-ms N] [--size WxH] [--dpr N]`
draws N frames, writes `DIR/frame-0001.png` … and `DIR/frames.json` (each frame's `timing` and the
drawing's `revision`, the adapter, and `gpuMemory` where the backend can say), and exits 0. It also opens
the control channel and prints `control 127.0.0.1:PORT` on stderr (with `control-token …` when no
`CADCRAFT_CONTROL_TOKEN` was given), so scripts can drive and inspect the session. `--script FILE`: wire
`InputEvent`s at given frames (`[{"frame": n, "events": [...]}]`). `--adapter auto|hardware|software`
(either mode) is a local measurement flag the host does not pass.

Sequence:

1. render server connects, opens the graphics device (`hello` names it), sends `hello`. When no device
   opens it sends nothing, writes the reason on stderr and exits 1: the host sees an exit before
   `ready`, not a crash.
2. host checks `v` and the token (constant time); wrong: `error` and close. Right: the host sends
   `policy`, `resize`, `visible` (current state; `resize` may equal the flags).
3. render server opens the control channel, draws the first frame, sends `ready`.
4. steady state: `frame` / `cursor` / `ime` / `copied` / `ask_*` up; `input` / `resize` / `visible` /
   `ack` / `policy` / `answer` down.
5. end: host sends `stop` → render server finishes the frame, removes the control token file, sends
   `bye`, exits. The render server also exits, with `bye` `host-gone` if it can still write, when leg A
   closes or errors: it never outlives its host. An exit without `bye` is a crash to the host.

## Messages

Field names are exactly as written (JSON). Rust: one enum per direction with
`#[serde(tag = "k", rename_all = "snake_case")]`; struct fields `#[serde(rename_all = "camelCase")]`
(the multi-word fields are `inputSeq`, `controlPort`, `decodeMs`, `readOnly`). Optional fields
(`detail`) are omitted when absent; `null` is written where the type says nullable.

### Render server → host

| `k` | fields | when |
|---|---|---|
| `hello` | `v: u32` (=1), `token: string`, `pid: u32`, `version: string` (the app version), `backend: "metal"\|"dx12"\|"vulkan"\|"gl"\|null`, `adapter: string`, `software: bool` | first message |
| `ready` | `controlPort: u16`, `w: u32`, `h: u32` (device px), `dpr: f32`, `samples: u32` (scene MSAA in effect) | control channel up, first frame drawn |
| `frame` | `seq: u64` (from 1), `inputSeq: u64` (highest `input.seq` the pass read; 0 none), `w: u32`, `h: u32`, `dpr: f32`, `full: bool`, `rects: [FrameRect]`, `timing: FrameTiming`; payload | below |
| `cursor` | `icon: CursorIcon` | when the cursor changes |
| `ime` | `ime: ImeState \| null` | when it changes |
| `copied` | `text: string` | the UI copied text (already on the system clipboard) |
| `ask_save` | `ask: u32`, `name: string`, `exts: [string]` (no dot) | the UI asked for a file to save to (`Services::start_pick` with `Pick::Save`) |
| `ask_open` | `ask: u32`, `exts: [string]` (empty: any) | the UI asked for a file to open (`Pick::Open`, `Pick::Lisp`, `Pick::Pgp`) |
| `ask_done` | `ask: u32`, `ok: bool` | the pass that acted on the answer ran (the save or export returned; see below) |
| `bye` | `why: "stop"\|"host-gone"\|"device-lost"\|"error"`, `detail?: string` | last message |
| `error` | `code: "version"\|"token"\|"malformed"\|"too-large"\|"unknown-kind"\|"order"`, `message: string` | then close |

### Host → render server

| `k` | fields | effect |
|---|---|---|
| `input` | `seq: u64` (increasing from 1 per page attach; the render server only echoes the last one it read as `frame.inputSeq` and never checks the order), `events: [InputEvent]` | appended to the next pass's `RawInput.events`, in order |
| `resize` | `w: u32`, `h: u32` (CSS px), `dpr: f32` | new target size; the next frame is `full` |
| `visible` | `visible: bool` | false: passes still run (control requests, timers) but nothing is read back or encoded; true: the next frame is `full` |
| `ack` | `seq: u64`, `decodeMs: f32` | one credit back (start: 2 credits) |
| `policy` | `readOnly: string \| null` | read-only on with this reason, or off |
| `answer` | `ask: u32`, `path: string \| null` | resolves that ask's picker future; null = cancel |
| `stop` | `why: string` | finish and exit |
| `error` | as above | then close |

A save's `answer` is a place of the host's (its staging folder): the UI writes the file there, the
host delivers it after `ask_done` and tells the user where it went. The UI's own status message that
names that place is not shown (an error names the file alone), so the user reads only the host's
location; nothing else on the wire changes.

### Host ↔ page only (the render server never sees these)

`attach {v, session, w, h, dpr, theme, lang}` (page → host, first), `choose_cancel {ask}`,
`restart {}`; `status {state: "starting"|"live"|"fallback"|"stopped", reason?, readOnly}`,
`choose_file {ask, exts}`, `notice {level: "info"|"warn", text}` (host → page). Listed so the kinds
never collide.

### Files through the host

Every file dialog of the UI (File ▸ Open, New from Template, Recover, Save As, Save of a drawing with
no file yet, Export, Import, Publish/Export PDF, the Plot dialog's plot to file, APPLOAD, External
References ▸ Attach, linetype files, the Command Aliases import) goes through the host:

| UI asks for | message | `exts` |
|---|---|---|
| a file to open (drawings, templates, imports, xrefs, linetype files share one dialog) | `ask_open` | `[]` (any file) |
| an AutoLISP file (APPLOAD) | `ask_open` | `["lsp"]` |
| a command alias file | `ask_open` | `["pgp"]` |
| a file to save to | `ask_save` | the suggested name's type first; for a drawing (`dxf`, `dwg`, `dwt`) the other two drawing types after it |

`ask_save.name` is the suggested file name (no folder). The UI shows one dialog at a time.

`ask_done.ok` is true when the answer gave a path and the pass that acted on it put no new message on
the status line (CADCraft reports a failed save, export or refused command there). A drawing opened
from an answer is read in the background: its `ask_done` says the read started.

A save's answer may be a place of the host's (its staging folder): where the UI then names that place
(the command line, the status line), only the file's name is shown, so the user reads only the host's
location. QSAVE of a drawing that has a file writes to that file without asking (as the desktop does).

## Shared types

**FrameRect** `{x: u32, y: u32, w: u32, h: u32, codec: "jpeg"|"png"|"rgba", ew: u32, eh: u32, offset: u32, length: u32}`:
a changed rectangle in device pixels; `ew`×`eh` is the encoded image's size (v1 always `= w`×`h`; a
later draft mode may send it smaller and the page scales it); `offset`/`length` locate its bytes in
the payload. `rgba`: `ew*eh*4` bytes, rows top to bottom, no padding, straight alpha = 255.
`jpeg`/`png`: a complete file each.

**FrameTiming** `{pass: f32, paint: f32, readback: f32, diff: f32, encode: f32}` ms: the UI pass
(`raw_input_hook` + `logic` + `ui`), tessellate + GPU encode/submit, copy back to the CPU (map
wait included), finding the changed tiles, encoding.

**Frame rules.** The page keeps the last frame and draws each rectangle over it. `full: true` means
the rectangles cover the whole `w`×`h`; it is sent first, after every `resize`, after `visible: true`,
and whenever `w`/`h`/`dpr` change. A frame is produced only when the pass painted something different
and a credit is left; while credits are 0 the changed region keeps growing (it is relative to the last
frame *sent*, so nothing is lost) and the next frame carries all of it. At most 60 frames a second.

**InputEvent**: an object with `"e"`. Rust: `#[serde(tag = "e", rename_all = "snake_case")]`.
Each maps to one `egui::Event` (egui 0.37, `crates/egui/src/data/input/event.rs`):

| `e` | fields | `egui::Event` |
|---|---|---|
| `move` | `x, y: f32` | `PointerMoved(pos2(x, y))` |
| `button` | `x, y: f32`, `button: "primary"\|"secondary"\|"middle"\|"extra1"\|"extra2"`, `pressed: bool`, `mods: Mods` | `PointerButton { pos, button, pressed, modifiers }` |
| `gone` | — | `PointerGone` |
| `wheel` | `dx, dy: f32` (already the UI's sign: −DOM delta), `unit: "point"\|"line"\|"page"`, `mods` | `MouseWheel { unit, delta: vec2(dx, dy), phase: TouchPhase::Move, source: MouseWheelSource::Unknown, modifiers }` |
| `zoom` | `factor: f32` | `Zoom(factor)` |
| `key` | `key: string` (DOM `key`), `physical: string \| null` (DOM `code`), `pressed, repeat: bool`, `mods` | `Key { key: Key::from_name(key).or(Key::from_name(physical)), physical_key: physical.and_then(Key::from_name), pressed, repeat, modifiers }`; dropped when neither name maps |
| `text` | `text` | `Text(text)` |
| `preedit` | `text`, `range: [u32, u32] \| null` (chars) | `Ime(ImeEvent::Preedit { text, active_range_chars: range.map(\|[a, b]\| a..b) })` |
| `commit` | `text` | `Ime(ImeEvent::Commit(text))` |
| `delete_surrounding` | `before, after: u32` | `Ime(ImeEvent::DeleteSurrounding { before_chars, after_chars })` |
| `copy` / `cut` | — | `Copy` / `Cut` |
| `paste` | `text` | `Paste(text)` |
| `focus` | `focused: bool` | `WindowFocused(focused)`, and `RawInput.focused` |
| `mods` | `mods` | `ModifiersChanged(modifiers)` |

**Mods** `{alt, ctrl, shift, meta: bool}` → `egui::Modifiers { alt, ctrl, shift, mac_cmd, command }`
with, on macOS, `mac_cmd = meta, command = meta`, elsewhere `mac_cmd = false, command = ctrl` (what
egui-winit does for a native window; the page cannot know which system the render server runs on).

**CursorIcon**: the `egui::CursorIcon` variant name (`Default`, `None`, `ContextMenu`, `Help`,
`PointingHand`, `Progress`, `Wait`, `Cell`, `Crosshair`, `Text`, `VerticalText`, `Alias`, `Copy`,
`Move`, `NoDrop`, `NotAllowed`, `Grab`, `Grabbing`, `AllScroll`, `ResizeHorizontal`, `ResizeNeSw`,
`ResizeNwSe`, `ResizeVertical`, `ResizeEast`, `ResizeSouthEast`, `ResizeSouth`, `ResizeSouthWest`,
`ResizeWest`, `ResizeNorthWest`, `ResizeNorth`, `ResizeNorthEast`, `ResizeColumn`, `ResizeRow`,
`ZoomIn`, `ZoomOut`), from `PlatformOutput.cursor_icon`.

**ImeState** `{purpose: "normal"|"password"|"terminal", rect: [x, y, w, h], cursor: [x, y, w, h], interrupt: bool}`
from `PlatformOutput.ime` (`IMEOutput { purpose, rect, cursor_rect, should_interrupt_composition }`),
in points. The page puts its hidden text area at `cursor`, so the system's candidate box opens at the
caret.

## Product name

The UI says CadZ wherever CADCraft's says CADCraft: every catalog message in every language, menu
labels (Help ▸ About CadZ, CadZ Help), the engine's messages on the command line and in the status
line, notices about caught faults and the graphics device, the Plot dialog's built-in devices.
About shows the product's name, version and licence only: CADCraft's credits, website and community
links are not shown. Help ▸ Check for Updates is not offered (`ui.updates.check` refuses). Quit from
the UI or `app.quit` from the control channel does not end the process: the host does (`stop`).
Fixed in the render server; there is no flag for it.

## What the render server does with the platform output

- `cursor_icon` → `cursor` (on change). `ime` → `ime` (on change).
- `OutputCommand::CopyText(t)` → the system clipboard (arboard), then `copied`. CADCraft mirrors its
  drawing clipboard as DXF text, so a copy of entities is a `copied` with DXF text; `paste` input with
  that text pastes them. `CopyImage`, `OpenUrl` and `TextSelectionSettled` → ignored (logged).
- Viewport commands (`Close`, `Title`, `InnerSize`, `Focus`, `RequestPaintWhileHidden`, …) are ignored:
  the pane is not a window. `Screenshot` (`ui.screenshot` over the control channel) is answered with
  the frame painted after the pass.

## Read-only (`policy`)

`readOnly: "<reason>"` turns the engine's host read-only switch on (`cadcraft_engine::read_only`):
QSAVE/SAVE and SAVEAS are refused with the reason; any other command that changes a drawing is put
back before anything records it and refused ("command `<id>` is not available right now: <reason>"),
whether the user, a script or the control channel ran it; an interactive command's changes are put
back when it ends, with the same message on the command line. Opening, looking, selecting,
measuring, exporting and plotting stay. The render server also keeps every open drawing as it was
(and each one opened later as it opened) and, after any pass that ends with one changed some other
way, puts it back and shows the reason in the status line. `null` turns both off.

## Differences from solidcraft-render-server

Message kinds, fields, framing, limits, units, frame rules and input events are identical. Different:

1. Program and environment names: `cadcraft-render-server`, `CADCRAFT_RENDER_TOKEN`,
   `CADCRAFT_CONTROL_TOKEN`, `CADCRAFT_RENDER_LOG`.
2. The control token **is** written to a file, the one CADCraft's MCP bridge reads for that port
   (`cadcraft-control-<port>.token`, owner-only, removed at exit): CADCraft's bridge takes its token
   from there only. SolidCraft's bridge takes it from the environment, so that render server writes none.
3. No `--config-dir` and no settings folder at all; no `--msaa`; `--open` works in stream mode too.
4. `ready.samples` is always 1: the drawing area is drawn inside the UI's own pass.
5. `ask_open.exts` is `[]` for CADCraft's shared Open dialog; `ask_save.exts` lists the drawing types a
   Save As may switch between. `ask_done.ok` is judged by the status line (above).
6. Read-only refuses QSAVE/SAVEAS and puts back changes (CADCraft has no `file.save`/`document.import`;
   its drawings are kept by reference, not exported as text).
