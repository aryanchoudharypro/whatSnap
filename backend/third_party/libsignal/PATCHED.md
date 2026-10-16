# Patched copy of go.mau.fi/libsignal v0.2.2

Source: https://github.com/tulir/libsignal-protocol-go (module `go.mau.fi/libsignal`),
used through `replace` in `backend/go.mod`. Licensed under the GPLv3 (see LICENSE).

Change from upstream v0.2.2: `logger.Debug` returns straight away when the default
logger is in use, since that logger discards debug messages. Before, every call
walked the call stack and formatted its arguments only to throw them away. That
was about 12% of CPU and 14% of allocations when encrypting a group message.

To update: copy the new upstream release over this directory and re-apply the change
in `logger/Logger.go`.
