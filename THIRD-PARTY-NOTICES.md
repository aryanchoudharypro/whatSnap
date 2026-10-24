# Third-Party Notices

whatSnap ships as two executables with different licenses. This is deliberate —
see "Why two binaries" below.

## whatSnap.exe — Rust frontend — MIT

Covered by `LICENSE`. Notable dependencies, all permissive (MIT, Apache-2.0,
BSD, Zlib, or dual MIT/Apache-2.0):

| Component | License |
|---|---|
| wxDragon | MIT |
| wxWidgets (linked by wxDragon) | wxWindows Library Licence (LGPL-2.1 + static-linking exception) |
| live-region | MIT |
| AccessKit | MIT / Apache-2.0 |
| rodio | MIT / Apache-2.0 |
| rodio-wsola | Apache-2.0 |
| opus (libopus bindings) | BSD-3-Clause |
| nnnoiseless (RNNoise) | BSD-3-Clause |
| qrcode, serde, serde_json, windows | MIT / Apache-2.0 |
| wx-utils, key-chord | MIT |
| libphonenumber metadata (generated into `src/phone_data.rs`) | Apache-2.0 |

The wxWindows Library Licence permits static linking into a binary released
under any terms, including proprietary ones, without requiring relinkable
object files.

## backend.exe — Go backend — GPLv3

Covered by `backend/LICENSE`.

`backend.exe` statically links `go.mau.fi/libsignal`, which is **GPLv3**
(26 of its packages are in the build graph). GPLv3 is strong copyleft, so the
backend binary as a whole must be distributed under GPLv3, and complete
corresponding source must be offered to anyone who receives it. The public
repository satisfies that obligation.

A copy of `go.mau.fi/libsignal` v0.2.2 with one small change (debug logging that
is discarded anyway is skipped) lives in `backend/third_party/libsignal`, with its
own GPLv3 LICENSE; see `PATCHED.md` there.

| Component | License |
|---|---|
| go.mau.fi/libsignal | **GPL-3.0** |
| go.mau.fi/whatsmeow | MPL-2.0 |
| go.mau.fi/util | MPL-2.0 |
| modernc.org/sqlite | BSD-3-Clause |
| github.com/Microsoft/go-winio | MIT |
| github.com/rs/zerolog | MIT |
| github.com/beeper/argo-go | Apache-2.0 |
| google.golang.org/protobuf, golang.org/x/* | BSD-3-Clause |

whatsmeow is used via the fork pinned by the `replace` directive in
`backend/go.mod`. MPL-2.0 is file-level copyleft: modifications to
whatsmeow's own files must be published, which the fork does.

## Why two binaries

The GPLv3 obligation stops at the process boundary. The frontend and backend
are separate programs that exchange JSON over a Windows named pipe — an
arm's-length IPC mechanism of the kind the FSF describes as forming separate
works rather than one derivative work. Merging them into a single linked
executable would put the whole thing under GPLv3 and remove that separation.

## Rebuilding

Both binaries build from source in this repository:

    cargo build --release

which drives `go build` for the backend via `build.rs`. Requires a Rust
toolchain (see `rust-version` in `Cargo.toml`) and Go (see `go` in
`backend/go.mod`).
