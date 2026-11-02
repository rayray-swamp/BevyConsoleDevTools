# BevyConsoleDevTools

Lets an AI agent (or you) drive a Bevy 0.20 game from the shell — no screen-control tools,
no OS permissions: scripted input, PNG frame capture, and reflected state dumps.

Input is **virtual**: it is injected into Bevy only. The OS cursor never moves and nothing
reaches other apps, so you can keep using your machine while a run is in progress.

## Integrate

```toml
[dependencies]
bevy_console_devtools = { git = "https://github.com/rayray-swamp/BevyConsoleDevTools", optional = true }

[features]
devtools = ["dep:bevy_console_devtools"]
```

```rust
#[cfg(feature = "devtools")]
app.add_plugins(bevy_console_devtools::ConsoleDevToolsPlugin);
```

Types you want to `dump` need `#[derive(Reflect)]` + `#[reflect(Component)]` / `#[reflect(Resource)]`.
Requires rustc 1.97.1+ (Bevy 0.20.1); this repo pins it in `rust-toolchain.toml`.

## Batch mode

```bash
BCDT_SCRIPT=scenarios/demo_drag.jsonl BCDT_OUT=out cargo run --features devtools
```

One JSON command per line (`#` comments allowed). Outputs: `out/frames/*.png`,
`out/events.jsonl`, `out/dump/<frame>.json`. Exit code: 0 ok, 1 runtime error / `BCDT_MAX_FRAMES`
(default 36000), 2 bad script.

In batch mode the game sees **only** the script's input: real mouse/keyboard events are
discarded every frame, so runs are deterministic even if you touch the window.

## Interactive mode

Run without `BCDT_SCRIPT`; send the same commands as BRP methods on `localhost:15702`:

```bash
curl -s localhost:15702 -d '{"jsonrpc":"2.0","id":1,"method":"devtools.click","params":{"x":300,"y":200}}'
```

Responses are `{frame, queued, path}` (`path` = PNG for `screenshot`). BRP builtins
(`world.query`, `world.get_resources`, …) work too. Real input keeps working here: an injected cursor holds until you move the real mouse. A `dump` of an unknown type returns a JSON-RPC error immediately.

## Commands

Coordinates are window logical pixels, origin top-left. Time advances a fixed 1/60 s per frame.

| cmd | params | frames |
|---|---|---|
| `move` | `x, y` | 0 |
| `down` / `up` | `button?` (`left`/`right`/`middle`) | 0 |
| `click` | `x, y, button?` | 1 |
| `drag` | `from:[x,y], to:[x,y], frames, button?` | frames+1 |
| `key` | `key` (Bevy `KeyCode` name, e.g. `Space`, `KeyA`), `action` (`press`/`release`/`tap`) | 0 (tap 1) |
| `wait` | `frames` | frames |
| `pause` / `resume` | – | 0 |
| `step` | `frames` (runs N frames, then pauses) | frames |
| `screenshot` | `name?` | 0 |
| `record` | `every, frames` (`frames:0` = until quit) | frames |
| `dump` | `resources?:[type path], components?:[type path]` | 0 |
| `mark` | `label` | 0 |
| `quit` | – | – |

## Caveats

- Injected keys carry no text: games reading `ButtonInput<KeyCode>` work, text fields don't.
- Adding this plugin also adds `RemotePlugin` in interactive mode; don't add it twice.
- Needs a window (macOS/desktop). Offscreen capture is planned.
