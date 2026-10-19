# S2-style deterministic simulation testing (DST) of piton's async Rust IO shell

Research date: 2026-10-18. Primary sources were read directly. I shallow-cloned and read these repos: `tokio-rs/turmoil` at HEAD 4899894 (2026-10-16), `s2-streamstore/s2` at HEAD (2026-10-16), `s2-streamstore/mad-turmoil` v0.2.1, `s2-streamstore/s2-verification`, and `madsim-rs/madsim` at HEAD (2026-02-16, v0.2.34). I also read files from `s2-streamstore/opbox` and `MaterializeInc/materialize` through the GitHub API. Tokio docs are docs.rs tokio 1.53.3.

Single most useful finding: S2 open-sourced a complete, small DST harness. It is the `sim/` workspace in `github.com/s2-streamstore/s2`, about 1,200 LOC, and covers turmoil, mad-turmoil, the meta test, a Porcupine history and CI. It can be copied nearly verbatim as a template for piton's shell.

---

## 1. S2's published DST/testing posts and exact techniques

### Takeaway
S2 has two DST posts: "Deterministic simulation testing for async Rust" (2025-04-02) and "Linearizability testing S2 with deterministic simulation" (2025-08-25). The blog index lists no later testing posts through 2026-10. The recipe has six parts:
- turmoil hosts on a single-threaded, paused-clock tokio runtime, with tokio's RNG seeded via `--cfg tokio_unstable`
- mad-turmoil libc interposition of `getrandom`, `getentropy`, `CCRandomGenerateBytes` and `clock_gettime`
- a patched `getrandom` crate
- in-memory emulators of dependencies, run as turmoil hosts reached over the simulated network
- a "meta" test that runs the same seed twice at `RUST_LOG=trace` and requires byte-identical output
- client histories checked offline by Porcupine for linearizability, including `match_seq_num` and fencing tokens

### Cited Findings

**The blog posts (complete list as of 2026-10)**
- The blog index lists only two testing-related posts: "Deterministic simulation testing for async Rust" (2025-04-02, `/blog/dst`) and "Linearizability testing S2 with deterministic simulation" (2025-08-25, `/blog/linearizability`). Later posts through 2026-10-07 ("Redis clients with S2") cover product and features, not testing. — [S2 blog index](https://s2.dev/blog)

**DST post (Shikhar Bhushan, 2025-04-02)**
- DST runs on every PR and commit, plus "thousands of nightly trials". Each run charts a random path through the state space from a single RNG seed. — [S2 DST post](https://s2.dev/blog/dst)
- The post names four sources of nondeterminism to control:
  - execution: single-threaded scheduling
  - entropy: all RNGs seeded
  - time: no physical clocks
  - I/O: no external I/O outside the simulation

  — [S2 DST post](https://s2.dev/blog/dst)
- Tokio settings:
  - single-threaded scheduler, with the clock "paused" so time advances only on `sleep()`
  - code must use `tokio::time::Instant`, not `std::time::Instant`
  - tokio's internal RNG, which drives the `select!` branch choice, is seeded

  — [S2 DST post](https://s2.dev/blog/dst)
- Each networked service runs as one or more turmoil hosts. turmoil's `TcpListener`/`TcpStream` are used behind a compile-time feature in DST mode. Metadata and object-storage dependencies have in-memory emulators, which also run as hosts reached over the simulated network. — [S2 DST post](https://s2.dev/blog/dst)
- Faults: turmoil injects "hardship", such as latency and crashed processes, through the simulated network. — [S2 DST post](https://s2.dev/blog/dst)
- Assertions:
  - liberal defense-in-depth checks in mainline code
  - expensive checks use `debug_assert!`
  - client-visible invariants, such as durability after a crash and concurrency semantics, are also checked from outside

  — [S2 DST post](https://s2.dev/blog/dst)
- Residual nondeterminism, found by diffing TRACE logs:
  - timestamps in HTTP packets
  - `HashMap`'s random `RandomState`: S2 could seed it in its own code, but not in dependencies
  - third-party crates that spawn threads, use RNGs, read system time or do I/O

  The fix was mad-turmoil (madsim-derived libc overrides). — [S2 DST post](https://s2.dev/blog/dst)
- Meta test: CI reruns the same seed and compares TRACE-level logs, reporting byte-for-byte identical wire traffic. A failing CI seed reproduces on a developer's Mac. — [S2 DST post](https://s2.dev/blog/dst)
- Bugs found: a running doc lists 17 notable findings, including nuances of external APIs, internal protocol issues and plain deadlocks. HN discussion mentions distributed deadlocks and caching/eventual-consistency paths. — [S2 DST post](https://s2.dev/blog/dst); [HN thread](https://news.ycombinator.com/item?id=43581900)

**Linearizability post (Stephen Balogh, 2025-08-25)**
- History logging: concurrent clients (tokio tasks) emit a Start event and a Finish event per op, paired by `client_id` and `op_id`. Examples:
  - an append Start records `num_records` and `last_record_xxh3`
  - a Finish records `AppendSuccess { tail }`

  — [S2 linearizability post](https://s2.dev/blog/linearizability)
- Porcupine model:
  - the stream is modelled as a register `(tail, last_record_hash)`
  - append is legal iff `new_tail == prev_tail + batch_size`
  - read and check-tail must match the current tail
  - `match_seq_num` acts as CAS: the append succeeds only if the tail matches
  - fencing tokens, set via a `fence` command record, are added to the register state

  — [S2 linearizability post](https://s2.dev/blog/linearizability)
- Indefinite failures (e.g. a timeout) are modelled with Porcupine's `NondeterministicModel`: there are two possible next states. Gotcha: letting a client continue after an indefinite failure produced false violations. The fix is to treat that client as unsafe to continue and set the op's end time after all other operations. — [S2 linearizability post](https://s2.dev/blog/linearizability)
- The same model is run on Antithesis workloads. The post reports no confirmed S2 linearizability bugs, only a model flaw. — [S2 linearizability post](https://s2.dev/blog/linearizability)

**s2-verification README (current)**
- The model has been strengthened. State is now `(tail, cumulative hash, fencing token)`, with `hash(stream + record) = xxh3_64_with_seed(le_bytes(xxh3_64(record.body)), seed = hash(stream))`.
- Reads fold the chain from seq 0. A lost, reordered or corrupted record anywhere therefore fails the check.
- Workflows: `regular`, plus match_seq_num and fencing clients.

— [s2-verification README](https://github.com/s2-streamstore/s2-verification)

**Open-source harness: `s2/sim/` (read in full)**
Source: [s2/sim](https://github.com/s2-streamstore/s2/tree/main/sim)

- `sim/Cargo.toml`:
  - a separate cargo workspace, "to isolate its patched dependencies"
  - deps: `turmoil = "0.7.2"`, `mad-turmoil = "0.2.1"`, `fastrand`, `tokio 1.53` with features `rt, sync, time, macros, io-util` (no `rt-multi-thread` listed)
  - `[patch.crates-io] getrandom = { git = "https://github.com/s2-streamstore/getrandom", rev = "9634f37…" }` with feature `deterministic-simulation`. The comment says upstream getrandom 0.2 "invokes the raw SYS_getrandom syscall, which bypasses mad-turmoil's libc interposition". The fork routes through the interposable libc wrapper.
  - `[profile.sim]` inherits `release` with `lto=false`, `debug-assertions=true`, `overflow-checks=true`

- `sim/src/main.rs`:
  - refuses to run unless `cfg!(tokio_unstable)`
  - order of setup: `let _clocks_guard = mad_turmoil::time::SimClocksGuard::init(); mad_turmoil::rand::set_rng(StdRng::seed_from_u64(args.seed)); fastrand::seed(args.seed);`
  - Builder settings: `.rng_seed(seed).simulation_duration(4h).min_message_latency(2ms).max_message_latency(30ms).tcp_capacity(10_000).tick_duration(1ms).epoch(SystemTime::UNIX_EPOCH)`, plus optional `.fail_rate(f)`
  - hosts: `s3` (mock S3 via `s3s` over an in-memory `BTreeMap`) and `s2-lite` (the real server, with slatedb pointed at the mock S3)
  - the client `"workload"` runs a scenario
  - the seed comes from `--seed` or env `SIM_SEED`

- Tracing timer: a custom `FormatTime` (`SimStepTimeFormat`) prints `[s{turmoil::sim_elapsed ms} e{global event counter}]` instead of wall-clock. This is "a prerequisite for the determinism meta test".

- `sim/src/meta.rs`: re-execs the binary twice concurrently, each in its own temp dir. It compares stdout and stderr line by line in lockstep, kills both children at the first mismatch, then compares exit statuses and produced `history.*.jsonl` files.

- `sim/src/net.rs`:
  - `TurmoilConnector` implements `tower::Service<http::Uri>` (a hyper-util `Connect`) over `turmoil::net::TcpStream`, wrapped in `TokioIo`
  - it uses a 5 s simulated-time connect timeout, because "turmoil does not retransmit" a dropped SYN
  - `serve_hyper` serves hyper and axum on a `turmoil::net::TcpListener`
  - the real s2-sdk is injected via `S2::new_with_connector`

- `sim/src/scenarios`:
  - the workload uses `AppendRetryPolicy::NoSideEffects`, because default retries "re-send appends of unknown outcome, which can duplicate records"
  - the linearizable scenario runs `client`, `match_seq_num_client` and `fencing_token_client` (from s2-verification) round-robin across N clients
  - indefinite-failure events are deferred behind a `Barrier` until all clients finish

- `sim/src/history.rs`: drains `LabeledEvent`s to `history.<seed>.jsonl`.

**CI** — [s2 ci.yml](https://github.com/s2-streamstore/s2/blob/main/.github/workflows/ci.yml)
- Job `simulation` runs on `ubuntu-24.04-arm`. The reason given: "on x86_64 Linux, fastant (via slatedb -> foyer) runs a pre-main TSC calibration loop that spins forever under mad-turmoil's interposed clock_gettime".
- Settings: `timeout-minutes: 30` and `RUSTFLAGS=--cfg tokio_unstable`.
- Meta tests at `RUST_LOG=trace` ("~50MB vs a dozen lines per run"): `sim meta smoke --seed 1`, `sim meta linearizable --seed 1`, `sim meta linearizable --seed 2 --fail-rate 0.005`.
- Linearizability: seeds 1–3 clean and seeds 4–5 with `--fail-rate 0.005`, each with 3 clients × 50 ops, then `s2-porcupine -file=history.$seed.jsonl`. Histories are uploaded on failure.
- Public CI therefore uses only about 5 seeds. "Thousands" of seeds run nightly on the internal harness ([S2 DST post](https://s2.dev/blog/dst)).

**mad-turmoil v0.2.1 (read in full; ~200 LOC)** — [mad-turmoil](https://github.com/s2-streamstore/mad-turmoil)
- `rand`: a global `OnceLock<Arc<Mutex<StdRng>>>` set by `set_rng` (single init), plus `try_rng` and `get_rng`.
  - `#[unsafe(no_mangle)] extern "C" fn getrandom` fills from that RNG, falling back to `/dev/urandom` before init.
  - `getentropy` (rejects >256 bytes) and macOS `CCRandomGenerateBytes` delegate to it.
- `time`: `#[no_mangle] clock_gettime` applies only when the `SimClocksGuard` flag is set and `tokio::runtime::Handle::try_current()` is ok.
  - `CLOCK_REALTIME` and `REALTIME_COARSE` return `turmoil::sim_elapsed()`.
  - `CLOCK_MONOTONIC`, `MONOTONIC_RAW`, `MONOTONIC_COARSE`, `BOOTTIME` and macOS `UPTIME_RAW` return tokio's paused `Instant::now()`, by transmuting `tokio::time::Instant` to a timespec.
  - Outside a runtime it returns the Unix epoch, 0. The source comment names mimalloc, tracing and fastrand as callers that still hit this path.
  - The guard exists because `Handle::try_current()` during process teardown can SIGTRAP ("global allocator may not use TLS").
- README warning: depend on it "only for simulation binaries!"

**opbox (S2's newer project)**
- `opbox/sim/src/main.rs` uses `compile_error!` without `tokio_unstable` and the same `SimClocksGuard`/`set_rng`/`fastrand::seed` trio.
- Subcommands: `Single`, `Meta`, `Parallel { trials (default 100), jobs, output_dir }` and `Sweep` over all workloads.
- It uses explicit `turmoil::partition`, `partition_oneway` and `hold` per named workload.
- A `FaultInjectingFileIO` wraps an `InMemoryFileIO` that implements the app's `#[async_trait] FileIO` trait. It holds a queue of scripted `Fault` variants, e.g. `GuardedWriteConflictAfterSwap` and `GuardedWriteTempLeakAndFail`.

— [opbox sim](https://github.com/s2-streamstore/opbox/tree/main/sim/src)

### Inferences
- The "S2 way" has seven pieces:
  1. A separate sim binary or workspace, not `#[test]`s.
  2. Real production code wired to simulated transports via injected connectors.
  3. Dependencies as in-memory emulators, run as turmoil hosts so they can be partitioned or crashed independently.
  4. Seeds, fail-rate and workload set via CLI.
  5. A meta determinism test that gates everything else.
  6. Invariants checked both inline (asserts) and offline (history → checker).
  7. Nightly parallel seed sweeps.
- opbox shows the pattern S2 uses for trait-based storage (`#[async_trait]` behind `Arc`): an in-memory impl plus a fault-injecting decorator. This matches piton's `LogStore`/`ToolHost` traits directly.
- Piton's lints ban `debug_assert!` and unbounded channels. S2 relies on both, so piton should use `assert!` in a `profile.sim` with checks on, and bounded or Vec-collected history sinks.

### Gaps
- S2's internal harness (nightly counts, Antithesis setup, the list of 17 bugs) is not public. The public `sim/` covers s2-lite only.
- The contents of the `s2-streamstore/getrandom` fork were not read beyond the Cargo.toml comment.

---

## 2. turmoil's current API (verified against source, 2026-10-16)

### Takeaway
turmoil is now a 4-crate workspace:

| Crate | Version | Role |
|---|---|---|
| `turmoil` | 0.7.2 | umbrella |
| `turmoil-net` | 0.1.0 | standalone simulated socket stack |
| `turmoil-fs` | 0.1.0 | simulated filesystem; turmoil feature `unstable-fs` |
| `turmoil-io-uring` | 0.1.0 | simulated io_uring; turmoil feature `unstable-io_uring` |

`turmoil` itself internally runs each host on a `current_thread` tokio runtime plus a `LocalSet`, with `start_paused(true)`, and seeds tokio's RNG only under `cfg(tokio_unstable)`.

### Cited Findings
- Workspace layout and README: "injects 'hardship' — latency, drops, partitions, crashes, torn writes — into the simulated network and filesystem". `turmoil-net` is a "Drop-in replacement for `tokio::net`" and `turmoil-fs` for "`std::fs` / `tokio::fs`". — [turmoil README](https://github.com/tokio-rs/turmoil)
- Features of `turmoil` 0.7.2 (`crates/turmoil/Cargo.toml`): `regex`, `unstable-fs`, `unstable-io_uring`, `unstable-barriers`. The unstable ones are "not covered by semver" (repo `CLAUDE.md`). `turmoil` 0.7.2 does not depend on `turmoil-net`; it has its own `net` module. — [turmoil Cargo.toml](https://github.com/tokio-rs/turmoil/blob/main/crates/turmoil/Cargo.toml)
- `Builder` (`crates/turmoil/src/builder.rs`) methods:
  - `new`, `epoch(SystemTime)`, `simulation_duration`, `tick_duration`, `ip_version`
  - `min_message_latency`, `max_message_latency`
  - `fail_rate(f64)`, `repair_rate(f64)`
  - `ephemeral_ports`, `tcp_capacity`, `udp_capacity`
  - `enable_tokio_io()`, `enable_random_order()` ("running of nodes in random order… race conditions… message send/receive order")
  - `fs() -> &mut FsConfig` (with `unstable-fs`)
  - `rng_seed(u64)`, `build()`

  Older docs mention `build_with_rng`, but it is not present in the current source. — [builder.rs](https://github.com/tokio-rs/turmoil/blob/main/crates/turmoil/src/builder.rs)
- `Sim` (`sim.rs`) methods:
  - `client(addr, fut)` (the test entry point; the sim ends when the clients finish) and `host(addr, || fut)` (software as a restartable factory)
  - `crash(addrs)`, `bounce(addrs)` (restart the software), `is_host_running`
  - `hold`, `release`, `partition`, `partition_oneway`, `repair`, `repair_oneway`
  - `set_max_message_latency`, `set_link_latency`, `set_link_max_message_latency`, `set_message_latency_curve`, `set_fail_rate`, `set_link_fail_rate`
  - `links(|LinksIter|)` to introspect in-flight messages
  - `run()`, `step() -> Result<bool>` for manual stepping and interleaving faults between steps
  - `elapsed`, `since_epoch`

  Free functions callable from inside hosts: `turmoil::{partition, repair, hold, release, partition_oneway, repair_oneway, lookup, in_simulation, elapsed, sim_elapsed, since_epoch, established_tcp_stream_count[_on]}`. — [sim.rs](https://github.com/tokio-rs/turmoil/blob/main/crates/turmoil/src/sim.rs), [lib.rs](https://github.com/tokio-rs/turmoil/blob/main/crates/turmoil/src/lib.rs)
- Crash semantics:
  - `Sim::crash` drops the host's runtime. With `unstable-fs` it also calls `host.fs.crash()`, which discards unsynced writes.
  - Data synced via `sync_all()` survives `bounce`.
  - Writes become durable on `sync_all()` or randomly per `FsConfig::sync_probability`.

  — [sim.rs](https://github.com/tokio-rs/turmoil/blob/main/crates/turmoil/src/sim.rs), [lib.rs docs](https://github.com/tokio-rs/turmoil/blob/main/crates/turmoil/src/lib.rs)
- Runtime per host (`rt.rs::init`):
  - `Builder::new_current_thread()`
  - under `cfg(tokio_unstable)`: `unhandled_panic(UnhandledPanic::ShutdownRuntime)` and `rng_seed(RngSeed::from_bytes(32 bytes from sim rng))`
  - `.enable_time().start_paused(true)`, and `enable_io()` only if configured
  - tasks run on a `LocalSet`, so `!Send` futures are fine
  - an initial 1 ms sleep "rounds" `Instant::now()`
  - `bounce` replaces both the Runtime and the LocalSet

  — [rt.rs](https://github.com/tokio-rs/turmoil/blob/main/crates/turmoil/src/rt.rs)
- `FsConfig` knobs:
  - `sync_probability`, `capacity`, `io_error_probability`, `corruption_probability`, `short_read_probability`
  - `noatime`, `direct_io_alignment`, `block_size`
  - `io_latency()` (min, max, distribution) and `page_cache()` (page_size, max_pages, random_eviction_probability)
  - the fs shim is at `turmoil::fs::shim::std::fs::OpenOptions` and similar

  — [turmoil-fs lib.rs](https://github.com/tokio-rs/turmoil/blob/main/crates/turmoil-fs/src/lib.rs)
- Barriers (`unstable-barriers`): `trigger(MyEvent).await` in source code; the test side uses `Barrier::build(Reaction::Suspend, pred)` and `.wait()` to suspend code at precise points. — [lib.rs docs](https://github.com/tokio-rs/turmoil/blob/main/crates/turmoil/src/lib.rs)
- `turmoil-net` (standalone, new):
  - it swaps `tokio::net` for `turmoil_net::shim::tokio::net` via `#[cfg]`
  - it implements real TCP mechanics (handshake, FIN/RST, MSS, backpressure)
  - fixtures: `fixture::lo`, `fixture::ClientServer`
  - fault injection via `Rule` returning `Verdict` (Pass, Deliver with delay, Drop)
  - `netstat` snapshots
  - "the stack is clock-free… a custom harness supplies its own" scheduler

  — [turmoil-net README](https://github.com/tokio-rs/turmoil/tree/main/crates/turmoil-net)
- Tracing: `RUST_LOG=turmoil=trace` gives packet-level Send, Delivered and Recv events. The tracing timer can be set to sim time (see the grpc example). — [lib.rs docs](https://github.com/tokio-rs/turmoil/blob/main/crates/turmoil/src/lib.rs)
- HTTP over turmoil:
  - the repo's `examples/axum` shows a hyper-util legacy `Client` with a custom connector wrapping `turmoil::net::TcpStream`
  - S2's `sim/src/net.rs` does the same for its SDK and for `object_store`'s S3 client (`with_http_connector(TurmoilHttpConnector)`)
  - reqwest has no pluggable connector in this pattern, so nothing here shows it running over turmoil

  — [turmoil examples/axum](https://github.com/tokio-rs/turmoil/tree/main/examples/axum), [s2 sim/src/net.rs](https://github.com/s2-streamstore/s2/blob/main/sim/src/net.rs)

### Inferences
- For piton, the HTTP `ModelProvider` must take an injectable hyper connector, or sit behind the trait so the sim swaps in a fake. Avoid hard-wiring reqwest with a built-in connector.
- WebSocket: `tokio-tungstenite::client_async`/`accept_async` accept any `AsyncRead + AsyncWrite` stream, so they should work over `turmoil::net::TcpStream`. This is unverified: I found no project doing it.
- TLS should be off in the sim. S2 uses plain `http://`.
- `Sim::host` takes a factory closure, so a "crash and restart of the shell process" is exactly `sim.crash("shell"); …; sim.bounce("shell")`. Any state that must survive has to live in a separate host, such as an emulated LogStore server, or in `turmoil-fs` synced files. Piton's in-memory LogStore should therefore be either (a) a separate turmoil host reached via RPC, like Materialize's `mz_persist::turmoil` and S2's mock S3, or (b) an `Arc` captured outside the host closure with explicit crash semantics.

### Gaps
- Whether `turmoil` will adopt `turmoil-net` internally, and when: not documented.
- The determinism guarantee of `enable_random_order` with respect to the seed was not verified.
- WebSocket over turmoil was not verified (see above).

---

## 3. madsim as an alternative

### Takeaway
madsim (v0.2.34, last commit 2026-02-16) replaces tokio entirely when built with `RUSTFLAGS="--cfg madsim"`. Its simulator is more total: it intercepts libc for time, randomness and threads. It needs `madsim-tokio`/`madsim-tonic` replacements and `[patch.crates-io]` forks of quanta, getrandom, tokio-retry, tokio-postgres and tokio-stream. Commit activity is lower than turmoil's.

### Cited Findings
- Usage: replace `tokio` with `package = "madsim-tokio"` (and likewise tonic, etcd-client, rdkafka, aws-sdk-s3); patch the listed crates; run `RUSTFLAGS="--cfg madsim" cargo test`. RisingWave uses it. — [madsim README](https://github.com/madsim-rs/madsim)
- Env knobs, from source: `MADSIM_TEST_SEED`, `MADSIM_TEST_NUM`, `MADSIM_TEST_JOBS`, `MADSIM_TEST_CONFIG`, `MADSIM_TEST_TIME_LIMIT`, `MADSIM_TEST_CHECK_DETERMINISM`, `MADSIM_ALLOW_SYSTEM_THREAD`. — [madsim source](https://github.com/madsim-rs/madsim/tree/main/madsim/src/sim)
- Runtime API: `Runtime::check_determinism(seed, config, f)`, `create_node() -> NodeBuilder` (with `restart_on_panic`, `restart_on_panic_matching`, `build`), `kill`, `restart`, `pause`, `resume`. Network API: `clog_node[_in/_out]`, `unclog_*`, `clog_link`, `unclog_link`. — [madsim runtime/mod.rs](https://github.com/madsim-rs/madsim/blob/main/madsim/src/sim/runtime/mod.rs), [net/mod.rs](https://github.com/madsim-rs/madsim/blob/main/madsim/src/sim/net/mod.rs)
- libc overrides: `getrandom`, `getentropy` and `CCRandomGenerateBytes` (`rand.rs`), `clock_gettime` (`time/system_time.rs`), plus `sysconf` and `pthread_attr_init` (`task/mod.rs`, to catch thread spawns). It falls back via `dlsym(RTLD_NEXT, …)`. mad-turmoil is explicitly derived from these files. — [madsim rand.rs](https://github.com/madsim-rs/madsim/blob/main/madsim/src/sim/rand.rs), [mad-turmoil](https://github.com/s2-streamstore/mad-turmoil)

### Inferences
- Tradeoff: madsim has more complete determinism, including thread-spawn detection and built-in `check_determinism`, but it forces the whole dependency graph onto shim crates and a `cfg`. turmoil plus mad-turmoil keeps real tokio and real hyper/axum, at the cost of DIY leak-hunting via the meta test. S2 chose turmoil for that reason ("presumes Tokio… turmoil's approach was not comprehensive enough" for time and randomness, hence mad-turmoil). For piton, which has `#[async_trait]` traits and hyper/WebSocket, turmoil plus mad-turmoil is the lower-friction fit.

### Gaps
- madsim's WebSocket and hyper support status was not checked.

---

## 4. tokio determinism knobs (docs.rs tokio 1.53.3)

### Takeaway
`Builder::rng_seed(RngSeed)` and `unhandled_panic` remain `tokio_unstable` as of tokio 1.53.3. `start_paused` is stable but needs feature `test-util` and current_thread. `LocalRuntime` (`build_local`) is documented under feature `rt` with no unstable marker.

### Cited Findings
- `rng_seed`:
  - requires `tokio_unstable`
  - it is "intended to make certain parts of the runtime deterministic", e.g. the `select!` polling order
  - it is not a full guarantee: the tokio version, the versions of dependencies that interact with tokio, and the rustc version must stay constant

  — [tokio Builder docs](https://docs.rs/tokio/latest/tokio/runtime/struct.Builder.html)
- `unhandled_panic(UnhandledPanic::ShutdownRuntime)`: `tokio_unstable`, current_thread only, "incomplete and may change" (tokio#4516). — [tokio Builder docs](https://docs.rs/tokio/latest/tokio/runtime/struct.Builder.html)
- `start_paused(bool)`: feature `test-util`; it panics on a non-current_thread runtime. — [tokio Builder docs](https://docs.rs/tokio/latest/tokio/runtime/struct.Builder.html)
- `build_local(LocalOptions) -> LocalRuntime`: drives `!Send` tasks without a `LocalSet`, and must not be combined with `LocalSet`. The docs page shows only "Available on crate feature `rt`". — [tokio LocalRuntime docs](https://docs.rs/tokio/latest/tokio/runtime/struct.LocalRuntime.html)
- turmoil and S2 both need `RUSTFLAGS="--cfg tokio_unstable"`. S2 bails at runtime and opbox uses `compile_error!`. — [s2 sim main.rs](https://github.com/s2-streamstore/s2/blob/main/sim/src/main.rs), [opbox sim](https://github.com/s2-streamstore/opbox/blob/main/sim/src/main.rs)

### Inferences
- Pin the tokio version and rustc in the sim workspace lockfile and toolchain. Determinism is version-sensitive by tokio's own docs, and devenv already pins rustc.
- Piton's root-workspace lints and `just ci` may need a separate `--cfg tokio_unstable` path for the sim crate, as S2 does with `just sim` and separate clippy. This is in tension with "keep local and CI on the same path"; solve it with a `just sim` recipe used by CI.

### Gaps
- I did not verify whether `LocalRuntime` was formally stabilised or is still flagged in prose elsewhere in the tokio docs.

---

## 5. Faking the shell's dependencies deterministically

### Takeaway
There are two complementary patterns:
- (a) Emulator-as-host: run an in-memory fake as its own turmoil host, reached over simulated TCP, so turmoil can partition or crash it independently. This is what S2 does for S3/FoundationDB and Materialize does for persist Consensus/Blob.
- (b) In-process trait fakes with fault decorators: opbox's `FaultInjectingFileIO` over `InMemoryFileIO`, both implementing an `#[async_trait]` trait.

Combine them with simulated-time timeouts and side-effect-aware retry policies.

### Cited Findings
- Materialize `src/persist/src/turmoil.rs`: "[Consensus] and [Blob] implementations suitable for use with [turmoil]". They forward calls over `turmoil::net::TcpStream` to server hosts backed by `MemConsensus` and `MemBlob`. "Both the consensus and the blob server should be run as separate hosts. This gives turmoil the ability to partition and crash them independently". URIs are `turmoil://consensus:7000`; `RPC_TIMEOUT = 1s`. It uses `CaSResult`, `SeqNo` and `Determinate`/`Indeterminate` errors, which is the same shape as piton's conditional-append LogStore. — [materialize persist/src/turmoil.rs](https://github.com/MaterializeInc/materialize/blob/main/src/persist/src/turmoil.rs)
- S2 mock S3 (`sim/src/s3.rs`): implements `s3s::S3` over `Arc<Mutex<BTreeMap<String, StoredObject>>>`. Unimplemented ops return `NotImplemented` loudly. The comment notes that metadata is load-bearing because slatedb re-reads after a lost put response to check whether its write landed. — [s2 sim/src/s3.rs](https://github.com/s2-streamstore/s2/blob/main/sim/src/s3.rs)
- opbox: `FaultInjectingFileIO { inner: InMemoryFileIO, state: Arc<Mutex<FaultState { faults: VecDeque<Fault>, … }>> }`, with scripted fault variants such as conflict-before-swap, conflict-after-swap and temp-leak-and-fail. — [opbox fault_injecting_file_io.rs](https://github.com/s2-streamstore/opbox/blob/main/sim/src/fault_injecting_file_io.rs)
- Indefinite outcomes: S2 forces `AppendRetryPolicy::NoSideEffects` in the sim and records maybe-applied appends as indefinite. A timed-out conditional append must be modelled as "may land later". — [s2 sim/src/scenarios/mod.rs](https://github.com/s2-streamstore/s2/blob/main/sim/src/scenarios/mod.rs), [S2 linearizability post](https://s2.dev/blog/linearizability)
- Under message loss, turmoil does not retransmit a dropped SYN, so connect needs a simulated-time timeout. — [s2 sim/src/net.rs](https://github.com/s2-streamstore/s2/blob/main/sim/src/net.rs)
- Commonware's `deterministic` runtime (commonware-runtime 2026.9.0) offers a different model:
  - a seeded random task scheduler
  - `context.auditor().state()` for determinism auditing
  - `Checkpoint` "to recover the state of the runtime"
  - storage `FaultConfig`, `WriteConfig` and `PartialWriteMode` controlling how many unsynced bytes survive a simulated crash

  This is useful prior art for LogStore crash and torn-write semantics. — [commonware-runtime deterministic docs](https://docs.rs/commonware-runtime/latest/commonware_runtime/deterministic/index.html)

### Inferences (concrete design for piton)
- **LogStore fake.** A `MemLogStore` holds a `BTreeMap` with acked state and a pending tail.
  - `append(expected_seq, fence_token, entries)` returns `Ok`, `Conflict`, `Fenced`, or `Indeterminate`.
  - Faults drawn from the sim RNG:
    - (i) reject (definite)
    - (ii) apply-then-drop-response (indeterminate, landed)
    - (iii) drop-then-timeout (indeterminate, not landed)
    - (iv) delayed apply, which lands after the client gave up. This is the case S2's model gotcha teaches.
    - (v) crash, which loses unacked/unsynced writes but never acked ones
    - (vi) a stale writer with an old fence token racing a new one
  - Run it as a separate turmoil host behind a tiny RPC, following Materialize, so `partition`/`hold`/`crash` apply. Or keep an in-proc `Arc` outside the shell's host closure, so it survives `sim.crash("shell")`.
- **Fake streaming model server.** Implement an axum/hyper host (`sim.host("model", …)`) that streams SSE or chunked tokens chosen by a seeded RNG. Inject mid-stream disconnects, stalls (`tokio::time::sleep`, which is simulated), 429/5xx responses and malformed chunks. Keep the real HTTP `ModelProvider` and inject a turmoil hyper connector, as S2 does with `S2::new_with_connector`.
- **Flaky ToolHost.** Use an in-proc decorator in the style of opbox's `FaultInjectingFileIO`, with scripted or seeded `VecDeque<Fault>`: timeout, error, duplicate completion, late completion after cancel.
- **Clients.** Make them `sim.client(...)` workloads over turmoil TCP and WebSocket. Disconnect and reconnect via `turmoil::partition`/`repair`, `hold`/`release`, or by dropping the stream. In-process channel clients run inside the shell host.
- **Crash/restart of the shell.** Make the shell a `sim.host("shell", || shell_main(cfg))` factory. Call `sim.crash("shell")` at an RNG-chosen `sim.step()`, then `sim.bounce("shell")`. Recovery must rebuild from the LogStore host plus the input journal. If the journal is file-based, use `unstable-fs` with `sync_probability`.
- **Wall-clock.** Route all time through `tokio::time` (Instant, sleep, timeout). Use mad-turmoil `SimClocksGuard` to catch `SystemTime::now()` in dependencies, and `Builder::epoch(UNIX_EPOCH)`. Better for piton: the sans-IO core takes time as an input event, so the shell is the only reader of the clock.

### Gaps
- I found no public example of turmoil plus WebSocket, or of an SSE/LLM streaming fake. These are design suggestions only.

---

## 6. Structured, replayable traces

### Takeaway
S2 emits two artifacts:
- (1) deterministic TRACE logs with sim-time and event-ordinal timestamps, used for the determinism diff
- (2) a typed JSONL operation history (`LabeledEvent` Start/Finish with client_id, op_id and inputs/outputs) consumed by an external checker (Porcupine)

The second is the model to copy for replay into a model checker.

### Cited Findings
- `SimStepTimeFormat` writes `[s{sim_ms} e{event_ordinal}]` per tracing line. — [s2 sim/src/main.rs](https://github.com/s2-streamstore/s2/blob/main/sim/src/main.rs)
- The history is `s2_verification::history::LabeledEvent`, serialised with `serde_json` one per line to `history.<seed>.jsonl`. Deferred indefinite events are appended at the end. The meta test also diffs the history files across the two runs. — [s2 sim/src/history.rs](https://github.com/s2-streamstore/s2/blob/main/sim/src/history.rs), [meta.rs](https://github.com/s2-streamstore/s2/blob/main/sim/src/meta.rs)
- turmoil logs `Send`, `Delivered` and `Recv` per packet, plus `Crash`/`Bounce` and `step N` at `turmoil=trace`. — [turmoil lib.rs](https://github.com/tokio-rs/turmoil/blob/main/crates/turmoil/src/lib.rs), [sim.rs](https://github.com/tokio-rs/turmoil/blob/main/crates/turmoil/src/sim.rs)
- Start events log inputs, e.g. per-record hashes, rather than pre-folded state, "since the prior state is only known inside the checker". — [s2-verification README](https://github.com/s2-streamstore/s2-verification)

### Inferences
- For piton, emit a typed `SimEvent` enum. Examples: `ClientOpStart/Finish`, `LogAppend{expected, fence, outcome}`, `ModelChunk`, `ToolCall/Result`, `Crash`, `Restart`, `Partition`, and the core's `Input`/`Effect`.
- Stamp each event with `(sim_elapsed, ordinal, host)`. Write JSONL via a bounded or Vec sink drained after `sim.run()`, since piton bans unbounded channels.
- Since the sans-IO core consumes inputs and produces effects, log the exact `Input` sequence fed to the core per session. That sequence is directly replayable into the core and into a Stateright or FizzBee model as a trace to check.
- Hash, don't store, large payloads (S2's xxh3 chain).

### Gaps
- No S2 source logs core-level state transitions. The trace schema above is a suggestion.

---

## 7. Other Rust projects doing turmoil/madsim DST (concrete paths)

### Takeaway
GitHub code search for `turmoil::Builder` returns these users:
- S2: `s2/sim`, `opbox/sim`
- Materialize: `src/persist/src/turmoil.rs`, `src/cluster/src/communication.rs`
- freenet-core: `crates/core/src/node/testing_impl/turmoil_runner.rs`
- aws/s2n-quic: `examples/turmoil-provider`
- dfinity/ic: `rs/p2p/quic_transport`
- pulsebeam: `crates/pulsebeam-runtime/src/testing.rs`
- rockwotj/chorus: `rust/dst/src/production.rs`
- f-rustrated/EastGuard: raft/swim sim tests under `src/it/`

RisingWave uses madsim. Commonware has its own deterministic runtime.

### Cited Findings
- The code-search results listed above came from `gh api search/code q='turmoil::Builder language:Rust'`, run 2026-10-18. Example: [freenet turmoil_runner.rs](https://github.com/freenet/freenet-core/blob/main/crates/core/src/node/testing_impl/turmoil_runner.rs).
- Materialize persist turmoil Consensus/Blob over RPC hosts. — [materialize persist/src/turmoil.rs](https://github.com/MaterializeInc/materialize/blob/main/src/persist/src/turmoil.rs)
- RisingWave uses MadSim and has a 2-part blog writeup. — [madsim README](https://github.com/madsim-rs/madsim), [RisingWave blog](https://www.risingwave.com/blog/deterministic-simulation-a-new-era-of-distributed-system-testing/)
- Commonware `deterministic` runtime: seeded scheduler, auditor, checkpoint, storage faults. — [commonware docs](https://docs.rs/commonware-runtime/latest/commonware_runtime/deterministic/index.html)

### Inferences
- Commonware's design is closest to piton's multi-host future, since it puts an abstract runtime trait (`Spawner + Clock + Network + Storage`) in the app. A shell written against such a trait could run on tokio, a deterministic executor, and possibly a DO/wasm executor. turmoil only covers tokio.

### Gaps
- foyer: no turmoil or madsim usage found. Of foyer, I only found S2's CI note that foyer's dependency fastant breaks under the clock interposition on x86_64.
- I did not read freenet, chorus or pulsebeam beyond the search hits.

---

## 8. Pitfalls and nondeterminism leaks

### Takeaway
Leaks come from:
- any multithreading (rt-multi-thread, rayon, `std::thread`, `spawn_blocking`)
- unseeded tokio RNG (`select!`)
- `HashMap` `RandomState` in dependencies
- raw syscalls that bypass libc (getrandom 0.2 on Linux)
- `std::time` and `SystemTime` (including HTTP `Date` headers and tracing timestamps)
- fastrand's time-based hashing
- pre-main clock-spinning crates (fastant TSC calibration)

The meta test at TRACE level is the detector. turmoil cannot cover wasm or Durable Object hosts.

### Cited Findings
- HTTP packet timestamps, `HashMap` seeds, and dependencies spawning threads, using RNGs, reading time or doing I/O were all observed leaks. — [S2 DST post](https://s2.dev/blog/dst)
- getrandom 0.2's raw `SYS_getrandom` on Linux bypasses interposition, so S2 patches it. — [s2 sim/Cargo.toml](https://github.com/s2-streamstore/s2/blob/main/sim/Cargo.toml)
- fastant pre-main TSC calibration spins forever under the interposed `clock_gettime` on x86_64, so the CI job runs on arm64. — [s2 ci.yml](https://github.com/s2-streamstore/s2/blob/main/.github/workflows/ci.yml)
- Callers outside the sim context (mimalloc, tracing, fastrand) get epoch 0. fastrand mixes in the current time even when seeded, so S2 also calls `fastrand::seed`. — [mad-turmoil time.rs](https://github.com/s2-streamstore/mad-turmoil/blob/main/src/time.rs), [mad-turmoil README](https://github.com/s2-streamstore/mad-turmoil)
- Process teardown with the interposed clock can SIGTRAP without a guard. — [mad-turmoil time.rs](https://github.com/s2-streamstore/mad-turmoil/blob/main/src/time.rs)
- Without `tokio_unstable`, turmoil does not seed tokio's RNG, so `select!` is nondeterministic. — [turmoil rt.rs](https://github.com/tokio-rs/turmoil/blob/main/crates/turmoil/src/rt.rs)
- tokio says seed determinism also depends on fixed tokio, dependency and rustc versions. — [tokio Builder docs](https://docs.rs/tokio/latest/tokio/runtime/struct.Builder.html)
- Info-level meta tests miss leaks that trace level catches (about 50 MB vs a dozen lines). — [s2 ci.yml](https://github.com/s2-streamstore/s2/blob/main/.github/workflows/ci.yml)
- madsim additionally interposes `sysconf` and `pthread_attr_init` to detect system threads (`MADSIM_ALLOW_SYSTEM_THREAD`). mad-turmoil does not. — [madsim task/mod.rs](https://github.com/madsim-rs/madsim/blob/main/madsim/src/sim/task/mod.rs)

### Inferences
- Piton's lint bans on hash-ordered collections already remove HashMap iteration-order leaks in first-party code. Dependencies remain a risk; mad-turmoil's `getrandom` override seeds std's `RandomState` only if std reaches it via the libc symbol (see Gaps).
- Avoid `spawn_blocking` and rayon in the shell, or put them behind a trait whose sim impl runs inline. A `Send`-free single-thread design also aligns with DO/wasm.
- mad-turmoil is unsafe `#[no_mangle]` symbol interposition. It must live only in the sim binary, never in a library crate: the README says so explicitly. It needs an `#[expect(unsafe_code, reason=…)]`-style exception under piton's strict lints.
- wasm/Durable Objects: turmoil depends on tokio (`rt`, `net`, `time` and more) and libc interposition, so it cannot run in a DO. Cover those hosts by keeping their shell thin over the same sans-IO core, plus a separate deterministic harness, e.g. a hand-written single-threaded executor driving the DO shell with fake `storage` and `fetch`. Or abstract the shell over a runtime trait in the Commonware style.

### Gaps
- Not verified: whether std's `HashMap` `RandomState` on current Linux and macOS std goes through the libc `getrandom`/`CCRandomGenerateBytes` symbols, and so is caught by interposition, or uses raw syscalls. S2 implies it is handled, but gives no explicit source.
- No source found on deterministic DST tooling for Cloudflare Durable Objects or workerd.
- `select!` fairness with `biased;` is a standard mitigation, but no S2 source addresses it beyond seeding.
