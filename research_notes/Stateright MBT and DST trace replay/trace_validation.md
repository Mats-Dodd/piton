# Trace Validation: replaying implementation traces through an executable model (for piton: DST traces -> Stateright)

Scope: theory and practice of trace validation / trace checking, and how to apply it to piton's single-threaded sans-IO core (`Session::handle(now, Input)` / `poll_submission()`), with a Stateright (Rust) model. Sources fetched 2026-10-18. Items marked UNVERIFIED were not confirmed against a primary source in this session.

## Q1. Key papers and writeups, and what they learned

### Takeaway
Every serious attempt found real discrepancies (both spec bugs and implementation bugs). What separates success (CCF, etcd, Cirstea et al.) from failure (MongoDB 2020) is (a) whether the spec was written or refined to match implementation atomicity, (b) whether the logged events are cheap to observe consistently (messages and state transitions, not snapshots of multithreaded internals), and (c) search tooling that copes with partial traces (DFS, action composition). The model-to-implementation direction (test-case generation) was consistently cheaper and higher-coverage when the component is self-contained.

### Cited Findings

**Cirstea, Kuppe, Loillier, Merz — "Validating Traces of Distributed Programs Against TLA+ Specifications" (arXiv 2404.16075, Apr 2024, rev. Sep 2024)**
- Reduces trace conformance to a constrained model-checking problem solved by TLC. Java instrumentation API records updates to spec variables (not full values), and developers can trace only some variables. Discrepancies were found in every case study — [arXiv abs](https://arxiv.org/abs/2404.16075)
- Trace format: newline-delimited JSON (`ndJsonDeserialize`), one entry per event, with a `clock`, per-variable update ops, and optional `event` and `event_args`. Each process writes its own file, and Python scripts merge them before TLC validation. Clocks can be in-memory, file-based, or server-based. Only scalar logical clocks are supported ("vector clocks can easily be added") — [arXiv HTML v2](https://arxiv.org/html/2404.16075v2)
- API: `notifyChange(var, path, op, args)` records a variable update. `log(eventName, args)` emits one entry covering all changes since the previous `log` — [arXiv HTML v2](https://arxiv.org/html/2404.16075v2)
- TLA+ side: a trace spec with a line counter `l`. `IsEvent(e)` advances `l`, matches the event name, and applies the logged updates. Each spec action is conjoined with it, e.g. `IsTMCommit == IsEvent("TMCommit") /\ TMCommit`. The acceptance condition is `TraceAccepted == TLCGet("stats").diameter - 1 = Len(Trace)` — [arXiv HTML v2](https://arxiv.org/html/2404.16075v2)
- Case-study discrepancies:
  - Two-phase commit: a counter-based TM counted a re-sent "Prepared" message twice and committed prematurely.
  - EWD998: the implementation let a node send a message to itself, which the spec forbids. Grain-of-atomicity mismatches also appeared, and nodes kept passing the token after termination — [arXiv HTML v2](https://arxiv.org/html/2404.16075v2)
- Logging-level experiment (2PC, KV store): logging full variables plus events explores the fewest states. "Logging only the variables or only the event names quickly leads to state-space explosion that makes trace validation infeasible." Events with arguments sufficed for the KV store because the arguments determine the variable values — [arXiv HTML v2](https://arxiv.org/html/2404.16075v2)

**Howard, Kuppe, Ashton, Chamayou, Crooks — "Smart Casual Verification of the Confidential Consortium Framework" (NSDI 2025; arXiv 2406.17455)**
- Binds a TLA+ spec to the C++ implementation of CCF's consensus and client-consistency model, runs in CI, and found 6 subtle bugs before production — [arXiv abs](https://arxiv.org/abs/2406.17455)
- Instrumentation:
  - About 15 log statements at side-effect-free linearization points (send/receive of messages, changes of high-level node state), taking about one day to add.
  - Traces come from a deterministic scenario driver with a single global clock replacing wall clocks.
  - Only constant-size values are logged (e.g. log *lengths*, not log entries).
  - Logging is compiled out of production builds — [arXiv HTML](https://arxiv.org/html/2406.17455)
- The separate Trace spec (369 LoC) reuses consensus-spec actions. Each trace action is enabled only when the current event matches and is parameterized by trace values. An `IsFault` action is composed in to cover unlogged message loss. Term update and AppendEntries handling were composed into one atomic action. The network became a multiset to allow resends. Stuttering covers multi-event high-level steps — [arXiv HTML](https://arxiv.org/html/2406.17455)
- DFS was implemented in TLC because one accepting behaviour suffices. Validating against the consistency spec took "less than a second using DFS, compared to about an hour with BFS" — [arXiv HTML](https://arxiv.org/html/2406.17455)
- Which technique found which bugs:
  - Trace validation: commit advance on AE-NACK (a serious safety bug), truncation from an early AE, inaccurate AE-ACK.
  - Exhaustive model checking: an election quorum-tally bug, after 48h on 128 cores.
  - Simulation: two further bugs — [arXiv HTML](https://arxiv.org/html/2406.17455)
- Effort:
  - Trace spec: about 2 engineer-months over 4 months, including the TLC enhancements (action composition, DFS, debugger, visualization). The consensus spec saw 107 changes and the trace spec 88 commits.
  - Consistency trace validation: about 1 engineer-week, observed through the REST API with no source instrumentation — [arXiv HTML](https://arxiv.org/html/2406.17455)
- The TLA+ wiki adds that CCF trace validation found "two significant data-loss issues" from about a dozen executions — [docs.tlapl.us trace validation](https://docs.tlapl.us/using:tlc:trace_validation)

**Davis, Hirschhorn, Schvimer — "eXtreme Modelling in Practice" (PVLDB 13(9), 2020; arXiv 2006.00915)**
- How MBTC on RaftMongo worked:
  - A C++ `logTlaPlusTraceEvent`, enabled only in testing, logged 4 variables as JSON.
  - Cross-process order came from running on one machine and sleeping until the millisecond timestamp changed.
  - A Python post-processor merged and sorted the logs, then rebuilt whole-replica-set states.
  - Pressler's method generated a `Trace.tla` for TLC.
  - Two engineers built this in 10 weeks — [arXiv HTML](https://arxiv.org/html/2006.00915v1)
- Why it failed:
  - Spec too abstract (arbiters, initial sync counting toward quorum, two leaders, term gossip). One violation came "only 4 steps from the trace's start and left the remaining 2,683 steps unchecked." 252 of 345 spec lines had to change.
  - Consistent snapshots of multithreaded state took about a month for 7 transitions.
  - Pressler's method was "impractically slow" for thousands of events.
  - Of 423 tests, 120 failed due to tracing incompatibilities, and only 1 handwritten test passed trace checking. The project was abandoned after 10 weeks — [arXiv HTML](https://arxiv.org/html/2006.00915v1)
- MBTCG on Realm Sync succeeded:
  - About 4 weeks of effort: a 795-line spec plus a 755-line Go generator that parsed TLC's state graph.
  - It produced 4,913 C++ tests with 100% branch coverage of 86 branches, against 21% for handwritten tests and 92% for AFL after about 8M executions.
  - TLC found a non-terminating merge case (ArraySwap/ArrayMove) that existed in the C++ code too — [arXiv HTML](https://arxiv.org/html/2006.00915v1)
- Advice from the paper:
  - "No initial infrastructure investment can make the marginal cost of trace-checking additional specifications cheap."
  - "MBTCG is the gold standard" when the state space is manageable and the implementation is self-contained.
  - Post-process "early and often."
  - "Bugs in post-processing could mask implementation bugs" — [arXiv HTML](https://arxiv.org/html/2006.00915v1)
- 2025 retrospective blog by Davis: "We'd ensure the spec and implementation conform at the start… We'd model easily observed events like network messages, to avoid snapshotting the internal state of a multithreaded process." He would write the spec first, "with trace-checking in mind." Trace-checking "should extend easily to multiple specs" — [MongoDB blog](https://www.mongodb.com/company/blog/engineering/conformance-checking-at-mongodb-testing-our-code-matches-our-tla-specs)

**etcd-io/raft PR #113 "TLA+ Trace validation" (Joshua Zhang; merged 2024-04-18)**
- Based on the CCF approach:
  - Trace log points record algorithm-relevant state and transitions, e.g. `traceCommit` in `maybeCommit`.
  - The real logger is in `state_trace.go`, with a no-op `state_trace_nop.go` for normal builds.
  - `Traceetcdraft.tla` replays the traces; `validate.sh`, `example.ndjson` and a README are in `tla/` — [PR #113](https://github.com/etcd-io/raft/pull/113)
- Reviewer concerns:
  - Per-node traces lose cross-node interleaving and can hide bugs such as two leaders in one term, so a central merged trace was proposed.
  - Crash/restart can desync trace logs from the WAL (deferred).
  - Others could not run the scripts at first.
  - Follow-ups: PR #192 (trace generation for tests) and PR #204 (a trace validation test) — [PR #113](https://github.com/etcd-io/raft/pull/113)
- The model was later used to reproduce etcd durability issue #14370 — [PR #113](https://github.com/etcd-io/raft/pull/113)

**TraceLink — Hackett & Beschastnikh, "TraceLinking Implementations with Their Verified Designs" (OOPSLA 2025, PACMPL 9 OOPSLA2, Art. 350)**
- A push-button trace validation tool for PGo (Modular PlusCal → TLA+ and Go). It auto-instruments the generated Go code and auto-generates the TLA+ validation setup. Prior TLA+ trace validation "requires significant manual work."
- It uses vector-clock causal tracing and considers multiple causally allowed interpretations of a trace.
- "Sidestep" path validation shortens counterexamples by 29%. Templatization compresses the generated TLA+ 93×.
- It found 9 bugs in PGo's trusted computing base (1 compiler miscompilation, 2 inaccurate network models, 2 runtime instrumentation bugs, 2 timeout behaviours, 1 misconfigured failure detector, 1 misplaced abstraction) — [paper PDF](https://www.cs.ubc.ca/~bestchai/papers/oopsla25-trace-link.pdf); [SPLASH page](https://2025.splashcon.org/details/OOPSLA/149/TraceLinking-Implementations-with-their-Verified-Designs)

**OmniLink — Hackett, Wrench, Wei, Beschastnikh, Macko, Davis (arXiv 2601.11836, Jan 2026)**
- Validates unmodified concurrent systems. Each call is a black box with a start/end "timebox", and TLC searches for a total order consistent with the intervals (linearizability-style) — the same idea as Porcupine, but against a TLA+ spec.
- On WiredTiger it found 2 implementation issues and 2 spec mismatches — [arXiv HTML](https://arxiv.org/html/2601.11836v1)
- Its related-work taxonomy: test generation (Mocket, SandTable, …), trace checking (Cirstea, Davis, Howard, Niu/ZooKeeper, TraceLink), linearizability checking (Porcupine), runtime verification (Choreographic PlusCal) — [arXiv HTML](https://arxiv.org/html/2601.11836v1)

**AWS P / PObserve**
- PObserve checks service logs from integration tests or production against P specification monitors, with "no additional instrumentation."
- Pipeline: a user-written log parser produces time-stamped events, a sequencer merges them into one globally sorted stream *by timestamp*, and a demultiplexer sends each event to the spec monitors that observe it (implemented with Java Streams) — [PObserve docs](https://p-org.github.io/P/advanced/pobserve/pobserve/)
- PObserve checks *properties/monitors* over the event stream rather than conformance to a full system model, so it is closer to runtime verification than to state-matching trace validation — [P site](https://p-org.github.io/P/)

**Howard, Gruner, Gravell, Ferreira, Augusto — "Model-Based Trace-Checking" (arXiv 1111.2825, 2011)**
- An early statement of the idea: reuse an existing model checker or animator (Spin, ProB) to check traces from a J2EE implementation rather than building a custom checker. Judged feasible, with scaling needing more automation. (Note: the first author is Yvonne Howard, not Heidi Howard of CCF.) — [arXiv](https://arxiv.org/pdf/1111.2825)

**Jepsen / Elle (Kingsbury & Alvaro, PVLDB 14(3))**
- A related but different approach. There is no model of the implementation: Elle infers an Adya-style dependency graph from client-observed transactions and finds cycles. It detects every Adya anomaly except predicate ones, is sound over specific datatypes, and is not complete — [arXiv](https://arxiv.org/pdf/2003.10554); [VLDB](https://www.vldb.org/pvldb/volumes/14/paper/Elle%3A%20Inferring%20Isolation%20Anomalies%20from%20Experimental%20Observations)

### Inferences
- All published successes used deterministic or controlled trace collection: CCF's scenario driver with a global clock, and etcd's unit-test-driven raft core, which is itself a sans-IO-style state machine. piton's design (deterministic core, DST shell) is the favourable case.
- MongoDB's failure modes (multithreaded snapshots, wall-clock ordering, a spec too abstract for the code) are largely designed away by piton's single-threaded fold-of-log core.

### Gaps
- No specific "Microsoft Azure CosmosDB trace validation" source was found in this session. CosmosDB TLA+ work is known for spec writing and model checking. UNVERIFIED whether it did trace validation.
- The TraceLink "sidestep" mechanism was read only from the abstract and intro (pages 1–3).
- Bug descriptions in the etcd PR thread were read from a summary of the PR page.

## Q2. Core techniques: refinement mapping, partial observability, granularity, nondeterminism, performance

### Takeaway
Trace validation is a constrained search: find *some* model behaviour whose projection equals the trace. The knobs are:
- what you log (more logged information means less search);
- the abstraction function from implementation state to model state;
- how to bridge granularity mismatches (stuttering for implementation steps invisible to the model, action composition for one implementation step that equals several model steps);
- how to resolve unlogged nondeterminism (existential quantification over unlogged arguments, fault actions);
- the search strategy (DFS for a single witness).

### Cited Findings
- **Mapping.** Kuppe (etcd issue #111) describes trace validation as mapping recorded implementation states to TLA+ spec states, avoiding symbolic interpretation of code. The approach is not exhaustive and covers only the executions observed — [docs.tlapl.us](https://docs.tlapl.us/using:tlc:trace_validation)
- **Partial observability.** Unlogged variables are generated nondeterministically: TLC checks "that the predicate is satisfied while non-deterministically generating suitable values for any variables left open in the trace." If `event_args` is missing, the action is tried with any instance (`\E r \in RM`) — [Cirstea et al.](https://arxiv.org/html/2404.16075v2)
- **Too little information is unsound.** A trace with too little information "may even lead to erroneous traces being accepted" — [Cirstea et al.](https://arxiv.org/html/2404.16075v2)
- **Stuttering.** Steps invisible at the abstract level (e.g. re-sending Prepared) are logged as no action, or as a stutter step. In CCF, finite stuttering covers a high-level action spanning several events — [Cirstea et al.](https://arxiv.org/html/2404.16075v2); [CCF](https://arxiv.org/html/2406.17455)
- **Action composition.** A∘B was added to TLC for implementation steps combining several spec steps, e.g. Raft term update plus append — [Cirstea et al.](https://arxiv.org/html/2404.16075v2); [CCF](https://arxiv.org/html/2406.17455)
- **Unobserved environment events.** CCF composes an `IsFault` action into the trace next-state relation so that message loss can happen at any step — [CCF](https://arxiv.org/html/2406.17455)
- **Granularity.** Log at linearization points: "when shared state has been updated, such as when sending or receiving messages, performing operations on locks or on stable data storage." The grain of atomicity choice is "important for trace validation to be meaningful" — [Cirstea et al.](https://arxiv.org/html/2404.16075v2)
- **Ordering and clocks:**
  - Cirstea et al. use scalar logical clocks with per-process files merged afterwards [arXiv](https://arxiv.org/html/2404.16075v2).
  - TraceLink uses vector clocks and explores multiple causally consistent linearizations [paper](https://www.cs.ubc.ca/~bestchai/papers/oopsla25-trace-link.pdf).
  - PObserve sorts by timestamp [docs](https://p-org.github.io/P/advanced/pobserve/pobserve/).
  - MongoDB used millisecond wall clocks with sleeps [arXiv](https://arxiv.org/html/2006.00915v1).
  - etcd reviewers flagged that per-node traces lose interleavings [PR #113](https://github.com/etcd-io/raft/pull/113).
- **Search.**
  - BFS gives the shortest counterexamples and is "most informative for debugging." DFS bounded by trace length can stop early [Cirstea et al.](https://arxiv.org/html/2404.16075v2).
  - TLC DFS is enabled via `-Dtlc2.tool.queue.IStateQueue=StateDeque` (added Jan 2024) [docs.tlapl.us](https://docs.tlapl.us/using:tlc:trace_validation).
  - CCF measured DFS under 1 s against BFS at about 1 h [CCF](https://arxiv.org/html/2406.17455).
  - Do not check deadlock, and do not express "reached end" as liveness [Cirstea et al.](https://arxiv.org/html/2404.16075v2).
- **Debugging.** The TLA+ debugger has hit-count and "unsatisfied" breakpoints for finding where the trace diverges — [Cirstea et al.](https://arxiv.org/html/2404.16075v2)
- **Spec adaptations for traces.** CCF changed the network to a multiset in the trace spec to admit duplicates and resends. Its consistency spec assumed global knowledge of all clients, but traces show one client's view, so an action had to reconstruct the others — [CCF](https://arxiv.org/html/2406.17455)

### Inferences
- The search cost grows with unlogged information. If the trace pins down the action, its arguments, and the full projected post-state, the search degenerates to a deterministic check — no search at all.

### Gaps
- No quantitative performance data for very long traces (more than 10^5 events) other than OmniLink's WiredTiger runs of up to 500k operations.

## Q3. The simplest effective scheme for a single-threaded deterministic sans-IO core

### Takeaway
For piton the core is a deterministic function of `(now, Input)` sequences, so the full TLA+ machinery (merging, clocks, existential search over hidden variables) is unnecessary. Log, at every `handle` call boundary, a record `(step_index, now, Input, Vec<Submission emitted>, projection(state_after))`. Then validate each step with "is this observed step an allowed model transition, and does the model's successor match the observed projection?" This is stronger than "replay inputs into the model and compare outputs." Search is needed only where the model is deliberately more nondeterministic than the core.

### Cited Findings
- CCF already relied on a deterministic driver with a single global clock replacing wall clocks to get clean, totally ordered traces — [CCF](https://arxiv.org/html/2406.17455)
- Logging events with their arguments and the resulting state minimizes states explored. With full information, trace validation is cheap — [Cirstea et al.](https://arxiv.org/html/2404.16075v2)
- Log only state the implementation can access at each transition — [Davis et al.](https://arxiv.org/html/2006.00915v1)
- CCF logs constant-size summaries (lengths, not contents) to keep traces small — [CCF](https://arxiv.org/html/2406.17455)
- Stateright's checker can already validate an action sequence against a model:
  - `Path::from_actions(model, init_state, actions) -> Option<Path>` (stateright 0.31.0).
  - For each supplied action it calls `model.next_steps(&prev_state)` and finds the pair whose action equals the supplied action. It returns `None` if no enabled step matches or if the init state is not in `init_states()`.
  - The doc comment says it "Panics for inputs unreachable via the model," but the code returns `None` — [Path docs](https://docs.rs/stateright/latest/stateright/struct.Path.html); [path.rs source](https://docs.rs/stateright/latest/src/stateright/checker/path.rs.html)

### Inferences
**Two checking modes, and why the second is better.**
- **(A) "Replay inputs, compare outputs."** Feed the logged `Input`s into the model (or a reference implementation), then diff the emitted Submissions and state.
  - It is simple, but it assumes the model is deterministic and at the same granularity.
  - It turns the model into a second implementation, which piton's CLAUDE.md ethos warns against as accidental complexity.
- **(B) "Each observed step is an allowed transition."** For each logged step, map it to a model `Action` (e.g. `Action::Handle(input)` plus the observed outputs), check it is in `model.actions(&s)` / `next_steps(&s)`, compute `s' = next_state(s, a)`, and assert `abstract(s') == logged_projection` and `model_outputs(s, a) ⊇/== logged_submissions`.
  - (B) still works when the model is nondeterministic. Example: the model allows either retry or give-up, and the implementation picked one; you search `next_steps` for a successor whose projection matches. This is CCF's "action enabled only when the event matches," done in Rust.
  - It also checks invariants at every intermediate state, which CCF emphasised over hand-placed assertions ([CCF](https://arxiv.org/html/2406.17455)).

**Concrete Rust shape:**
- A `TraceStep { now, input, submissions, projected }` serde record (ndjson), emitted by the DST shell or a feature-gated core hook. The no-op default mirrors etcd's `state_trace_nop.go`.
- A validator loop that keeps a *frontier* `Vec<ModelState>` (normally of size 1). For each step it expands `next_steps`, filters by action match and projection equality, and fails with the last good state plus the candidates on empty.
  - A size-1 frontier is the DFS/greedy case. A small BFS frontier handles model nondeterminism that is not logged.
  - Run each Stateright `properties()` `Always` invariant on every accepted state.
- Allow a bounded number of model-internal "silent" actions between logged steps (stuttering or composition): extend the frontier through any action tagged internal, up to depth k. Prefer instead to make the model's action granularity equal to `handle()`; then no composition is needed. This is the main lesson from EWD998, CCF, and MongoDB.
- Because piton's state is a fold of a durable log, the abstraction function can be computed from the *log* rather than from internal fields. The projection is then implementation-agnostic and cheap.
- Timestamps: since `now` is an explicit input, logical time is in the trace by construction. The model can ignore it or abstract it (e.g. timer-fired vs not).
- Multi-session or multi-node runs in DST: the simulator owns the global order, so emit one merged trace in simulator order. This avoids the per-node interleaving loss raised in etcd PR #113.

### Gaps
- No published example of exactly this pattern (logging and replaying a DST run into a Stateright model) was found. See Q5.

## Q4. Bidirectional sync: model→implementation (MBT) and implementation→model (trace validation), plus coverage

### Takeaway
Use both directions. Model-generated tests catch "the implementation doesn't handle a case the spec allows." Trace validation catches "the implementation did something the spec doesn't allow." Both share the same mapping and driver glue, so build that once. Measure coverage of the model by traces (which model actions and abstract states DST traces hit) to find DST blind spots.

### Cited Findings
- tla-connect (Rust) states this explicitly. Replay and interactive modes catch cases where the implementation "doesn't handle a case the spec allows." NDJSON implementation-trace validation catches behaviour the spec doesn't permit — [tla-connect docs](https://docs.rs/tla-connect)
- MBTCG at MongoDB: 4,913 generated tests reached 100% branch coverage of the merge rules, against 21% for handwritten tests and 92% for AFL — [Davis et al.](https://arxiv.org/html/2006.00915v1)
- quint-connect (Rust) direction is spec→implementation only:
  - Traces come from Quint simulation or tests.
  - Nondeterministic picks (`nondet`/`oneOf`) are passed to the driver via `switch!`.
  - The driver projects implementation state into a spec-shaped type, compared after each transition.
  - Trace validation (implementation→spec) is listed as "future work."
  - It found a known bug in Emerald in under one person-week — [Quint blog, Dec 2025](https://quint.sh/posts/quint_connect); [docs.rs](https://docs.rs/quint-connect)
- Other test-generation tools surveyed by Davis (2025): Mocket (Java), Multi-Grained Specifications (ZooKeeper), SandTable (syscall-level nondeterminism control, samples spec state space to maximise branch coverage) — [MongoDB blog](https://www.mongodb.com/company/blog/engineering/conformance-checking-at-mongodb-testing-our-code-matches-our-tla-specs)
- CCF counterexamples from model checking became implementation tests that "confirmed previously unknown problems" — [Cirstea et al.](https://arxiv.org/html/2404.16075v2)

### Inferences
- **Model→implementation in Stateright.** Run the checker (BFS, or `.spawn_simulation` for random walks), take discovery or example `Path`s, and use `into_actions()` / `into_vec()` to drive the real core `Session` step by step, asserting the projection equals the model state.
  - This is quint-connect's design reimplemented on Stateright `Path`.
  - The core is sans-IO, so it can be driven directly without a DST shell.
- **Implementation→model.** Run DST seeds and validate the traces as in Q3.
- **Coverage.** Over the trace corpus, count distinct `(abstract_state_fingerprint)` and `(action discriminant)` hits against the model's reachable set (Stateright's BFS gives the unique state count). Unhit model actions are DST generator blind spots. Trace steps that are rejected are model or implementation drift.
- **Shared code.** The `abstract(&Session) -> ModelState` function and the `Input → Action` mapping are shared by both directions. Keep them in one place, e.g. a test-support module in the model crate.

### Gaps
- No published coverage metrics of "model states covered by implementation traces" for CCF or etcd were found.

## Q5. Rust / Stateright-specific tooling and examples

### Takeaway
There is no first-party Stateright trace-validation feature, but `Model::next_steps` and `Path::from_actions` provide the primitive. The Rust prior art is quint-connect (spec→implementation replay; Driver/State/`switch!`) and tla-connect (adds NDJSON implementation→spec validation via Apalache). Writing a ~100–200-line frontier validator over a Stateright `Model` is the straightforward route for piton.

### Cited Findings
- stateright 0.31.0 `Path` API: `from_actions`, `last_state`, `into_states`, `into_actions`, `into_vec`, `encode`. `from_actions` validates via `next_steps` (see Q3) — [docs.rs Path](https://docs.rs/stateright/latest/stateright/struct.Path.html); [source](https://docs.rs/stateright/latest/src/stateright/checker/path.rs.html)
- quint-connect 0.1.2 (Apache-2.0):
  - `Driver { type State; fn step(&mut self, step: &Step) -> Result }`
  - `State<D>::from_driver(&D) -> Result<Self>`
  - `switch!` dispatches on action name and extracts nondet picks.
  - `#[quint_test(spec, test)]` and `#[quint_run(spec)]`; depends on the `itf` crate ^0.4.
  - `QUINT_SEED` for reproducibility — [docs.rs](https://docs.rs/quint-connect)
- tla-connect 0.0.4 (MIT/Apache):
  - Apalache-based (TLC is not mentioned).
  - Three modes: ITF trace replay, interactive Apalache JSON-RPC, and implementation trace validation (`validate_trace`, `TraceValidatorConfig`, `StateEmitter`, NDJSON).
  - Driver, `State`/`ExtractState<D>` traits; features `replay`, `trace-gen`, `trace-validation`, `rpc`, `parallel` — [docs.rs](https://docs.rs/tla-connect)
- Informal Systems' modelator (TLA+/Apalache MBT for Rust) — UNVERIFIED in this session; reportedly older and superseded by quint-connect. Do not rely on it.
- "tla-rs": no verified crate by that name was found in this session — UNVERIFIED.
- The TLA+ wiki links an eatonphil/raft-rs discussion on generating specs from code — [docs.tlapl.us](https://docs.tlapl.us/using:tlc:trace_validation)

### Inferences
- Since piton's model will be in Rust (Stateright), there is no cross-language serialization gap. The trace record can hold the core's own `Input` and `Submission` types, or model-level mirrors, and the validator can be a plain `#[test]` over ndjson files written by DST runs.
- quint-connect's ergonomic lessons apply directly:
  - Name every model action.
  - Report divergences as structural diffs of the projection.
  - Make the seed and trace file the reproducer.
  - Start from an empty dispatch and let failures guide coverage — [Quint blog](https://quint.sh/posts/quint_connect)

### Gaps
- No public example was found of anyone replaying implementation traces into a Stateright `Model` (searches did not surface one). This appears to be novel territory; treat as unverified absence.

## Q6. Pitfalls reported in practice

### Takeaway
The recurring costs are spec/implementation granularity mismatch, an abstract spec that diverges within a few steps, consistent state capture in concurrent code, trace ordering across nodes, post-processing bugs, under-logging (which accepts bad traces) or over-logging (which costs time), and maintenance of the mapping as both sides evolve. Most are mitigated by co-designing the spec with trace-checking in mind and keeping it in CI from day one.

### Cited Findings
- **Abstract spec diverges early.** MongoDB's first violation came 4 steps in, leaving 2,683 steps unchecked, and 252 of 345 spec lines had to change — [Davis et al.](https://arxiv.org/html/2006.00915v1)
- **Concurrency snapshots.** About a month to log consistent snapshots for 7 transitions — [Davis et al.](https://arxiv.org/html/2006.00915v1)
- **Post-processing bugs** "could mask implementation bugs" — [Davis et al.](https://arxiv.org/html/2006.00915v1)
- **Per-spec cost.** The marginal cost per additional spec stays high, so infrastructure should be reusable across specs — [Davis et al.](https://arxiv.org/html/2006.00915v1); [MongoDB blog](https://www.mongodb.com/company/blog/engineering/conformance-checking-at-mongodb-testing-our-code-matches-our-tla-specs)
- **Grain of atomicity, unobserved events (message loss), and different terminology between spec and code** — [CCF](https://arxiv.org/html/2406.17455)
- **Spec churn during alignment.** 107 spec changes plus 88 trace-spec commits, about 2 engineer-months — [CCF](https://arxiv.org/html/2406.17455)
- **Under-logging.** It can make validation accept erroneous traces, and logging only variables or only event names explodes the search — [Cirstea et al.](https://arxiv.org/html/2404.16075v2)
- **Per-node traces lose interleavings.** Crash/restart can desync traces from the WAL, and the tooling was hard for non-authors to run — [etcd PR #113](https://github.com/etcd-io/raft/pull/113)
- **Model checking too slow for every CI run.** CCF bounded the state space and used simulation in CI — [CCF](https://arxiv.org/html/2406.17455)
- **Manual instrumentation and mapping burden** is the motivating problem for TraceLink's automation — [TraceLink](https://www.cs.ubc.ca/~bestchai/papers/oopsla25-trace-link.pdf)
- **Production overhead.** CCF and etcd compile tracing out of production builds (compile-time disable or no-op build file) — [CCF](https://arxiv.org/html/2406.17455); [etcd PR #113](https://github.com/etcd-io/raft/pull/113)

### Inferences
Mitigations for piton:
- Make the model's step granularity identical to `Session::handle`, so there are no composition or stutter rules. This is the dominant lesson.
- Derive the projection from the durable log rather than from internals.
- Emit traces from the DST simulator in global order.
- Gate the trace hook behind a cargo feature or the DST shell, so production pays nothing.
- Run trace validation over every DST seed in `just ci`, so drift is caught as it is introduced, which is the CCF "turning point."
- False positives will mostly be spec bugs. Budget for model fixes, as CCF did.

### Gaps
- Logging overhead numbers (CPU or bytes per event) were not reported by the sources read.
