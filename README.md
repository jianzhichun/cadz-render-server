# cadz-render-server

CADCraft's editing UI drawn offscreen, with no window, and streamed into an embedding host's pane:
frames go out, the user's pointer, keyboard and IME input come back, and the model drives the same
drawing session over CADCraft's control channel. The program is `apps/cadcraft-render-server`
(binary `cadcraft-render-server`); its wire protocol is in
[`apps/cadcraft-render-server/PROTOCOL.md`](apps/cadcraft-render-server/PROTOCOL.md).

This repository is [CADCraft](https://github.com/storytold/cadcraft) v0.20.1 (by the ArtCraft team
and the CADCraft contributors) with that program added and a few small, opt-in hooks in the engine and
the interface that an embedding host turns on: its product's name in the messages, read-only, and the
control server on a port and token the host chooses. Without a host nothing in CADCraft's own programs
changes. The list of changes is in [`apps/cadcraft-render-server/NOTICE`](apps/cadcraft-render-server/NOTICE).

## Build and check

```
cargo build -p cadcraft-render-server --release
cargo test -p cadcraft-render-server --release
target/release/cadcraft-render-server --snapshot out --frames 1 --size 1280x800
python3 apps/cadcraft-render-server/scripts/stream_smoke.py target/release/cadcraft-render-server stream-out
```

The workflow [`.github/workflows/cadz-render-server.yml`](.github/workflows/cadz-render-server.yml)
builds Linux x64, Windows x64 and macOS arm64, draws one frame and runs a short stream session on each,
and publishes a release for a tag `render-server-v<CADCraft version>-<build>`.

## Licence

MIT OR Apache-2.0, as CADCraft (LICENSE-MIT, LICENSE-APACHE, NOTICE). Copyright (c) 2026 ArtCraft
Team and the CADCraft contributors. Bundled assets are listed in ATTRIBUTION.md. The ArtCraft name,
wordmark and logos are trademarks of the ArtCraft Team and are not part of this repository; this
modified version is not an ArtCraft product. CADCraft is not affiliated with Autodesk, Inc.
