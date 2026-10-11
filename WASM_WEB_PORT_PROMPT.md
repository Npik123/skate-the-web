# Prompt: port the Skate 3 Rust Engine to WebAssembly + a web server

Paste everything below the line into a coding LLM (Claude Code, etc.) with the repo as its working directory.

---

## Role and goal

You are a senior Rust/WebAssembly/graphics engineer. Port this repository, a Bevy 0.18.1 reimplementation of Skate 3, so it runs in a browser as `wasm32-unknown-unknown` with WebGPU (WebGL2 only as a documented fallback), and add a small web server that serves the static client and any multiplayer or relay endpoints. The desktop Windows build must keep working. Do not break `cargo test --workspace` on desktop.

## Definition of done: the real Skate 3

The target is the **actual retail game experience** running in the browser, not a tech demo, a sandbox, or a placeholder-asset build. That means the real skater and customiser, the real disc maps (University and the rest), the real animations, tricks, grinds, offboard movement, scoring HUD and session markers, retail materials, sky, lighting and tone mapping, and the real sound and music if the engine has them. All of it comes from the user's own ISO, converted in their browser. The `format-demo.skate` map is only a smoke-test fixture for early phases. Do not stop at it, and do not call anything finished until the retail maps play end to end in the browser with the retail look. Anything the desktop engine does today must work on the web, and anything the desktop engine does not yet do is not your job to invent.

## Use subagents heavily

You are the orchestrator. Spawn as many subagents as the work needs, and run independent ones **in parallel** (several Agent calls in one message). Do not do large reads or long builds yourself when a subagent can do them. Guidelines:

- **Phase 0:** launch one Explore or research subagent per area at the same time (`skate-core`, `skate-data`, `skate-dynamics`, `skate-mods`, `skate-net`, `skate-game` input/IO/process/thread use, renderer and WGSL, vendored Bevy, Python asset pipeline, `mlua`/`zstd`/`rapier` wasm viability). Each returns a compatibility table and file:line findings. You merge them into the plan.
- **Implementation:** give each subagent one narrow, self-contained task with exact files, the interface it must keep, and the check it must pass. Good splits: clock and fs abstraction; storage layer (OPFS); Gamepad/keyboard input transport; `AssetReader`; each asset-pipeline stage port (ISO/XEX reader, refpack, VLT, texture decode, character GLB, map builder); each shader/pipeline group; HUD VM; audio; multiplayer transport; server crate; CI. Use `isolation: "worktree"` for subagents that touch overlapping files, and merge their results yourself.
- **Verification:** use separate subagents to build, run native tests, run wasm tests, and drive a real browser (WebGPU) for screenshots and input checks. Spawn independent reviewer subagents (`/code-review` style) after each phase to look for native regressions, determinism drift and legal-constraint violations.
- **Hard problems:** when a subagent gets stuck on something like Pyodide versus a Rust port of the importer, `mlua` on wasm, or a WebGPU limit, spawn two or three subagents to try different approaches in parallel and keep the one that works.
- Each subagent prompt must be self-contained (they start with no context): repo path, the goal, the legal constraint, the files to read, what to return. Make them report what they verified, with command output, not what they hope works.
- Don't take a subagent's "done" on trust. Confirm with a build, a test or a browser run before moving on.

## Hard legal and design constraint (read first)

This engine ships **no Skate 3 assets**. They are EA's copyrighted data, and the desktop flow has the user supply their own Xbox 360 ISO or `default.xex`, which `tools/setup.py` and `tools/asset_pipeline/` convert locally into a `data/` folder. The web version must keep that model:

- The server must **never host, bundle, cache or redistribute** converted or original Skate 3 assets.
- The user provides their own ISO or extracted game folder in the browser (file picker or drag-and-drop). Conversion runs client-side (Web Worker + WASM).
- Converted output is stored in the user's own browser storage (OPFS preferred, IndexedDB fallback), keyed by asset-pipeline version so it can be refreshed incrementally, like `tools/asset_pipeline/versions.py`.
- The server may serve only the engine (wasm/js/html), the original procedural `maps/format-demo.skate`, mod examples already in the repo, and multiplayer signalling or relay.
- Add a short in-page notice that the site is unofficial, is not affiliated with EA, and needs the user's own copy of the game.

If a step seems to need server-side assets, stop and redesign it client-side.

## Repo orientation (verified)

Cargo workspace (edition 2024), crates:

| Crate | ~LOC | Role | Web-port notes |
|---|---|---|---|
| `skate-core` | 90k | Pure gameplay logic: physics, air/grind/riding, animation, camera, input, graph, scoring. Zero dependencies. | Should compile to wasm as is. Audit `std::time::Instant` and `SystemTime` uses (found in `input/controller.rs`, `player/state.rs`, `physics/tests/*`). Replace them with an injected clock. Verify float determinism, since it is plain Rust f32. |
| `skate-data` | 6k | Parsers for `.skate` maps, state graphs, collision (`retail_collision`), animation banks, flate2/zstd. | `zstd` (C binding) is a wasm blocker. Swap to a pure-Rust decoder (`ruzstd`) behind a cfg or feature, or compile zstd with a wasm-capable C toolchain. `std::fs` use is in `collections.rs`, `gesture_patterns.rs`, `input_recording.rs`, `physics_skeleton.rs`, `scoring.rs`. Abstract it so data can load from in-memory or OPFS bytes. |
| `skate-dynamics` | 4k | `rapier3d 0.35.3` wrappers for mod physics. | Rapier supports wasm. Check the `Instant` use in `deformation.rs` and `visual_contact.rs`. |
| `skate-mods` | 6k | Lua 5.4 mod runtime: `mlua` with `vendored`, plus `zip` and `gltf`. | `mlua` vendored Lua needs a C compiler targeting wasm (clang plus a wasi-sysroot or emscripten shim). Try this first. Otherwise gate mods off on wasm and document it. A pure-Rust Lua is a possible later step, not a prerequisite. |
| `skate-net` | 4k | Multiplayer protocol: `socket2`, blake3, half. | `socket2` UDP does not exist in browsers. Put the transport behind a trait. Add a WebSocket relay (served by the new server) and optionally a WebRTC data-channel transport. Keep the packet formats unchanged. |
| `skate-steam-relay` | 0.5k | `steamworks` raw-bindings relay. | Desktop only. Exclude from the wasm build. |
| `skate-game` | 89k | The Bevy app. Bins `skate3rust`, `skate3-multiplayer`, `skate3-custom-models`. `main.rs` plus about 80 modules: renderer, HUD (the `apt_*` Flash-style VM), customiser, setup, updater, crash reporter, replay. | The main work. See below. |

Other relevant parts:

- `vendor/bevy_pbr`, `vendor/bevy_core_pipeline`: patched Bevy crates wired through `[patch.crates-io]`. Keep them compiling for wasm. Check their WGSL and features.
- `crates/skate-game/src/*.wgsl` and `retail_*.rs`: custom shaders and render pipeline (`retail_world`, `retail_character`, `retail_sky`, `retail_tone`, `retail_exposure`, `retail_irradiance`, `customiser_*`, `hud_*`). There is a naga/`naga_oil` shader test in dev-dependencies. Validate every shader against WebGPU limits.
- `crates/skate-game/src/app.rs` sets `AssetPlugin { file_path: config.asset_root }`, which is filesystem-based. Replace it with an `AssetReader` backed by OPFS or in-memory bytes on wasm.
- `tools/setup.py` plus `tools/asset_pipeline/*.py` (about 10k lines of Python, plus `refpack_native.rs`) convert the ISO to game assets. This is the biggest porting question (see Phase 3).
- `crates/skate-game/src/input/platform.rs` is a **Windows XInput-only** transport that feeds raw signed axis and trigger bytes to `skate_core::input::xbox::XboxState`. Browsers expose the Gamepad API instead.
- `sdk/`, `mods/`, `docs/physics/`: Lua mod SDK, examples, and physics notes. Read `sdk/ENGINE_API.md` and `docs/physics/*.md` before touching the physics or input.
- `.github/workflows/release.yml` and `scripts/*.ps1` are the Windows release pipeline. Leave them alone, and add a separate web workflow.

## Known desktop-only things to replace or gate with `#[cfg(target_arch = "wasm32")]`

Gate or replace each, and keep desktop behaviour identical:

1. **Bevy features.** The default feature `dev-dynamic` enables `bevy/dynamic_linking`, which is invalid on wasm. The wasm build must use `--no-default-features`. Add a `web` feature profile. Enable `webgpu` (and optionally `webgl2`) on Bevy. Review `trace`, `debug`, `bevy_audio`, `wav` and `jpeg`. Audio needs a wasm-compatible backend, and the browser needs a user-gesture AudioContext resume.
2. **Process spawning (`std::process::Command`).** Used in `setup.rs` (Python setup helper), `updater.rs`, `custom_models.rs`, `crash_report.rs` (also `powershell.exe`), `multiplayer/transport.rs` (steam relay helper) and `modding/menu.rs`. None work in browsers. Gate them, and replace setup with the in-browser flow below. Replace the updater with normal static-site versioning and cache-busting. Drop or reimplement the crash reporter (console plus an optional `POST` to the server).
3. **Threads.** `std::thread::spawn` and sleeps (`crash_report.rs`, `custom_models.rs`). Use Bevy's wasm-compatible task pools. Only enable wasm threads (`SharedArrayBuffer`, atomics, `-C target-feature=+atomics,+bulk-memory,+mutable-globals`, rebuilt std via `-Zbuild-std`) if profiling shows it is needed. If it is, the server **must** send `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp`. Prefer single-threaded first.
4. **Filesystem (`std::fs`).** Used throughout `config.rs`, `map_library.rs`, `difficulty.rs`, `custom_difficulty.rs`, `graphics_menu.rs`, `customiser*.rs`, `modding/*` and others. Introduce a small storage abstraction (native: files, wasm: OPFS or IndexedDB, with settings in localStorage) and route all of these through it.
5. **Time.** `std::time::Instant` and `SystemTime` panic on `wasm32-unknown-unknown`. Use `web-time`, Bevy's `Time`, or an injected clock.
6. **Input.** Add a Gamepad API transport that produces the same `XboxState` and `DevicePacket` as XInput. The raw-byte semantics (no extra deadzone or normalisation beyond the original conversion) must match `platform.rs`. Map standard-mapping buttons and axes, and document the mapping. Also add keyboard and mouse bindings so the game is playable without a controller. Check `gameplay_map.rs` and `gesture.rs` in `skate-core::input` before choosing the mapping. Keep the touch overlay out of scope unless time allows.
7. **Windowing.** Canvas element, resize observer, pointer lock for debug-cam or look, fullscreen API, `devicePixelRatio`.
8. **Memory.** The wasm32 address space is 4 GB, and browsers often cap lower. Map textures and the retail character pipeline can be large. Stream and decode lazily, and free CPU copies after GPU upload. Check `render_capacity.rs`.
9. **Performance.** Profile with `--trace`-style timeline capture ported to `performance.mark`/`measure`. Target 60 fps at 1080p on a mid-range discrete GPU. Report what is not achievable.
10. **Panics and logging.** `console_error_panic_hook`, `tracing-wasm` or the Bevy log plugin for wasm.

## Phases (do them in order; stop and report after each)

**Phase 0: Survey and plan.** Read the files above. Produce `docs/web-port-plan.md` with a dependency-by-dependency wasm compatibility table, risks, and a go/no-go on `mlua`, `zstd`, `rapier3d`, Bevy WebGPU with the vendored patches, and the memory budget. Do not skip this, and do not start coding before the table exists.

**Phase 1: Core compiles for wasm.** `skate-core`, `skate-data` and `skate-dynamics` build for `wasm32-unknown-unknown` (`cargo check --target wasm32-unknown-unknown -p skate-core -p skate-data -p skate-dynamics`). Fix the clock and filesystem issues. Add `wasm-bindgen-test` or `wasm-pack test --headless` smoke tests that parse `maps/format-demo.skate` and step the physics, and check results match native within a documented tolerance.

**Phase 2: Engine boots in the browser.** `skate-game` builds with a `web` profile and the whole Bevy app runs on the canvas, smoke-tested on `maps/format-demo.skate` (original procedural content) with a controllable skater. This is a checkpoint only: the real game comes from Phase 3 onward, and no placeholder assets ship in the final build. Put in an asset abstraction (`AssetReader`) and the storage abstraction. Ship a `trunk` or `wasm-bindgen` plus custom `index.html` build, and `wasm-opt -O3`. Report the wasm and JS size, and gzip/brotli sizes.

**Phase 3: User-supplied asset import in the browser.** This is the hardest part. Evaluate and pick one, with justification in the plan doc:
  - (a) Run the existing Python pipeline in a Web Worker via **Pyodide** (it already contains `refpack_native.rs`, which would need a wasm build), keeping the logic as is. Slowest but least porting.
  - (b) Port the pipeline stages to Rust in wasm (XEX/ISO reading, refpack decompression via `fast_refpack`/`refpack_native.rs`, VLT, texture decode in `skate-data/src/skate_map/texture_decode.rs`, `character_glb`, `build_map`, `map_writer`) into a worker, reusing `skate-data` parsers where possible.
  - (c) A hybrid: Rust/wasm for hot paths (refpack, textures), Pyodide for orchestration.
  The importer needs: file picker and drag-drop for an ISO (multi-GB, so **stream with `File.slice`, never load it whole**) or a directory (`showDirectoryPicker`, with `webkitdirectory` fallback), progress UI, cancel and resume, receipts and versioning matching `tools/asset_pipeline/group_receipts.py` and `versions.py`, and a "delete my data" button. Note that the desktop setup states ISO extraction needs internet access, so find out why in `tools/asset_pipeline/install.py` and `tools/setup.py`. If it downloads a tool or data, say exactly what and from where. Do not silently add a server-side download or proxy for it.

**Phase 4: Retail game running.** After import, the user picks University (and every other disc map) and plays the real game: retail skater and customiser, animations, HUD, session markers, sky and lighting, map switching without a reload, and audio. Fix everything that stops this.

Then the full rendering parity pass. Make the retail renderer work on WebGPU: custom pipelines, the HUD (`apt_vm`, `hud_render.wgsl`, `hud_composite.wgsl`), sky, tone mapping, exposure, irradiance, customiser. Fix limit issues (bind groups, storage buffers, texture formats such as BC/ASTC support detection, and so on). Provide a documented fallback when a feature is missing. Take comparison screenshots against desktop.

**Phase 5: Server.** A small Rust (`axum`) or Node server, in a new crate `crates/skate-web-server` (excluded from the wasm build) that:
  - serves the static client with correct MIME types (`application/wasm`), brotli/gzip, immutable cache headers for hashed files, and COOP/COEP headers if threads are used;
  - exposes `/healthz`;
  - hosts the **multiplayer WebSocket relay** speaking the existing `skate-net` packet formats (room codes, join/leave, simple rate limits, max message size, no persistence of game data);
  - never stores user assets, and logs no personal data;
  - has a Dockerfile and a documented `cargo run -p skate-web-server`.
  Wire the `skate-net` transport trait to the WebSocket client in wasm. Single-player must work with no server beyond static hosting.

**Phase 6: Mods and extras (optional, after the above).** Lua mods via `mlua` on wasm if it proved viable, loaded from user-uploaded `.zip` (`skate-mods` already uses `zip`). The sandbox must not give mods network or filesystem access beyond the SDK.

**Phase 7: CI and docs.** Add `.github/workflows/web.yml` (do not touch `release.yml`): build wasm, run wasm tests, publish the static bundle as an artifact. Update `README.md` with a Web section, browser requirements (Chrome/Edge with WebGPU first; Firefox and Safari status), and the legal notice. Add `docs/web-port.md`.

## Engineering rules

- Keep desktop behaviour byte-for-byte the same. Use `cfg` gates and small trait seams, not rewrites. Run `cargo test --workspace` on desktop after each phase.
- Don't change `skate-core` physics math to "fix" wasm issues. If native and wasm results diverge, find out why (usually time or float formatting) and report it.
- Pin versions the way the workspace already does (`=x.y.z`). Justify every new dependency.
- No `unsafe` unless unavoidable, and comment it when used.
- Match the existing code style (very few comments, `skate_*` naming, edition 2024).
- Don't invent features. Where the original game's behaviour is unknown, leave it and say so.
- After every phase give: what changed, how to run it, what was verified (with commands and output), what is still broken, and the next step. Be honest about failures. Do not claim parity you have not measured.
- Use a real browser (headless Chrome with WebGPU, or the browser pane) to verify rendering and input. Don't claim visual correctness from a compile alone.

## Deliverables

1. `docs/web-port-plan.md` (Phase 0) and `docs/web-port.md` (final).
2. A `web` build (script plus `index.html`) that outputs `dist/`.
3. `crates/skate-web-server` with the Dockerfile.
4. The in-browser asset importer and OPFS storage layer.
5. Gamepad, keyboard and mouse input, with documented mappings.
6. A web CI workflow and wasm tests.
7. A final report: what works, what doesn't, performance and memory numbers, bundle sizes, and browser support matrix.

Start with Phase 0 now.
