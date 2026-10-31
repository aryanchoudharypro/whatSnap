# whatSnap

whatSnap is a lightweight, fast, and accessible WhatsApp client for Windows. It is designed to be completely usable with screen readers and the keyboard. It does not use a browser or WhatsApp Web in the background. It links to your phone as a WhatsApp companion device, the same way WhatsApp Web and WhatsApp for Windows do.

whatSnap is in development. There are no numbered releases yet. Each change to `master` builds a new Development Build.

## Documentation

For a comprehensive user guide, including a full list of features and hotkeys, please see the [User Manual](doc/readme.md).

## How whatSnap Is Built

whatSnap has two programs:

- `whatSnap.exe` is the user interface. It is written in Rust with [wxDragon](https://github.com/AllenDang/wxDragon), so all controls are native Windows controls. The message list is a custom control that uses [AccessKit](https://accesskit.dev).- `backend.exe` connects to WhatsApp. It is written in Go, with a fork of [whatsmeow](https://github.com/aryanchoudharypro/whatsmeow) for the protocol and a fork of meowcaller for calls. It keeps the session and the message history in a SQLite database.

`whatSnap.exe` starts `backend.exe` and talks to it through named pipes, with one JSON message per line. During a call, the microphone and speaker audio goes between the two programs over a local UDP socket instead. A third, small program, `whatSnapShare.exe` in `share-target/`, adds whatSnap to the Windows Share menu.

Some internal names still say `whantv`, the old name of the app. These include the pipe names, the single instance mutex, registry values, and the Go module name. We keep them on purpose, so that an old and a new build can't run at the same time and no registry data has to move. Don't rename them. Text that users see must say whatSnap.

## Building

whatSnap builds only on Windows. To build, you'll need:

- Rust `1.88.0` or later, with the MSVC toolchain.
- Go, at the version in `backend/go.mod`.
- CMake and Ninja, for building wxDragon. In addition, you also need LLVM, from LLVM.org.
- The Windows SDK, for `makeappx`. The Visual Studio C++ tools install it.
- Inno Setup, only for the installer.

```batch
set CMAKE_POLICY_VERSION_MINIMUM=3.5
cargo build --release
```

`build.rs` also runs `go build` in `backend/`, so this one command makes both programs. It generates `target/release/whatSnap.exe` and `target/release/backend/backend.exe`. Keep the two together: `whatSnap.exe` looks for the backend in the `backend` folder next to it.

### Release Packages

`packaging\build-release.ps1` builds everything that a release needs:

- `dist\whatSnap-<version>-setup.exe`, the installer.
- `dist\whatSnap-<version>-portable.zip`, the portable version.

The script reads the version from `Cargo.toml`. To set it yourself, use `-Version 0.2.0`. To build only the zip, use `-SkipInstaller`. To sign the files, give a sign command with `-SignCommand`. The script replaces `{0}` in the command with the path of each file.

### Continuous Integration

GitHub Actions builds each push to `master` and each pull request into it. A push to `master` also replaces the `latest` release, named Development Build, with the new installer and zip.

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
