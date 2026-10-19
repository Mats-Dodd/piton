# Deterministic Simulation Testing (DST), with a focus on modern Rust

Research date: 2026-10-18. Links were checked by fetching the page, or by querying the GitHub API for repository paths, unless marked "(not fetched)".

## Foundational material: what to read and what each source teaches

### Takeaway
The core reading is short. Start with Will Wilson's FoundationDB talk, the sled simulation guide, Phil Eaton's 2024 overview, and TigerBeetle's VOPR and TigerStyle material. The Rust-specific posts worth reading are S2 (2025, turmoil plus libc interception), Polar Signals (2025, sans-IO state machines), and RisingWave (2023, madsim). Two curated indexes keep the list current: Pierre Zemb's reading list and ivanyu's awesome-DST list.

### Cited Findings

**Classics and foundations (non-Rust)**
- **Will Wilson, "Testing Distributed Systems w/ Deterministic Simulation" (Strange Loop 2014).** This is the canonical origin talk for DST. Pierre Zemb's list and the awesome-DST list both give the URL https://www.youtube.com/watch?v=4fFDFbi3toc — [Zemb](https://pierrezemb.fr/posts/learn-about-dst/), [awesome-DST](https://github.com/ivanyu/awesome-deterministic-simulation-testing)
  - Conflict: S2's post links https://www.youtube.com/watch?v=fFSPwJFXVlw as "the FoundationDB talk". Zemb labels that video as Wilson's later talk, "Autonomous Testing and the Future of Software Development". Use 4fFDFbi3toc for the Strange Loop 2014 talk. — [S2](https://s2.dev/blog/dst); [Zemb](https://pierrezemb.fr/posts/learn-about-dst/)
  - Zemb also lists two more Wilson talks: "Testing a Single-Node, Single Threaded, Distributed System Written in 1985" (https://www.youtube.com/watch?v=m3HwXlQPCEU) and "Let's all write good software" (https://www.youtube.com/watch?v=eZ1mmqlq-mY). — [Zemb](https://pierrezemb.fr/posts/learn-about-dst/)
- **FoundationDB "Testing" docs** give the official overview of how FoundationDB simulates whole clusters deterministically with injected failures: https://apple.github.io/foundationdb/testing.html — [Zemb](https://pierrezemb.fr/posts/learn-about-dst/)
- **Ben Collins, "FoundationDB Testing: Past & Present"** (https://www.youtube.com/watch?v=IaB8jvjW0kk) — [Zemb](https://pierrezemb.fr/posts/learn-about-dst/)
- **Alex Miller, "BUGGIFY"** (https://transactional.blog/simulation/buggify) explains FoundationDB's technique of placing fault-injection points in the code itself. — [Zemb](https://pierrezemb.fr/posts/learn-about-dst/)
- **CockroachLabs, "Demonic Nondeterminism"** (https://www.cockroachlabs.com/blog/demonic-nondeterminism/) — [Zemb](https://pierrezemb.fr/posts/learn-about-dst/)
- **TigerBeetle (Zig).**
  - The VOPR runs a whole cluster of real code under network, storage, and process faults at about 1000x speed. TigerBeetle says it runs "24/7 on 1024 cores". The storage faults it models are silent corruption, misdirected I/O, gray failure, and latent sector errors.
  - TigerStyle (https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md) draws on NASA's "Power of Ten" rules. TigerBeetle also uses static memory allocation.
  - Further links: the VOPR liveness post (https://tigerbeetle.com/blog/2023-07-06-simulation-testing-for-liveness/) and the playable simulator (https://sim.tigerbeetle.com). — [TigerBeetle safety docs](https://docs.tigerbeetle.com/concepts/safety/)
  - Two more TigerBeetle posts show how to make OS IO simulatable:
    - "A Friendly Abstraction Over io_uring and kqueue" (https://tigerbeetle.com/blog/2022-11-23-a-friendly-abstraction-over-iouring-and-kqueue/)
    - "We Put a Distributed Database in the Browser" (https://tigerbeetle.com/blog/2023-07-11-we-put-a-distributed-database-in-the-browser/)
    — [Zemb](https://pierrezemb.fr/posts/learn-about-dst/)
  - The VOPR doc page is https://docs.tigerbeetle.com/about/vopr. — [awesome-DST](https://github.com/ivanyu/awesome-deterministic-simulation-testing)
  - Files to study in the repository (verified with the GitHub API):
    - `src/vopr.zig`
    - `src/testing/` (`cluster.zig`, `packet_simulator.zig`, `storage.zig`, `state_machine.zig`, `fuzz.zig`, `vortex/`)
    - `src/*_fuzz.zig`
    — [tigerbeetle/tigerbeetle](https://github.com/tigerbeetle/tigerbeetle/tree/main/src/testing)
- **Dropbox, "Testing Our New Sync Engine"** (https://dropbox.tech/infrastructure/-testing-our-new-sync-engine) is a pre-Rust-era industrial case study. Both curated lists and the sled guide cite it. — [sled guide](https://sled.rs/simulation.html)

**Overviews and essays**
- **Phil Eaton, "What's the big deal about Deterministic Simulation Testing?"** (2024-08-20).
  - He describes two approaches:
    1. Transparent, lower-level determinism through Antithesis, Hermit, or rr. Hermit and rr have limited fault injection and do not run on macOS or ARM.
    2. Design for simulation: run on a single thread with async IO, and inject the clock, seed, and IO as dependencies.
  - Required disciplines: inject the clock and RNG instead of reading globals, use async IO, keep the deterministic core large, and write your own workload.
  - Caveats:
    - External edges such as Kafka and Postgres must be mocked.
    - Tuning workloads is labor-intensive, and branch coverage is a weak signal, so he points to "Sometimes" assertions.
    - Seeds stop reproducing a bug once the code changes, so a found bug should be turned into a regular test.
    - Residual nondeterminism remains in CPU instructions, syscalls, and malloc. He calls determinism "a spectrum".
  - He says the tokio simulator "never fully worked" and points to turmoil and Pekka Enberg's hiisi. — [Eaton](https://notes.eatonphil.com/2024-08-20-deterministic-simulation-testing.html)
- **sled "Simulation Guide" ("Jepsen-proof engineering")** (author not stated on the page; the sponsor link points to the "spacejam" account, which is Tyler Neely). It gives the original receive/tick state-machine recipe:
  - `receive(msg) -> Vec<(dest, msg)>`, plus `tick(at)` for timers. `tick` can be folded into `receive` with self-addressed timer messages.
  - All events go into a priority queue keyed on delivery time. Each message gets a deterministic delivery time or is dropped.
  - It recommends an asynchronous network model in which any message can be delayed, dropped, or reordered.
  - Check client-observable invariants such as linearizability and split brain.
  - Shrink failing seeds with quickcheck. — [sled](https://sled.rs/simulation.html)
- **Pierre Zemb, "So, You Want to Learn More About Deterministic Simulation Testing?"** (2025-04-11) is the best curated reading list. It also lists AWS "Systems Correctness Practices" (ACM Queue), Marc Brooker's re:Invent 2024 ARC403 talk, Resonate (Dominik Tornow), poorlydefinedbehaviour's DST post, and the FOSDEM 2025 talk "Squashing the Heisenbug with DST". — [Zemb](https://pierrezemb.fr/posts/learn-about-dst/)
- **ivanyu/awesome-deterministic-simulation-testing** is a curated list. It includes a DST talk by Pekka Enberg (https://www.youtube.com/live/29Vz5wkoUR8), hiisi, MadRaft, Unthread, and the Java framework Tickloom. — [awesome-DST](https://github.com/ivanyu/awesome-deterministic-simulation-testing)
- **Planet DST** is an aggregated feed of DST posts: https://deterministic-simulation-testing.github.io/planet-dst/ — [Zemb](https://pierrezemb.fr/posts/learn-about-dst/)

**Rust-specific industry posts**
- **S2, "Deterministic simulation testing for async Rust"** (Shikhar Bhushan, 2025-04-02).
  - Stack: single-threaded tokio with a paused clock and seeded RNG, turmoil for the network, in-memory emulators for FoundationDB metadata and S3 reached over the simulated network, and **mad-turmoil** for libc overrides.
  - A CI meta-test reruns the same seed and diffs the TRACE logs.
  - 17 notable bugs found. — [S2](https://s2.dev/blog/dst)
- **S2, "Linearizability testing S2 with deterministic simulation"**: the follow-up on randomized workloads checked for linearizability in CI and in nightly runs. — [S2](https://s2.dev/blog/linearizability) (search snippet only, not fetched)
- **Polar Signals, "Deterministic Simulation Testing in Rust: A Theater Of State Machines"** (Alfonso Subiotto Marqués, 2025-07-08). This is the sans-IO approach; details are in the approaches section below. — [Polar Signals](https://www.polarsignals.com/blog/posts/2025/07/08/dst-rust)
  - Earlier Go post: "(Mostly) DST in Go" (2024-05-28), which used WASM and a single-threaded Go runtime. — [Polar Signals Go](https://www.polarsignals.com/blog/posts/2024/05/28/mostly-dst-in-go)
  - Talks: FOSDEM 2026, "Random seeds and state machines: An approach to DST in Rust", and QCon London (March 2026), "A DST Journey: From WASM in Go to State Machines in Rust". — [FOSDEM 2026](https://fosdem.org/2026/schedule/event/rust-deterministic-simulation-testing), [QCon](https://qconlondon.com/presentation/mar2026/deterministic-simulation-testing-dst-journey-wasm-go-state-machines-rust)
- **RisingWave, "Deterministic Simulation: A New Era of Distributed System Testing" (Part 1) and "Applying Deterministic Simulation: The RisingWave Story" (Part 2, 2023-04-25).**
  - Setup: madsim with simulated etcd, S3, and Kafka. Simulation runs about 4–5x faster than real execution, with many seeds run in parallel on 16 cores.
  - Tests: unit, end-to-end sqllogictest (about 8 min down to about 2), recovery (random kill and restart), and scaling (random shard rescheduling compared against a baseline).
  - Bugs found: deadlocks, data loss and duplication during source migration, and cache invalidation problems.
  - Lessons: recovery tests took about two months of debugging. Test time grew, so the team made error injection probabilistic. The Kafka simulator is the only external-source simulator they maintain. — [RisingWave Part 2](https://risingwave.com/blog/applying-deterministic-simulation-the-risingwave-story-part-2-of-2/), [Part 1](https://risingwave.com/blog/deterministic-simulation-a-new-era-of-distributed-system-testing/)
- **WarpStream, "DST for Our Entire SaaS"** is an Antithesis case study: https://www.warpstream.com/blog/deterministic-simulation-testing-for-our-entire-saas — [Zemb](https://pierrezemb.fr/posts/learn-about-dst/) (not fetched)
- **Smaller practical posts:**
  - Astradot, "DST on every commit" (turmoil) — [Astradot](https://blog.astradot.com/deterministic-simulation-testing-on-every-commit-2/)
  - Tiare Balbi, "Catching a Retry Race With One Seed: Rust + turmoil" — [Balbi](https://www.tiarebalbi.com/en/blog/catching-retry-race-deterministic-simulation-rust-turmoil)
  - Both come from search snippets and were not fetched.

**Antithesis**
- Antithesis's "Determinator" is a deterministic hypervisor that runs a multi-container test environment inside one VM. — [Antithesis blog](https://antithesis.com/blog/deterministic_hypervisor/). It is built on FreeBSD/bhyve — [FreeBSD Foundation](https://freebsdfoundation.org/antithesis-pioneering-deterministic-hypervisors-with-freebsd-and-bhyve/)
- The Rust SDK is the `antithesis_sdk` crate. It provides assertions (including "sometimes" assertions), randomness, and lifecycle signals, and is set up by calling `antithesis_init()`. — [Antithesis Rust SDK docs](https://antithesis.com/docs/reference/sdk/rust/)
- Also worth reading: the DST primer page (https://antithesis.com/docs/resources/deterministic_simulation_testing/) and the "Super Mario" talk post (https://antithesis.com/blog/sdtalk/). — [Zemb](https://pierrezemb.fr/posts/learn-about-dst/)

### Inferences
- A reading order for a Rust developer: Wilson 2014 talk → sled guide → Eaton → TigerStyle and the VOPR post → S2 post → Polar Signals post → RisingWave Part 2. Together these cover theory, a minimal recipe, tradeoffs, design discipline, and the two main Rust architectures.

### Gaps
- I did not fetch these, so their content is unverified: Resonate's DST posts, Materialize's engineering blog, Readyset, Fly.io, WarpStream, and S2's linearizability post.
- I found no Fly.io or Readyset DST write-up in my searches.
- I found no dedicated Turso blog post on its simulator (two searches). The best Turso sources are the repository READMEs and the Pekka Enberg talk linked above.

## Rust tools and the four approaches: runtime replacement vs libc interception vs sans-IO vs executor or scheduler control

### Takeaway
There are four families, plus whole-machine determinism (Antithesis).
1. **Runtime replacement**: madsim, and turmoil on top of single-threaded tokio. Lowest retrofit cost for tokio codebases. Determinism is leaky wherever a dependency bypasses the runtime.
2. **libc symbol interception**: madsim internals, mad-turmoil, Turso's unreliable-libc. Plugs the time, entropy, and IO leaks from third-party crates. Platform-specific and somewhat magic.
3. **Sans-IO state machines with a custom event loop**: sled guide, Polar Signals, TigerBeetle-style hiisi. Full control and failure injection written once. Requires designing for it from the start.
4. **Concurrency schedulers for threads and atomics**: loom (exhaustive) and shuttle (randomized). These test small concurrent data structures, not distributed systems.

### Cited Findings

**madsim (madsim-rs/madsim)**
- An async runtime "similar to tokio" with deterministic simulation, modelled on FoundationDB and the sled guide.
- Turned on with `RUSTFLAGS="--cfg madsim"`. Without the cfg, the crates behave like the originals.
- Supplies drop-in replacements: `madsim-tokio`, `madsim-tonic` (and `madsim-tonic-build`), `madsim-etcd-client`, `madsim-rdkafka`, and `madsim-aws-sdk-s3`.
- `[patch.crates-io]` swaps in patched versions of `quanta`, `getrandom`, `tokio-retry`, `tokio-postgres`, and `tokio-stream`.
- Has APIs to kill nodes, disconnect the network, and inject failures.
- Users: RisingWave and MadRaft. — [madsim README](https://github.com/madsim-rs/madsim)
- Latest version is 0.2.34. — [docs.rs](https://docs.rs/crate/madsim/latest)
- It overrides libc symbols such as `getrandom` and `clock_gettime`, and the idea is discussed in madsim discussion #159. — [S2](https://s2.dev/blog/dst); see https://github.com/madsim-rs/madsim/discussions/159
- It can run a test twice with the same seed to detect nondeterminism. A global seeded PRNG is the only source of randomness, and a timer priority queue drives all events. — [RisingWave Part 1](https://risingwave.com/blog/deterministic-simulation-a-new-era-of-distributed-system-testing/) (via search snippet)

**turmoil (tokio-rs/turmoil)**
- Runs many hosts on one thread, with seeded fault injection: latency, drops, partitions, crashes, and filesystem torn writes.
- It is now a family of crates:
  - `turmoil`
  - `turmoil-net` (drop-in replacement for `tokio::net`)
  - `turmoil-fs` (replacement for `std::fs` and `tokio::fs`)
  - `turmoil-io-uring`
- The repository root has `crates/` and `examples/`.
- The README calls the project experimental. — [turmoil README](https://github.com/tokio-rs/turmoil); [S2 search snippet re docs](https://docs.rs/crate/turmoil/0.5.2)
- Turmoil alone does not control time or randomness inside arbitrary dependencies, and `HashMap` uses randomized hashing. This gap is why S2 built mad-turmoil. — [S2](https://s2.dev/blog/dst)

**mad-turmoil (s2-streamstore/mad-turmoil)**
- Described as "madsim-inspired determinism for turmoil-based simulation tests".
- The whole crate is `src/lib.rs`, `src/rand.rs`, and `src/time.rs` (verified with the GitHub API).
- `rand.rs` overrides `getrandom`, `getentropy`, and, on macOS, `CCRandomGenerateBytes`, backed by an RNG set with `set_rng()`.
- `time.rs` overrides `clock_gettime` to read turmoil's clock, scoped by a `SimClocksGuard`. — [S2](https://s2.dev/blog/dst), [repo](https://github.com/s2-streamstore/mad-turmoil)

**Sans-IO with a custom event loop (Polar Signals, sled guide)**
- Each component is a single-threaded state machine with two methods: `receive(&mut self, m: Message)`, which returns outgoing messages, and `tick(&mut self, curtime: Instant)`.
- In production a "driver" wraps each machine. In tests a single-threaded message bus (the "Director") is the only caller of `tick` and `receive`.
- Concurrency: the trait has no `async`, so the compiler guarantees no futures are spawned inside a machine. The schedule is just the order in which messages are applied.
- Time: machines see only the timestamp passed to `tick`.
- Randomness: one seeded PRNG drives everything, and the seed is printed at the start of each run.
- Failure injection: IO dependencies are also modelled as state machines. The bus can error, drop, or reorder messages, and that logic is written once for every component.
- Costs:
  - Cognitive overhead; under time pressure, logic leaks into the untested driver.
  - Nondeterministic dependencies such as UUID generation must be mocked.
  - Async dependencies are wrapped in `block_on`.
  - It needs a new codebase.
- The author recommends madsim for retrofitting existing Rust code. The post does not discuss turmoil or libc interception. — [Polar Signals](https://www.polarsignals.com/blog/posts/2025/07/08/dst-rust)
- Pekka Enberg's **hiisi** applies TigerBeetle style to Rust: "a proof of concept libSQL written in Rust following TigerBeetle-style with deterministic simulation testing" (repository description). — [penberg/hiisi](https://github.com/penberg/hiisi)

**Executor or scheduler control for threads (loom, shuttle)**
- Shuttle (awslabs) does randomized concurrency testing, including PCT (probabilistic concurrency testing). It is "heavily inspired by Loom".
- Shuttle is not sound: a passing test does not prove correctness. In exchange it scales to much larger tests than loom.
- Shuttle controls the scheduling of every thread, so a failing schedule can be replayed deterministically.
- Loom explores interleavings exhaustively, which only stays feasible for small cases. — [shuttle](https://github.com/awslabs/shuttle), [docs.rs/shuttle](https://docs.rs/shuttle)
- Shuttle was still active in October 2026 (last push 2026-10-15, per the GitHub API).

**Newer crates (2024–2026)**
- **commonware-runtime `deterministic` module** (Commonware monorepo, `runtime/src/deterministic.rs`). It provides a seeded `deterministic::Runner` with a timeout, and a `start_and_recover` pattern for testing unclean shutdown and restart. To check determinism, run twice with the same seed and compare `context.auditor().state()`. Version 2026.7.0. — [commonware TESTING.md](https://docs.rs/crate/commonware-runtime/latest/source/TESTING.md)
  - Its runtime is a trait with separate backends: `tokio/`, `iouring/`, `deterministic.rs`, `mocks.rs`, `network/`, and `storage/` (verified with the GitHub API). That design is runtime abstraction through traits, not cfg-based replacement.
- **moonpool**: DST inspired by FoundationDB. It calls itself hobby-grade and was at 0.8.0 in July 2026. — [lib.rs](https://lib.rs/crates/moonpool)
- **simulacra**: a discrete-event engine with a Tokio-style async API, 0.1.0 in May 2026. — [lib.rs](https://lib.rs/crates/simulacra)
- **foundationdb-simulation**: runs Rust workloads inside FoundationDB's own simulator (0.3.0, June 2026). — [lib.rs](https://lib.rs/crates/foundationdb-simulation); background in [Zemb's post on embedding Rust in FDB sim](https://pierrezemb.fr/posts/providing-safety-fdb-rs/#user-safety)
- **refault**: an async Rust framework that intercepts randomness and runs inside a deterministic simulation. — [docs.rs](https://docs.rs/crate/refault/0.2.0) (search snippet only)
- **antithesis_sdk** (Rust): see the Antithesis section above. — [docs](https://antithesis.com/docs/reference/sdk/rust/)

**Whole-machine determinism**
- Antithesis (hypervisor), Hermit, and rr make code deterministic without changing it. As noted above, Hermit and rr have limited fault injection and do not run on macOS or ARM. — [Eaton](https://notes.eatonphil.com/2024-08-20-deterministic-simulation-testing.html)

### Inferences

| Approach | Pros | Cons | Best for |
|---|---|---|---|
| Runtime replacement (madsim; turmoil plus single-threaded tokio) | Retrofits onto existing tokio code. Ecosystem shims exist for tonic, etcd, kafka, and S3. | Third-party crates that bypass the runtime (std time, threads, OS RNG) leak nondeterminism. Forked or patched dependencies must be maintained. Fault injection is limited to what the shims model. | Existing async services such as RisingWave and S2 |
| libc interception (madsim internals, mad-turmoil, LD_PRELOAD such as Turso's unreliable-libc) | Catches leaks in dependencies you don't control, such as HashMap seeds and `clock_gettime`. | Platform-specific (macOS needs `CCRandomGenerateBytes`), relies on link-order details, and fragile at teardown. | Plugging the gaps left by runtime replacement |
| Sans-IO state machines plus custom event loop (sled, Polar Signals, TigerBeetle, hiisi) | Full control. Failure injection written once. Nondeterminism is ruled out at compile time (no async in the core). Fastest simulation. | Design discipline is required from day one. The production driver is untested glue. Async dependencies are awkward. | New systems; very plausibly piton |
| Trait-abstracted runtime with a deterministic backend (commonware) | Async/await ergonomics kept, and no cfg magic. | Every dependency must go through the runtime traits. | New async codebases |
| loom / shuttle | Find memory-model and lock-ordering bugs in threaded code. | Not for distributed systems or IO. | Lock-free structures and caches |
| Hypervisor (Antithesis) | No code changes. Covers the whole system, including C dependencies. | Commercial and costs compute. You still write the workloads and assertions. | Whole-product system tests |

- Turso combines several of these: its own simulator and IO layer, an LD_PRELOAD fault-injection library, and Antithesis (see the next section).

### Gaps
- I did not verify tokio's own `unstable` deterministic options, such as `RngSeed` for `tokio::select!` and the runtime builder, from primary docs. S2 says it uses tokio's seedable internal RNG — [S2](https://s2.dev/blog/dst).
- I did not compare the maturity of moonpool, simulacra, or refault beyond their lib.rs listings.
- I found no primary-source benchmark comparing simulation throughput across tools.

## Exemplar Rust codebases using DST, and which directories to study

### Takeaway
The best Rust code to read:
- Turso's `testing/simulator` (property-based SQL simulator) and `testing/concurrent-simulator` ("Whopper")
- RisingWave's `src/tests/simulation` (the madsim harness)
- Neon's `libs/desim` and `safekeeper/tests/walproposer_sim` (a FoundationDB-style simulator for a consensus protocol)
- S2's `mad-turmoil`
- Commonware's `runtime/src/deterministic.rs`
- Materialize's `src/persist/src/turmoil.rs`

### Cited Findings
- **Turso (formerly Limbo; tursodatabase/turso).** `testing/` contains `simulator`, `concurrent-simulator`, `unreliable-libc`, `antithesis`, `differential-oracle`, `sqlancer`, and `stress`. The repository root also has `Dockerfile.antithesis`, `fuzz/`, and `tlaplus/` (verified with the GitHub API). — [repo](https://github.com/tursodatabase/turso/tree/main/testing)
  - `testing/simulator/` (`main.rs`, `model/`, `generation/`, `plan_to_test.rs`, `shrink/`, `runner/`, `profiles/`, `run-miri.sh`, `COVERAGE.md`):
    - It generates random configurations: workload distribution, page size, and numbers of readers and writers.
    - From those it builds "interaction plans" made of random SQL plus properties with assertions, for example two SELECTs that must return the same row count with and without the optimizer.
    - CLI flags: `--seed`, `--doublecheck` (runs the plan twice and compares output, a determinism check), `--differential` (compares Limbo against SQLite), `--load` (loads a plan from the bug base), and `--watch`.
    - Properties live in the `Property` enum.
    — [simulator README](https://github.com/tursodatabase/turso/blob/main/testing/simulator/README.md)
  - `testing/concurrent-simulator/` (crate `turso_whopper`) is a "deterministic concurrent simulator". It is driven by `SEED=... --mode ... --max-steps` and has modes for WAL and MVCC. Modules include:
    - `yield_injection.rs` and `allocation_fault.rs`
    - `chaotic_btree.rs`
    - `elle.rs` and `chaotic_elle.rs` (Jepsen's Elle checker)
    - `multiprocess.rs`, `properties.rs`, `io.rs`
    — [concurrent-simulator README](https://github.com/tursodatabase/turso/blob/main/testing/concurrent-simulator/README.md)
  - `testing/unreliable-libc/file.c` is an LD_PRELOAD shim. It injects `ENOSPC` and `EIO` with 1% probability each and short writes with 5% probability, using `drand48`. — [file](https://github.com/tursodatabase/turso/blob/main/testing/unreliable-libc/file.c)
  - A 2024 commit says the project uses Antithesis "in addition to our own DST tool". — [mirror commit](https://git.joshthomas.dev/mirrors/limbo/commit/be4014a1df53e148490f82c39e479085c77ceaea)
- **RisingWave.** `src/tests/simulation/` contains `src/` and `tests/`. In `src/`:
  - `cluster.rs`: the simulated multi-node cluster
  - `slt.rs`: the sqllogictest runner
  - `kafka.rs`: the Kafka simulator
  - `nexmark.rs` and `nexmark/`: workloads
  - `ctl_ext.rs`: control and rescheduling
  - TOML cluster configs such as `risingwave-scale.toml` and `backfill.toml`
  - (verified with the GitHub API) — [repo](https://github.com/risingwavelabs/risingwave/tree/main/src/tests/simulation)
- **Neon.** `libs/desim` is a "Discrete Event SIMulator" whose idea is borrowed from FoundationDB. Each node runs as a separate thread, and virtual time fast-forwards through idle periods. It was built to test the walproposer and safekeeper together. The README says it is deterministic and injects network failures. The harness is in `safekeeper/tests/walproposer_sim/` (`simulation.rs`, `safekeeper_disk.rs`, `walproposer_disk.rs`, `block_storage.rs`), and the tests are in `safekeeper/tests/random_test.rs` and `simple_test.rs`. — [libs/desim README](https://github.com/neondatabase/neon/tree/main/libs/desim), [walproposer_sim](https://github.com/neondatabase/neon/tree/main/safekeeper/tests/walproposer_sim)
- **S2.** The open-source piece is `s2-streamstore/mad-turmoil`. The S2 server's DST harness itself is not public, as far as I found. — [S2](https://s2.dev/blog/dst)
- **Materialize.** `src/persist/src/turmoil.rs` and `src/ore/src/netio/socket.rs` refer to turmoil, which shows turmoil-based simulation tests of the persist layer (paths found with GitHub code search). — [MaterializeInc/materialize](https://github.com/MaterializeInc/materialize/blob/main/src/persist/src/turmoil.rs)
- **foyer** (hybrid cache). GitHub code search finds `madsim` referenced in `foyer-tokio/src/lib.rs`, `foyer-common/src/spawn.rs`, `foyer-storage/src/io/engine/mod.rs`, and `xtask/src/main.rs`. That suggests a madsim-compatible build, likely because foyer is used inside RisingWave. — [foyer-rs/foyer](https://github.com/foyer-rs/foyer)
- **Commonware.** `runtime/src/deterministic.rs`, alongside the `tokio/` and `iouring/` backends. — [commonware TESTING.md](https://docs.rs/crate/commonware-runtime/latest/source/TESTING.md)
- **Polar Signals' new Rust database.** Closed source; the architecture is described only in the blog post and talks. — [Polar Signals](https://www.polarsignals.com/blog/posts/2025/07/08/dst-rust)
- **hiisi** (penberg/hiisi) and **MadRaft** (madsim-rs/madraft, Raft labs built on madsim) are small, readable teaching codebases. — [awesome-DST](https://github.com/ivanyu/awesome-deterministic-simulation-testing)

### Inferences
- For a new Rust workspace, the most transferable code to study:
  - The sans-IO architecture: hiisi and the Polar Signals post.
  - The workload, property, and shrinking machinery: Turso's `testing/simulator`.
  - The determinism self-check: Turso `--doublecheck`, S2's TRACE-log diff, and commonware's auditor state.
- I could not inspect foyer's and Materialize's usage in depth. The paths above come from code search, not a reading of the code.

### Gaps
- I did not confirm whether sled's current codebase still contains a simulator module. The guide is conceptual.
- I did not check Readyset, Fly.io (corrosion), or other candidates such as Rivet or Tonbo for DST harnesses.
- I found no public DST code from Polar Signals' Rust database.

## Controlling sources of nondeterminism in Rust: time, randomness, HashMap ordering, scheduling, tokio

### Takeaway
Every source of nondeterminism must come from the simulator: one seed, simulated time, a single-threaded executor, and simulated IO. Then verify by rerunning the same seed and diffing the logs. In Rust the usual leaks are:
- `std::time::Instant` and `SystemTime`
- OS entropy (`getrandom`, which also seeds `HashMap`'s `RandomState`)
- multi-threaded runtimes and `spawn_blocking`
- `tokio::select!` branch randomization
- real sockets and files
- timestamps inside wire protocols such as HTTP headers

### Cited Findings
- **S2's checklist.** Four sources to control:
  - execution: single-threaded scheduling
  - entropy: every RNG seeded
  - time: no physical clocks
  - IO: nothing outside the simulation
- **Tokio.** Use the current-thread runtime with a paused clock, where time advances only on `sleep`. Use the seedable internal RNG, which also governs `tokio::select!` branch choice. Code must use `tokio::time::Instant`, not `std::time::Instant`.
- **HashMap.** Its randomized hashing leaks entropy; see rust-fuzz/book#35 (https://github.com/rust-fuzz/book/issues/35). S2 fixed this by overriding `getrandom` and `getentropy` at the libc level.
- **Verification.** Mac and CI runs diverged. The cause was traced through TRACE logs to timestamps in HTTP packets. The fix is a CI meta-test that runs one seed twice and compares the TRACE logs. — [S2](https://s2.dev/blog/dst)
- **Inject dependencies.** Pass the clock and RNG in explicitly instead of reading globals, use async IO, and accept residual nondeterminism (CPU instructions, malloc) as part of a spectrum. — [Eaton](https://notes.eatonphil.com/2024-08-20-deterministic-simulation-testing.html)
- **Sans-IO answer.** Machines never read system time, and there is no async in the core trait. One PRNG is used for scheduling, ticks, and faults, and the seed is printed at the start of every run. — [Polar Signals](https://www.polarsignals.com/blog/posts/2025/07/08/dst-rust)
- **Turso double-check.** `--doublecheck` reruns the same plan and compares outputs. It can also run the simulator under Miri via `run-miri.sh`. — [Turso simulator README](https://github.com/tursodatabase/turso/blob/main/testing/simulator/README.md)
- **Commonware auditor.** Run twice with the same seed and assert that `auditor().state()` is equal. — [commonware TESTING.md](https://docs.rs/crate/commonware-runtime/latest/source/TESTING.md)
- **Patching dependencies.** madsim patches `getrandom` and `quanta` (a TSC-based clock that bypasses `clock_gettime`) with `[patch.crates-io]`. — [madsim README](https://github.com/madsim-rs/madsim)

### Inferences
- piton's existing lint policy already removes several leaks. `CLAUDE.md` and `clippy.toml` ban hash-ordered collections, which avoids `RandomState` ordering, and they ban unbounded channels. A sim-friendly next step would be to also ban `std::time::{Instant, SystemTime}`, `std::thread::spawn`, `rand::thread_rng`, and `tokio::spawn_blocking` in core crates through `clippy.toml` `disallowed-methods` and `disallowed-types`, and route time and RNG through injected traits. This is my recommendation, not something a source states.
- One point conflicts with piton's lints: S2 gates its expensive invariant checks behind `debug_assert!`, which piton bans. piton would need explicit `assert!` or a sim-only feature flag instead.

### Gaps
- I found no single authoritative Rust guide listing every nondeterminism leak. The material is spread across the S2, Polar Signals, and madsim sources.
- I did not verify current tokio APIs (`Builder::rng_seed`, `start_paused`) against primary docs in this session.

## Recent developments (2024–2026)

### Takeaway
DST in Rust has moved from niche to mainstream:
- S2 (2025) and Polar Signals (2025) published production write-ups.
- Polar Signals gave DST talks at FOSDEM 2026 and QCon London 2026.
- Turso built a family of simulators (simulator, Whopper, unreliable-libc) plus Antithesis.
- turmoil split into net, fs, and io_uring crates.
- New crates appeared: moonpool, simulacra, foundationdb-simulation, and the commonware deterministic runtime.
- Antithesis ships a Rust SDK.

### Cited Findings
- S2 DST post, April 2025 — [S2](https://s2.dev/blog/dst)
- Polar Signals Rust DST post, July 2025 — [Polar Signals](https://www.polarsignals.com/blog/posts/2025/07/08/dst-rust)
- FOSDEM 2026 Rust DST talk — [FOSDEM](https://fosdem.org/2026/schedule/event/rust-deterministic-simulation-testing)
- QCon London, March 2026 — [QCon](https://qconlondon.com/presentation/mar2026/deterministic-simulation-testing-dst-journey-wasm-go-state-machines-rust)
- turmoil companion crates: turmoil-net, turmoil-fs, turmoil-io-uring — [turmoil](https://github.com/tokio-rs/turmoil)
- Turso's Whopper concurrent simulator, Elle checking, yield injection, and Miri support — [Turso concurrent-simulator](https://github.com/tursodatabase/turso/blob/main/testing/concurrent-simulator/README.md), [Turso simulator](https://github.com/tursodatabase/turso/blob/main/testing/simulator/README.md)
- moonpool 0.8.0 (July 2026), simulacra 0.1.0 (May 2026), foundationdb-simulation 0.3.0 (June 2026) — [lib.rs moonpool](https://lib.rs/crates/moonpool), [simulacra](https://lib.rs/crates/simulacra), [foundationdb-simulation](https://lib.rs/crates/foundationdb-simulation)
- commonware-runtime 2026.7.0 deterministic runtime — [docs.rs](https://docs.rs/crate/commonware-runtime/latest/source/TESTING.md)
- Antithesis Rust SDK — [docs](https://antithesis.com/docs/reference/sdk/rust/)
- BugBash 2025, Antithesis's reliability conference — [Zemb](https://pierrezemb.fr/posts/learn-about-dst/)

### Inferences
- Shared trends across these:
  - Teams use more than one technique. Turso pairs its own simulator with Antithesis, and S2 pairs turmoil with libc overrides.
  - Determinism is checked automatically: double-run diffs and auditor-state comparison.
  - Correctness checking is moving from ad-hoc assertions to history checkers such as Elle and linearizability checkers.

### Gaps
- I did not check RustConf, EuroRust, or P99 CONF 2024–2026 programs for DST talks, beyond FOSDEM and QCon.
- I did not find whether madsim is still actively developed in 2025–2026; its README shows no releases. docs.rs reports 0.2.34.
- I did not fetch Hacker News discussions.
