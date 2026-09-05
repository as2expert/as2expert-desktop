# AS2Expert Desktop

A native, cross-platform desktop client for **AS2Expert** — read, organize, send
and receive AS2/EDI messages, like an email client for B2B messaging.

Built in Rust on [egui/eframe](https://github.com/emilk/egui) with the OpenGL
(`glow`) backend — a single self-contained binary per OS, **no webview, no web
stack**. Every API call goes through the official
[`as2expert`](https://github.com/as2expert/as2expert-rust) Rust SDK.

## Features

- A web-portal-style layout: a **station picker** in the toolbar, a **folder
  tree** on the left, a sortable **message grid** in the centre, and a **reading
  pane** on the right — with Silk icons and a refined light theme.
- Connect to `free`, `b2b`, or any self-hosted AS2Expert deployment.
- Pick a station and browse its **real folder tree** (loaded from the API, with
  hierarchy and per-folder counts); the grid follows the selected folder.
- Sort by subject/partner/date/MDN and search by subject or partner;
  incoming/outgoing and MDN status at a glance.
- Read a message: metadata (partner, AS2 id, MDN, signature, encryption) and
  its decoded payload.
- Organize: mark read / unread, delete, and **download the payload to a location
  you choose** (native save dialog).
- Compose and send a new message to a trading partner — **Browse** for the file
  (native open dialog) or drag it onto the window.
- **Maintenance modules** (left nav rail): browse and create **stations**,
  **partners** and **certificates** (self-signed) — each with a searchable grid,
  a detail pane, and a create form driven entirely by the API.
- All work runs off the UI thread; the window stays responsive.

## Install

Download a binary for your platform from the
[Releases](https://github.com/as2expert/as2expert-desktop/releases) page:

| Platform | Asset |
|----------|-------|
| macOS (Intel + Apple Silicon) | `as2expert-desktop-macos-universal.tar.gz` |
| Windows x86_64 | `as2expert-desktop-windows-x86_64.zip` |
| Linux x86_64 | `as2expert-desktop-linux-x86_64.tar.gz` |

Unpack and run the executable. On macOS, the binary is unsigned — the first
launch may require *right-click → Open* (or `xattr -dr com.apple.quarantine`).

## Build from source

Requires a recent stable Rust toolchain.

```bash
cargo build --release
./target/release/as2expert-desktop
```

On Linux you need the usual windowing/GL development headers:

```bash
sudo apt-get install -y libgl1-mesa-dev libxkbcommon-dev libwayland-dev \
  libx11-dev libxcursor-dev libxrandr-dev libxi-dev pkg-config
```

## Usage

1. Launch the app and pick an **environment** (or a custom base URL).
2. Paste your **API token** (create one in the AS2Expert portal). Tick
   *Remember token* to store it for next time.
3. **Connect** — the station list and your messages load.
4. Select a station to filter, click a message to read it, or **New message**
   to send one.

### Where settings live

Connection settings are stored in a small `config.json` under the OS config
directory (`%APPDATA%\as2expert` on Windows, `~/Library/Application
Support/as2expert` on macOS, `~/.config/as2expert` on Linux). The token is
written **only** when *Remember token* is enabled, and in plain text — leave it
off on shared machines.

## Remote desktop (RDP / VNC / VDI)

Remote sessions typically expose only OpenGL 1.1, which is too old for the GPU
renderer. The app detects a remote session and automatically switches to a
**software renderer** — local sessions keep hardware acceleration.

- **Windows (RDP):** the release ships Mesa's `llvmpipe` software OpenGL in a
  `mesa/` folder next to the executable; it is loaded only inside an RDP session.
  Keep the `mesa/` folder alongside `as2expert-desktop.exe`.
- **Linux (VNC/SSH-forwarded):** software rendering is enabled automatically for
  SSH sessions; otherwise the app retries in software if the GPU context fails.

You can force software rendering anywhere by setting `AS2EXPERT_SOFTWARE_GL=1`
before launching.

## Dependencies

Kept deliberately small for a GUI: `eframe`/`egui` (glow backend, no wgpu, no
webview), the `as2expert` SDK, a `tokio` runtime for off-thread calls, and
`serde_json`. No native file-dialog crate — the compose window accepts a dropped
file or a typed path.

## Icons

Toolbar and list icons are from the **Silk** icon set by Mark James
(famfamfam.com), licensed under [Creative Commons Attribution 2.5](https://creativecommons.org/licenses/by/2.5/)
and bundled under `assets/icons/` (see `assets/icons/NOTICE`). The application
code is Apache-2.0.

## License

Apache-2.0 — see [LICENSE](LICENSE).
