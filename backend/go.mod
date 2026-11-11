module whantv-backend

go 1.27.3

require (
	github.com/Microsoft/go-winio v0.6.2
	github.com/purpshell/meowcaller v0.0.0-20260724091720-56f599712aaa
	github.com/rs/zerolog v1.35.1
	go.mau.fi/util v0.11.0
	go.mau.fi/whatsmeow v0.0.0-20260814123134-0dcf1f50f4b1
	golang.org/x/crypto v0.58.0
	golang.org/x/image v0.47.0
	golang.org/x/sys v0.49.0
	google.golang.org/protobuf v1.36.12
	modernc.org/sqlite v1.53.0
)

require (
	filippo.io/edwards25519 v1.2.0 // indirect
	github.com/beeper/argo-go v1.1.2 // indirect
	github.com/coder/websocket v1.8.15 // indirect
	github.com/dustin/go-humanize v1.0.1 // indirect
	github.com/elliotchance/orderedmap/v3 v3.1.1 // indirect
	github.com/google/uuid v1.6.0 // indirect
	github.com/hajimehoshi/go-mp3 v0.3.4 // indirect
	github.com/kazzmir/opus-go v1.6.0 // indirect
	github.com/mattn/go-colorable v0.1.15 // indirect
	github.com/mattn/go-isatty v0.0.24 // indirect
	github.com/ncruces/go-strftime v1.0.0 // indirect
	github.com/petermattis/goid v0.0.0-20260918085751-abfca077860b // indirect
	github.com/pion/datachannel v1.6.2 // indirect
	github.com/pion/dtls/v3 v3.1.5 // indirect
	github.com/pion/logging v0.2.4 // indirect
	github.com/pion/opus v0.1.0 // indirect
	github.com/pion/randutil v0.1.0 // indirect
	github.com/pion/sctp v1.11.1 // indirect
	github.com/pion/transport/v4 v4.1.0 // indirect
	github.com/remyoudompheng/bigfft v0.0.0-20230129092748-24d4a6f8daec // indirect
	github.com/vektah/gqlparser/v2 v2.5.36 // indirect
	go.mau.fi/libsignal v0.2.2 // indirect
	golang.org/x/exp v0.0.0-20261014143641-f0251eb4aa3e // indirect
	golang.org/x/net v0.61.0 // indirect
	golang.org/x/sync v0.24.0 // indirect
	golang.org/x/text v0.43.0 // indirect
	modernc.org/libc v1.73.4 // indirect
	modernc.org/mathutil v1.7.1 // indirect
	modernc.org/memory v1.11.0 // indirect
)

replace go.mau.fi/whatsmeow => github.com/aryanchoudharypro/whatsmeow v0.0.0-20261111084805-c76639ad8ae5

replace github.com/purpshell/meowcaller => github.com/aryanchoudharypro/meowcaller v0.0.0-20261107173757-b33b813e4b2e

replace go.mau.fi/libsignal => ./third_party/libsignal
