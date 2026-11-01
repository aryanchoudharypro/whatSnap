# whatSnap

whatSnap is a lightweight, fast, and accessible WhatsApp client for Windows. It is designed to be completely usable with screen readers and the keyboard. It does not use a browser or WhatsApp Web in the background. It links to your phone as a WhatsApp companion device, the same way WhatsApp Web and WhatsApp for Windows do.

whatSnap is in development. There are no numbered releases yet. Each change to `master` builds a new Development Build.

## Documentation

For a comprehensive user guide, including a full list of features and hotkeys, please see the [User Manual](doc/readme.md).

## How whatSnap Is Built

whatSnap has two programs:

- `whatSnap.exe` is the user interface. It is written in Rust with [wxDragon](https://github.com/AllenDang/wxDragon), so all controls are native Windows controls. The message list is a custom control that uses [AccessKit](https://accesskit.dev).
- `backend.exe` connects to WhatsApp. It is written in Go, with a fork of [whatsmeow](https://github.com/aryanchoudharypro/whatsmeow) for the protocol and a fork of meowcaller for calls. It keeps the session and the message history in a SQLite database.

`whatSnap.exe` starts `backend.exe` and talks to it through named pipes, with one JSON message per line. During a call, the microphone and speaker audio goes between the two programs over a local UDP socket instead. A third, small program, `whatSnapShare.exe` in `share-target/`, adds whatSnap to the Windows Share menu.

Some internal names still say `whantv`, the old name of the app. These include the pipe names, the single instance mutex, registry values, and the Go module name. We keep them on purpose, so that an old and a new build can't run at the same time and no registry data has to move. Don't rename them. Text that users see must say whatSnap.

## Building

whatSnap builds only on Windows. To build, you'll need:

- Rust `1.88.0` or later, with the MSVC toolchain.
- Go, at the version in `backend/go.mod`.
- CMake and Ninja, for building wxDragon. In addition, you also need LLVM, from LLVM.org.
- The Windows SDK, for `makeappx`. The Visual Studio C++ tools install it.

```batch
cargo build --release
```

`build.rs` also runs `go build` in `backend/`, so this one command makes both programs. It generates `target/release/whatSnap.exe` and `target/release/backend/backend.exe`. Keep the two together: `whatSnap.exe` looks for the backend in the `backend` folder next to it. The build also converts `doc/readme.md` to `readme.html`, and fills in the installer script `packaging/whatSnap.iss.in` as `whatSnap.iss`, both in `target/release`.

### Release Packages

`cargo release` builds everything that a release needs:

- `target\release\whatSnap_setup.exe`, the installer.
- `target\release\whatSnap.zip`, the portable version.

It reads the version from `Cargo.toml`. You don't need to install Inno Setup: if it isn't installed, `cargo release` downloads it one time to a cache for your user. The code is in `xtask/`, and it uses [shipfitter](https://crates.io/crates/shipfitter).

### Update Signatures

whatSnap updates itself with [ship-shape](https://crates.io/crates/ship-shape). It installs an update only when the update's minisign signature verifies with the public key in `src/platform/update_check.rs`. When the `MINISIGN_KEY` and `MINISIGN_PASSWORD` environment variables are set, `cargo release` signs the zip and the installer, and writes a `.minisig` file beside each one. Without the key, it makes the files but doesn't sign them. To sign other files, use `cargo run -p xtask -- sign <file>`.

### Releases

GitHub Actions builds each push to `master` and each pull request into it. There are two kinds of release:

- Development Build: each push to `master` replaces the `latest` release. Its notes list the new commits, one per line, as `- <hash> <subject>`. The update check on the dev channel reads these lines, so don't change their format.
- Stable releases: to publish one, set the version in `Cargo.toml` and commit. Then either push a tag with the same version, for example `v0.1.0`, or run the Release whatSnap workflow on `master` from the Actions tab, which makes the tag for you. The workflow checks that the tag and `Cargo.toml` agree. It publishes the release with no notes, so write the changelog on the release page afterwards.

Both workflows need the `MINISIGN_KEY` and `MINISIGN_PASSWORD` secrets.

## Before Committing

Run the formatter before you commit changes:

```batch
cargo +nightly fmt
```

Set up the repository pre-commit hooks from the repo root:

```batch
cargo install prek
prek install
```

Run the same checks that we expect:

```batch
cargo +nightly fmt --all --check
cargo clippy --workspace --all-targets
cargo test --workspace
cd backend
go vet ./...
go test ./...
```

The backend tests use Windows-only packages, so run them on Windows. On other systems, you can still check the backend with `GOOS=windows go vet ./...`.

## Testing on a Real Phone

Many features need a test with a real WhatsApp account before we know that they work, for example calls and settings that sync with your phone. When you add or change one of these features, say in the commit message that it is not yet tested live. The User Manual lists the features that still wait for a live test.

## License

whatSnap is two programs with different licenses:

- `whatSnap.exe` and `whatSnapShare.exe` are licensed under the [MIT License](LICENSE).
- `backend.exe` is licensed under the [GNU General Public License version 3](backend/LICENSE), because it links libsignal, which is GPLv3.

For the licenses of the libraries that whatSnap uses, see [Third-Party Notices](THIRD-PARTY-NOTICES.md).
