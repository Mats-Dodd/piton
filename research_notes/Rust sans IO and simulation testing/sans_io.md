# Sans-IO / poll-based state machine protocol design in Rust

Research date: 2026-10-18. About 16 tool calls. Each link below was fetched or returned by search unless it is listed under Gaps.

## What are the canonical writeups?

### Takeaway
Thomas Eizinger's Firezone post (July 2024) is the canonical Rust writeup. The Python sans-io.readthedocs.io site and Cory Benfield's PyCon US 2016 talk are the foundational originals. The 2025–2026 discussion adds a skeptical counterpoint ("sans-io: meh"), fasterthanlime's pro-sans-io video, and Firezone's follow-up on DST and fuzzing.

### Cited Findings
**Foundational (non-Rust)**
- **sans-io.readthedocs.io.** Defines a sans-I/O library as one that "implements a network protocol by operating directly on bytes or text" with no I/O. Code that merely abstracts I/O does not qualify. The stated motivation is reuse across sync and async code: the caller drives the network, and the library imposes no I/O API. It links Cory Benfield's PyCon US 2016 talk. Every listed implementation is Python, including h11, hyper-h2, wsproto, aioquic, matrix-nio and jeepney. — [sans-io.readthedocs.io](https://sans-io.readthedocs.io/)

**Rust canonical**
- **"sans-IO: The secret to effective Rust for network services"** by Thomas Eizinger, Firezone blog, 2 July 2024. The main claim: "our protocols are implemented as pure state machines" (connlib/snownet). — [Firezone blog](https://www.firezone.dev/blog/sans-io)
  - The post frames sans-IO as dependency inversion: "policies (what to do) should not depend on implementation details (how to do it)". Sends become a `Transmit` value (destination plus payload) instead of a socket call. — [Firezone](https://www.firezone.dev/blog/sans-io)
  - Its worked example is a STUN binding client written three ways: tokio, blocking std, and sans-IO. The sans-IO API has `handle_input(packet, now)`, `poll_transmit()`, `poll_timeout() -> Option<Instant>`, `handle_timeout(now)` and a query method, `public_address()`. A timer then re-sends the binding request every 5 seconds. — [Firezone](https://www.firezone.dev/blog/sans-io)
  - On time injection: "every function that needs to know the current time receives an `Instant` parameter instead of calling `Instant::now` itself". The post calls boringtun only partly sans-IO because it calls `Instant::now` internally. — [Firezone](https://www.firezone.dev/blog/sans-io)
  - Ownership argument: the state machine needs no `'static` spawned futures, no `Arc<Mutex<T>>` and no channels; `&mut self` is enough. — [Firezone](https://www.firezone.dev/blog/sans-io)
  - Cross-references: quinn/quinn-proto, quiche, str0m (`IceAgent`), rustls, tungstenite, proptest. — [Firezone](https://www.firezone.dev/blog/sans-io)
- **"Let futures be futures"** by withoutboats, 3 Feb 2024. It argues against `maybe(async)` as a general fix and proposes sans-IO libraries ("handle no IO and only implement state machines") as one alternative, citing str0m and Cloudflare's quiche. It also notes that async Rust compiles each task into a state machine "very similar to hand-written asynchronous servers in a language like C". — [without.boats](https://without.boats/blog/let-futures-be-futures/)
- **"The case for sans-io"** by fasterthanlime (Amos Wenger), a video posted around 7 Feb 2025. It is listed on r/fasterthanlime and referenced by the SDR podcast. Amos maintains rc-zip, a sans-io zip implementation. — [SDR podcast show notes](https://sdr-podcast.com/episodes/sans-io/); [r/fasterthanlime listing](https://axebps-redlib.hf.space/r/fasterthanlime)
- **"sans-io: meh"**, Self-Directed Research podcast S2, 11 June 2025, with James Munns and Amos Wenger. This is the main critique (details under Critiques). — [SDR podcast](https://sdr-podcast.com/episodes/sans-io/)
- **"Finding bugs you didn't think to test for"** by Thomas Eizinger, Firezone blog. The page is dated 25 Sep 2026. It extends sans-IO into deterministic simulation, proptest state machines and coverage-guided fuzzing. — [Firezone blog](https://www.firezone.dev/blog/finding-bugs-you-didnt-think-to-test-for)
- **Firezone architecture docs** point readers to the sans-IO post as the design rationale for connlib. — [Firezone KB: Tech stack](https://www.firezone.dev/kb/architecture/tech-stack)

### Inferences
- Suggested reading order: sans-io.readthedocs.io for the definition, then the Firezone sans-IO post for the Rust API shape and testing, then withoutboats for where it fits in async Rust, then the SDR podcast for the counterargument, then Firezone's 2026 fuzzing post for the payoff.

### Gaps
- I did not fetch the exact title or video URL of Cory Benfield's PyCon 2016 talk. From memory it is "Building Protocol Libraries the Right Way"; verify before citing.
- I did not fetch the YouTube URL for fasterthanlime's "The case for sans-io" or its content.
- I did not locate the HN or lobste.rs threads on the Firezone post (search returned only fragments).
- I found no RustConf or EuroRust talk specifically on sans-IO.

## Which Rust crates are exemplars, and what does the core API look like?

### Takeaway
They share one API shape. Inputs are `handle_*(…, now)`. Outputs are drained with `poll_transmit` / `poll_event` / `poll_timeout`. The caller owns sockets, clocks and buffers, and time is always an `Instant` argument. quinn-proto, str0m and Firezone's connlib/snownet are the clearest exemplars. rustls (unbuffered API), matrix-sdk-crypto and webrtc-rs/rtc are notable newer or adjacent ones.

### Cited Findings
**quinn-proto (QUIC), split from quinn (the tokio shell)**
- The crate describes itself as a deterministic QUIC implementation with no networking code; the application supplies timestamps and I/O. `Endpoint` dispatches datagrams to `Connection`s, and `Connection` holds most of the protocol logic. — [docs.rs quinn-proto](https://docs.rs/quinn-proto)
- `Connection` API in v0.11.19: `handle_event(ConnectionEvent)`, `handle_timeout(now: Instant)`, `poll_timeout() -> Option<Instant>`, `poll_transmit(now, max_datagrams, buf: &mut Vec<u8>) -> Option<Transmit>` (with `max_datagrams` for GSO batching), `poll_endpoint_events()`, and `poll() -> Option<Event>`. — [docs.rs Connection](https://docs.rs/quinn-proto/latest/quinn_proto/struct.Connection.html)
- Ordering contract: time must be monotonic. Events and timeouts at the same instant may interleave in any order. Recommended polling order is transmit, timeout, endpoint events, app events, and `poll_timeout` must follow `poll_transmit`. Extra `handle_timeout` calls are harmless no-ops. — [docs.rs Connection](https://docs.rs/quinn-proto/latest/quinn_proto/struct.Connection.html)
- Lesson: a two-level design (Endpoint and Connection, linked by `EndpointEvent` and `ConnectionEvent`) shows how to compose sans-IO machines. Caller-provided buffers and GSO batching show sans-IO is not inherently slow.

**str0m (WebRTC, from the ureq author's org)**
- `Rtc` does no networking and has no internal threads or tasks. The loop is: wait for a timeout, packet or mutation; feed it via `handle_input` or an API mutation; then drain `poll_output()` until `Output::Timeout`. Outputs are `Output::Transmit` (send on the socket), `Output::Event` (to the app) and `Output::Timeout` (next deadline). — [docs.rs str0m](https://docs.rs/str0m)
- The docs warn you must drain to `Timeout` between mutations, or the engine is left in an inconsistent state. Time is input only, which enables faster-than-realtime tests. — [docs.rs str0m](https://docs.rs/str0m)
- Lesson: a single `poll_output` enum instead of several `poll_*` methods. It shows the "drain until timeout" invariant explicitly. withoutboats cites it as an exemplar. — [without.boats](https://without.boats/blog/let-futures-be-futures/)

**Firezone connlib / snownet (ICE + WireGuard)**
- Protocols are pure state machines; an outer event loop executes side effects. Firezone notes connlib is built on tokio but uses almost no `tokio::spawn`, and multiplexes over a single UDP socket. — [Firezone blog](https://www.firezone.dev/blog/sans-io); [secondhand summary](https://segmentfault.com/p/1210000047068090)
- Test-harness files to study, under `rust/tests/fuzz/src/`: `reference.rs` (`ReferenceState`), `transition.rs` (`Transition`), `assertions.rs`, `sut.rs`, `sim_net.rs`, `buffered_transmits.rs`, `arb/topology.rs`, `arb/values.rs`. The entry point shown is `TunnelTest::init_test`. — [Firezone 2026 post](https://www.firezone.dev/blog/finding-bugs-you-didnt-think-to-test-for)
- Related PRs show the reference model evolving, e.g. "model NAT for domains resolved after connecting" (#15498) and "model TCP connections as flows" (#15613). — [PR #15498](https://github.com/firezone/firezone/pull/15498); [PR #15613](https://github.com/firezone/firezone/pull/15613)

**rustls unbuffered API**
- Entry points are `UnbufferedClientConnection::new`, `UnbufferedServerConnection::new` and `process_tls_records(&mut [u8]) -> UnbufferedStatus`. The caller owns all buffers and I/O, and no `std::io` traits are required, so it works in `no_std`. `UnbufferedStatus { discard, state }`: the caller must drop `discard` bytes before calling again. — [rustls unbuffered.rs source](https://codebrowser.dev/slint/crates/rustls/src/conn/unbuffered.rs.html); [UnbufferedClientConnection docs](https://doc.servo.org/rustls/client/struct.UnbufferedClientConnection.html)
- Ergonomic critique: issue #1958 (May 2024) argues the `ConnectionState` design fits poorly with interleaved read/write after the handshake, because callers must create new handles on each call. — [rustls#1958](https://github.com/rustls/rustls/issues/1958)

**matrix-sdk-crypto (OlmMachine)**
- Described as a no-network-IO state machine for Matrix E2EE. It uses push/pull: feed in sync responses, call `outgoing_requests()`, send them over HTTP yourself, then call `mark_request_as_sent()`. Unmarked requests are retried. — [docs.rs matrix-sdk-crypto](https://docs.rs/matrix-sdk-crypto/); [Thunderbird OutgoingRequestsManager.js](https://searchfox.org/comm-central/source/chat/protocols/matrix/lib/matrix-sdk/rust-crypto)
- Lesson: sans-IO works for request/response (HTTP) protocols as well as datagrams. The "outgoing request queue plus ack" variant suits that model.

**webrtc-rs/rtc and the `sansio` crate (2025–2026)**
- webrtc-rs rewrote its stack as the sans-I/O crate `rtc`. v0.7.0 claims feature parity with the async `webrtc` crate. The API has eight methods: outputs `poll_write`, `poll_read`, `poll_event`, `poll_timeout`, and inputs `handle_read`, `handle_write`, `handle_event`, `handle_timeout`. Each layer (ICE, DTLS, SRTP/SCTP, interceptors) implements a shared `sansio::Protocol` trait to form a pipeline. The author says `sansio` is at v1.0. — [Show HN: webrtc-rs/rtc](https://news.ycombinator.com/item?id=46570584); [lib.rs sansio](https://lib.rs/crates/sansio)
- `rtc-sctp`'s `Association` uses the same `poll_timeout` / `handle_timeout` / `poll_transmit` contract as quinn-proto. — [docs.rs rtc-sctp Association](https://docs.rs/rtc-sctp/0.20.0/rtc_sctp/struct.Association.html)

**Smaller or experimental**
- `wireguard-sans-io`: the caller supplies datagrams, time and entropy. — [docs.rs](https://docs.rs/wireguard-sans-io/latest/wireguard_sans_io/)
- `asansio`: an experiment that uses async/await to generate the sans-IO state machine. It is `no_std` and does not heap-allocate. — [docs.rs asansio](https://docs.rs/asansio/latest/asansio/); [Rust forum announcement](https://users.rust-lang.org/t/asansio-experimental-crate-for-async-await-sans-i-o/134480)
- rc-zip (Amos Wenger) is a sans-io zip reader. — [SDR podcast notes](https://sdr-podcast.com/episodes/sans-io/)
- Cloudflare quiche (QUIC) is cited as sans-IO by both Firezone and withoutboats. — [Firezone](https://www.firezone.dev/blog/sans-io); [without.boats](https://without.boats/blog/let-futures-be-futures/)

### Inferences
- A common API vocabulary has emerged: `handle_input` / `handle_timeout(now)` / `poll_transmit` / `poll_timeout` / `poll_event`. Callers should expect two contracts: drain all outputs after every input, and time is monotonic and caller-supplied.
- There are two styles of output API: several `poll_*` methods (quinn-proto, Firezone, webrtc-rs/rtc) or a single output enum (str0m). The enum makes "drain until Timeout" explicit and easier to get right.
- The IO shell is typically a small `select!` / mio / tokio loop around one socket plus one sleep timer, e.g. `quinn` around `quinn-proto`. I did not verify specific shell files.

### Gaps
- I did not verify these exemplars this session, so their details come from memory: s2n-quic (its own provider-based IO abstraction, arguably not classic sans-IO), rust-libp2p `NetworkBehaviour` (poll-based, with `on_swarm_event` / `poll` returning `ToSwarm` actions; the Swarm is the shell), iroh (built on quinn-proto/noq), httparse (a pure sans-IO parser), h2/hyper (not sans-IO; tied to their own IO traits). Verify before citing.
- I did not verify file paths inside quinn (e.g. `quinn-proto/src/connection/mod.rs`, `quinn/src/connection.rs`) or str0m (`src/lib.rs` loop docs, `src/ice/` IceAgent).

## How does sans-IO relate to deterministic simulation testing?

### Takeaway
Sans-IO makes deterministic simulation testing (DST) nearly free. With no clocks, sockets or spawned tasks, a single-threaded harness can wire many state machines together, control time and message ordering from a seed, inject faults, and compare against a reference model. The alternative is a deterministic runtime such as madsim, which needs no redesign but gives less control.

### Cited Findings
- **Firezone 2024.** Tests advance time by minutes instantly and pass `Transmit`s directly between agents. A proptest state-machine test compares connlib against a reference state machine, sampling thousands of scenarios per CI run. — [Firezone sans-IO](https://www.firezone.dev/blog/sans-io)
- **Firezone 2026.** Clients, gateways and relays run in a simulated network. Generated `Transition`s (`EditResource`, `RoamClient`, `ConnectTcp`) drive both a `ReferenceState` and the system under test, with invariants checked each step. About 500 replayed scenarios reach over 85% coverage in under 2 minutes. A nightly 30-minute coverage-guided fuzz job has a bot open PRs adding corpus inputs. — [Firezone 2026](https://www.firezone.dev/blog/finding-bugs-you-didnt-think-to-test-for)
  - Quoted rationale: the harness itself is a state machine because writing these tests by hand would be too verbose. Debug assertions act as fuzzer targets. — [Firezone 2026](https://www.firezone.dev/blog/finding-bugs-you-didnt-think-to-test-for)
  - Stated costs: dependencies must also be sans-IO or written in-house. The side-effect layer still needs Docker end-to-end and QA tests. Getting the reference model right is hard, and some mismatches turn out to be model bugs rather than code bugs. — [Firezone 2026](https://www.firezone.dev/blog/finding-bugs-you-didnt-think-to-test-for)
- **Polar Signals, "DST in Rust: A Theater of State Machines"** (Alfonso Subiotto Marqués, 8 Jul 2025). Every component implements `trait StateMachine { fn receive(&mut self, m: Message) -> Option<Vec<(Message, Destination)>>; fn tick(&mut self, curtime: Instant) -> Option<Vec<(Message, Destination)>>; }`. A single-threaded message bus is the only scheduler. One printed seed drives ordering, tick selection and fault injection, and faults are injected in one place (the bus) rather than per interface. — [Polar Signals blog](https://blog.polarsignals.com/blog/posts/2025/07/08/dst-rust)
  - They considered madsim, a deterministic tokio drop-in, and call it "probably a good choice" for existing codebases. They chose state machines because they were writing a new database. Downsides: developers drift logic into the untested production driver, and non-deterministic dependencies (e.g. UUIDs) must be mocked. The post never says "sans-io", but the design is the same idea. — [Polar Signals blog](https://blog.polarsignals.com/blog/posts/2025/07/08/dst-rust)
  - Follow-up talks: FOSDEM 2026 "Random Seeds and State Machines" (Subiotto and Branczyk) and QCon London 2026, on moving from DST via WASM in Go to state machines in Rust. — [FOSDEM 2026](https://fosdem.org/2026/schedule/event/rust-deterministic-simulation-testing); [QCon London 2026](https://qconlondon.com/presentation/mar2026/deterministic-simulation-testing-dst-journey-wasm-go-state-machines-rust)
- proptest-state-machine (reference model plus SUT, in the style of Erlang's eqc_statem) runs transitions sequentially only. — [docs.rs proptest-state-machine](https://docs.rs/proptest-state-machine/^0.1.0); [tzemanovic overview](https://hackmd.io/@tzemanovic/B12jdEk8p)

### Inferences
- There are two routes to DST in Rust. You can design sans-IO state machines and get determinism by construction (Firezone, Polar Signals), or keep async code and swap the runtime (madsim, and probably turmoil). Sans-IO costs more upfront but gives finer control over time, ordering and faults, and it composes with coverage-guided fuzzing.
- For a new codebase (which the piton workspace is), sans-IO plus a seed-driven single-threaded harness is the pattern with the best documented results.

### Gaps
- I did not research turmoil (tokio-rs) or other DST runtimes. Comparing them is probably another researcher's scope.
- No published bug counts from Firezone's approach.

## Critiques, tradeoffs, and limitations

### Takeaway
The main costs are ergonomic. Hand-written enum state machines are verbose for sequential workflows; async generates the same state machine automatically. Getting the event loop wrong causes subtle bugs, sans-IO dependencies are scarce, and the IO shell still needs separate testing. The strongest skeptic (James Munns) prefers async with well-defined frame-level interfaces. Generators may eventually narrow the gap.

### Cited Findings
- **Firezone's own caveats.** A hand-written event loop can busy-loop if `poll_timeout` is not advanced. Sequential workflows need more code than `async`. The pattern is "not particularly wide-spread (yet)", so few libraries follow it. — [Firezone](https://www.firezone.dev/blog/sans-io)
- **James Munns ("sans-io: meh", 2025).** He used to recommend sans-io but now finds compiler-generated async state machines clearer than hand-written enums. Hand-written enums cause nested matches, borrow-checker friction and placeholder values. He finds async well suited to timeouts, racing and cancellation. postcard-rpc uses frame-level wire traits instead, tested over in-memory channels. — [SDR podcast](https://sdr-podcast.com/episodes/sans-io/)
- **Amos Wenger's counterpoints** (same episode). He defends sans-io via rc-zip and notes tokio's `AsyncRead` does not fit io-uring buffer ownership, which sans-io sidesteps. He also lists async's weak spots: poor stack traces, awkward recursion and large future sizes. He argues what James really wants is (unstable) generators. Both hosts agree that defining the interface you need is the core value. — [SDR podcast](https://sdr-podcast.com/episodes/sans-io/)
- **rustls unbuffered API ergonomics.** Issue #1958 says it is awkward for interleaved read/write. Harbor task write-ups from March 2026 describe edge-case bugs: data sent after the peer half-closes, and records deframed after close_notify. — [rustls#1958](https://github.com/rustls/rustls/issues/1958); [Harbor task 2332](https://hub.harborframework.com/tasks/abundant/rustls__rustls-2332); [Harbor task 2576](https://hub.harborframework.com/tasks/abundant/rustls__rustls-2576)
- **str0m's misuse hazard.** Mutating twice without draining to Timeout leaves the engine inconsistent, so the protocol contract is easy to break from the caller side. — [docs.rs str0m](https://docs.rs/str0m)
- **DST maintenance cost.** Logic drifts into the untested driver, and dependencies must be mocked or wrapped. — [Polar Signals](https://blog.polarsignals.com/blog/posts/2025/07/08/dst-rust)
- **withoutboats.** Sans-IO is one answer to the sync/async split, not a replacement for futures; futures should be used for intra-task concurrency where they shine. — [without.boats](https://without.boats/blog/let-futures-be-futures/)

### Inferences
- Use sans-IO when the logic is a long-lived protocol or controller with timers, many peers and failure cases (QUIC, ICE, TLS, consensus, replication), when you need sync and async (or no_std/WASM) users, or when you want DST. Avoid it, or use async with frame-level traits, for mostly sequential request/response glue where explicit states add boilerplate without testing benefit.
- `asansio` and future stable generators point toward a hybrid: write sequential code, get a sans-IO machine.

### Gaps
- I did not find the HN or lobste.rs critique threads on the Firezone post. The search snippets were truncated.
- No quantitative performance comparison of sans-IO and async implementations. The webrtc-rs HN thread asked for CPU benchmarks and got none.

## 2024–2026 developments

### Takeaway
Between 2024 and 2026 the idea went from niche to named: Firezone's 2024 post popularized the term in Rust, and the 2025 debate (fasterthanlime video, SDR podcast) followed. In 2025–2026 came stable shared traits (`sansio` 1.0), full-stack rewrites (webrtc-rs/rtc), and DST-plus-fuzzing results (Polar Signals, Firezone).

### Cited Findings
- July 2024: Firezone sans-IO post. — [Firezone](https://www.firezone.dev/blog/sans-io)
- Feb 2024: withoutboats endorses sans-IO as an alternative to `maybe(async)`. — [without.boats](https://without.boats/blog/let-futures-be-futures/)
- Feb 2025: fasterthanlime video "The case for sans-io". — [r/fasterthanlime listing](https://axebps-redlib.hf.space/r/fasterthanlime)
- June 2025: SDR "sans-io: meh" episode. — [SDR podcast](https://sdr-podcast.com/episodes/sans-io/)
- July 2025: Polar Signals DST state-machine post, followed by FOSDEM 2026 and QCon London 2026 talks. — [Polar Signals](https://blog.polarsignals.com/blog/posts/2025/07/08/dst-rust); [FOSDEM 2026](https://fosdem.org/2026/schedule/event/rust-deterministic-simulation-testing)
- webrtc-rs/rtc v0.7.0 reaches feature parity with async webrtc on a shared `sansio::Protocol` trait, and the `sansio` crate is at 1.0. The HN post shows January releases with no year; it was likely Jan 2026, but this is unconfirmed. — [Show HN](https://news.ycombinator.com/item?id=46570584)
- `asansio` (async/await-generated sans-IO) was announced on the Rust users forum. — [forum](https://users.rust-lang.org/t/asansio-experimental-crate-for-async-await-sans-i-o/134480)
- Sep 2026: Firezone "Finding bugs you didn't think to test for", covering DST, proptest and coverage-guided fuzzing on sans-IO connlib. — [Firezone 2026](https://www.firezone.dev/blog/finding-bugs-you-didnt-think-to-test-for)

### Inferences
- The trend is from "sans-IO for reuse" (the original Python motivation) to "sans-IO for testability and DST" (the Rust 2024–2026 emphasis).

### Gaps
- I could not verify exact dates for the webrtc-rs/rtc releases or the asansio announcement.
