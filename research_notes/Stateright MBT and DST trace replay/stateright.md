# Stateright for model-based testing and DST trace replay in the piton sans-IO core

Method: I cloned github.com/stateright/stateright (master @ `ab8c8be`, 2025-07-27) and read the source directly. Source line references point to that commit, as `src/<file>:<line>`, permalink base https://github.com/stateright/stateright/blob/ab8c8be9341505e0f71edbe5dd88ed275bd976a4/. I also used crates.io, the GitHub API, the Stateright book, HN, Lobsters and web searches. "Verified in source" means I read the code myself.

## 1. Current status: versions, cadence, maintenance, MSRV, license, maintainers, forks

### Takeaway
Stateright is MIT-licensed and works, but development is slow and sporadic. There was one release in 2024 (0.30.2), one in 2025 (0.31.0, 2025-07-27), and no commits to master since then. An open clippy PR from Oct 2025 has not been merged. There is no declared MSRV. Treat it as a stable but thinly maintained dependency that piton might eventually need to vendor or fork.

### Cited Findings
- The latest crates.io version is **0.31.0, published 2025-07-27**. Earlier releases: 0.30.2 (2024-06-03), 0.30.1 (2023-08-27), 0.30.0 (2023-05-28), 0.29.0 (2022-03-19), 0.28.0 (2021-05-22). Release cadence was heavy in 2020–21 (about 40 releases) and has been roughly yearly since 2022. — [crates.io versions API](https://crates.io/api/v1/crates/stateright/versions)
- No crates.io version declares `rust_version` (MSRV is null for all 52 versions). `Cargo.toml` uses `edition = "2021"` and has no `rust-version`. Verified in source. — [crates.io versions API](https://crates.io/api/v1/crates/stateright/versions); [Cargo.toml](https://github.com/stateright/stateright/blob/master/Cargo.toml)
- The license is MIT, and the sole listed author is Jonathan Nadal. — [Cargo.toml](https://github.com/stateright/stateright/blob/master/Cargo.toml)
- The HEAD of master is commit `ab8c8be` "version 0.31.0" (2025-07-27). There have been 39 commits since 2024-01-01, by Jonathan Nadal (15; mostly release, fmt, clippy and dependency upgrades), Peiyang He (7), LiangrunDa (5), Andrew Jeffery (4), cjen1/cjen1-msft (4) and Maurice Lam (3; his email is @google.com). Verified via `git log`. — [commit history](https://github.com/stateright/stateright/commits/master)
- GitHub stats: about 1,902 stars, 63 forks, 11 open issues/PRs, not archived, default branch `master`, last push 2025-07-27. — [GitHub API](https://api.github.com/repos/stateright/stateright)
- Open, unmerged PRs include #86 "actor/network.rs: Fix cargo clippy warnings with Rust 1.90" (opened 2025-10-17) and #75 "Reduce calls to get properties" (opened 2024-02-29). — [GitHub issues API](https://api.github.com/repos/stateright/stateright/issues?state=all&per_page=100)
- The git tags are stale. The newest tag is `v0.30.1`, although Cargo.toml says 0.31.0. Issue #59 ("Tag your releases") was closed, but the tagging habit has not stuck. Verified via `git tag`. — [issue #59](https://github.com/stateright/stateright/issues/59)
- 0.31.0 changes: actor crash **recovery and storage** (`Actor::Storage`, `Out::save`, `ActorModelAction::Recover`); the Explorer and `Path` now reference behaviours by **action index** instead of fingerprints; a **bug fix in the `eventually` checker**; a panic on property-name collision; a Raft example; and `Actor::on_random` / `Out::choose_random` for random choices plus an LWW-register example. — [CHANGES.md](https://github.com/stateright/stateright/blob/master/CHANGES.md)
- 0.30.x added a simulation checker, a checker timeout, `HasDiscoveries` (finish conditions), crash failures (`max_crashes`), the on-demand checker, max-depth tracking and named timers. 0.29 added symmetry reduction (by Chris Jensen) and selectable network semantics. — [CHANGES.md](https://github.com/stateright/stateright/blob/master/CHANGES.md)
- Remote branches besides master include `Cjen1-symmetry`, `ordered-network` and `replicated-processor`. Verified via `git ls-remote`. — [repo](https://github.com/stateright/stateright)
- There is a project Discord and a YouTube video playlist. — [README](https://github.com/stateright/stateright/blob/master/README.md)

### Inferences
- Maintenance is "alive but slow". Jonathan Nadal merges community PRs in batches about once a year. Features in 2024–25 came from outside contributors (academics from NJU/TUM, Microsoft (cjen1-msft) and Google (Maurice Lam)). If piton needs a fix, expect to carry a patch or fork rather than wait for a release.
- Because there is no MSRV and dependencies are on current majors (`rand 0.9`, `dashmap 6.1`), Stateright floats on recent stable Rust. That is fine with devenv-pinned toolchains.
- Stateright pulls in `tiny_http`, `serde_json`, `dashmap`, `parking_lot`, `crossbeam-utils`, `rand`, `ahash` and others **unconditionally**, with no cargo features to slim it down. This conflicts with piton's rule that "every external dependency [is declared] with default features off". It is acceptable only because Stateright would be a dev-dependency of a test crate and never a dependency of the no_std core.

### Gaps
- I did not check whether any fork is more active than upstream. The fork count is 63, but I did not survey them.
- I did not compile it against the newest stable Rust (1.90+). PR #86 suggests clippy warnings but no build breakage.

## 2. Core API: Model, Property, Checker, symmetry, fingerprints, Path, visitors, Explorer

### Takeaway
The core is small. `Model` has associated `State`/`Action` types plus `init_states`, `actions` and `next_state`. Properties are `fn`-pointer predicates with Always/Sometimes/Eventually semantics. `CheckerBuilder` spawns BFS, DFS, on-demand, simulation or Explorer runs in background threads. Discoveries come back as `Path`s, which are now encoded as action indices. The liveness (`eventually`) support is weak: there is no fairness and it is unsound on cycles.

### Cited Findings: `Model` trait (`src/lib.rs`, verified in source)
```rust
pub trait Model: Sized {
    type State;
    type Action;
    fn init_states(&self) -> Vec<Self::State>;
    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>);
    fn next_state(&self, last_state: &Self::State, action: Self::Action) -> Option<Self::State>;
    fn format_action(&self, action: &Self::Action) -> String where Self::Action: Debug { .. }
    fn format_step(&self, last_state: &Self::State, action: Self::Action) -> Option<String> where Self::State: Debug { .. }
    fn as_svg(&self, _path: Path<Self::State, Self::Action>) -> Option<String> { None }
    fn next_steps(&self, last_state: &Self::State) -> Vec<(Self::Action, Self::State)> { .. }
    fn next_states(&self, last_state: &Self::State) -> Vec<Self::State> { .. }
    fn properties(&self) -> Vec<Property<Self>> { Vec::new() }
    fn property(&self, name: &'static str) -> Property<Self> { .. } // panics if unknown
    fn within_boundary(&self, _state: &Self::State) -> bool { true }
    fn checker(self) -> CheckerBuilder<Self> where Self: Send + Sync + 'static, Self::State: Hash + Send + Sync { .. }
}
```
— [src/lib.rs](https://github.com/stateright/stateright/blob/master/src/lib.rs)

- **Determinism requirement:** the docs on `init_states` and `actions` say "The order of actions must be deterministic for the same input state so that the action indices … based `Path` construction can work properly." The panic message in `path.rs` warns specifically that iterating a `HashMap`/`HashableHashMap` can break this. — [src/lib.rs](https://github.com/stateright/stateright/blob/master/src/lib.rs); [src/checker/path.rs](https://github.com/stateright/stateright/blob/master/src/checker/path.rs)
- **`None` from `next_state` behaves as a disabled action, not a self-loop.** The doc comment says "[`None`] indicates that the action does not change the state". But in BFS, `if next_state.is_none() { continue; }` (`src/checker/bfs.rs` ~L304), so the action is skipped and does not count toward non-terminality. `Path::from_actions` uses `next_steps`, which also filters out `None`. Verified in source. — [src/checker/bfs.rs](https://github.com/stateright/stateright/blob/master/src/checker/bfs.rs)
- `next_steps` calls `actions()` **twice** per state because `next_state` consumes the action. `actions()` must therefore be pure and cheap. — [src/lib.rs](https://github.com/stateright/stateright/blob/master/src/lib.rs)
- **`Property`** is `pub struct Property<M> { pub expectation: Expectation, pub name: &'static str, pub condition: fn(&M, &M::State) -> bool }`. The condition is a plain `fn` pointer, not a closure, so any parameters must live in the model struct (`&M`). There are three constructors:
  - `always`: a safety invariant. A counterexample is a discovery.
  - `sometimes`: reachability. An example is a discovery, and its absence is a failure.
  - `eventually`: liveness.
  
  The `eventually` doc states: "`eventually` properties only work correctly on acyclic paths … A path ending in a cycle is not viewed as terminating … an `eventually` property that has not been met by the cycle-closing edge will be ignored -- a false negative." `Expectation::discovery_is_failure()` is true for Always and Eventually. — [src/lib.rs](https://github.com/stateright/stateright/blob/master/src/lib.rs)
- A test named `fixme_can_miss_counterexample_when_revisiting_a_state` in `src/checker.rs` documents that `eventually` misses counterexamples on cycles and on DAG re-joins. Verified in source. — [src/checker.rs](https://github.com/stateright/stateright/blob/master/src/checker.rs)
- **Fingerprints:** `type Fingerprint = NonZeroU64`, computed with `ahash::AHasher` using fixed seeds ("stable hashing, wherein hashes should not vary across builds"). It panics if a hash is 0. States are deduplicated by this 64-bit hash only, so a hash collision silently prunes a state. — [src/lib.rs](https://github.com/stateright/stateright/blob/master/src/lib.rs)

### Cited Findings: `CheckerBuilder` / `Checker` (`src/checker.rs`, verified in source)
- Builder methods:
  - `serve(addr)` starts the Explorer web UI.
  - `spawn_bfs()`
  - `spawn_dfs()`
  - `spawn_on_demand()`
  - `spawn_simulation<C: Chooser<M>>(seed: u64, chooser: C)`
  - `symmetry()` (requires `State: Representative`)
  - `symmetry_fn(fn(&State) -> State)`
  - `finish_when(HasDiscoveries)`
  - `target_state_count(usize)`
  - `target_max_depth(usize)`
  - `threads(usize)`
  - `visitor(impl CheckerVisitor<M> + Send + Sync + 'static)`
  - `timeout(Duration)`
  
  — [src/checker.rs](https://github.com/stateright/stateright/blob/master/src/checker.rs)
- `spawn_bfs`/`spawn_dfs`/`spawn_simulation` bounds: `M::State: Hash + Send + Sync + Clone + PartialEq + 'static, M::Action: Clone + PartialEq`. `serve` additionally needs `Debug` on both State and Action. Checkers run on background threads and are `#[must_use]`. — [src/checker.rs](https://github.com/stateright/stateright/blob/master/src/checker.rs)
- BFS "will find the shortest Path to each discovery if checking is single threaded". DFS "uses dramatically less memory … at the cost of not finding the shortest Path". On-demand "doesn't compute any states until it is asked to" and is used for the Explorer. Simulation "repeatedly traverses the model from initial states to a terminal state … for models that cannot practically be checked exhaustively". It takes a `Chooser` trait with `UniformChooser` built in. — [src/checker.rs](https://github.com/stateright/stateright/blob/master/src/checker.rs); [src/checker/simulation.rs](https://github.com/stateright/stateright/blob/master/src/checker/simulation.rs)
- `Checker` trait methods:
  - `model()`
  - `state_count()`
  - `unique_state_count()`
  - `max_depth()`
  - `discoveries() -> HashMap<&'static str, Path<..>>`
  - `discovery(name)`
  - `join()`
  - `join_and_report(&mut reporter)`
  - `report(&mut reporter)`
  - `is_done()`
  - `discovery_classification(name)`
  - `assert_properties()` ("verifies examples exist for all `sometimes` properties and no counterexamples exist for any `always`/`eventually`")
  - `assert_any_discovery(name) -> Path`
  - `assert_no_discovery(name)`
  - `assert_discovery(name, actions: Vec<M::Action>)`, which replays the actions through `Path::from_actions` from each init state and checks that they really constitute a discovery
  
  — [src/checker.rs](https://github.com/stateright/stateright/blob/master/src/checker.rs)
- `HasDiscoveries` variants: All, Any, AnyFailures, AllFailures, AllOf(set), AnyOf(set), which control early finish. — [src/has_discoveries.rs](https://github.com/stateright/stateright/blob/master/src/has_discoveries.rs)
- **Memory model:** BFS keeps `generated: DashMap<Fingerprint, Option<Fingerprint>>` (fingerprint → parent) plus a job queue of full states with their action-index path (`type Job<State> = (State, Fingerprint, EventuallyBits, NonZeroUsize, Vec<usize>)`). DFS keeps a `DashSet<Fingerprint>`. So visited states are stored as 8-byte fingerprints, but the frontier holds full cloned states. — [src/checker/bfs.rs](https://github.com/stateright/stateright/blob/master/src/checker/bfs.rs); [src/checker/dfs.rs](https://github.com/stateright/stateright/blob/master/src/checker/dfs.rs)
- A comment in `bfs.rs` reads "a refactoring … is being postponed until DPOR is implemented". **There is no partial-order reduction**; DPOR has been on the roadmap since the 2021 talk. — [src/checker/bfs.rs](https://github.com/stateright/stateright/blob/master/src/checker/bfs.rs); [2021 talk slides](https://www.stateright.rs/2021-03-30/)
- **Symmetry:** `pub trait Representative { fn representative(&self) -> Self; }`, supported by `Rewrite<R>`/`RewritePlan` and `util::DenseNatMap` to permute process IDs. The doc cites Bošnački et al., "Symmetric Spin". — [src/checker/representative.rs](https://github.com/stateright/stateright/blob/master/src/checker/representative.rs); [src/checker/rewrite.rs](https://github.com/stateright/stateright/blob/master/src/checker/rewrite.rs)
- **`Path`:** `pub struct Path<State, Action>(Vec<(State, Option<Action>)>)`, which derives `Clone, Debug, Eq, Hash, Ord`. Public API:
  - `Path::from_actions(model, init_state, actions) -> Option<Self>`
  - `last_state()`
  - `into_states()`
  - `into_actions()`
  - `into_vec()`
  - `encode(model) -> String` (slash-delimited action indices, with index 0 being the init state)
  
  `from_action_indices` and `final_state` are `pub(crate)`. — [src/checker/path.rs](https://github.com/stateright/stateright/blob/master/src/checker/path.rs)
- **Visitors:** `pub trait CheckerVisitor<M> { fn visit(&self, model: &M, path: Path<M::State, M::Action>); }`. It is implemented for any `Fn(Path<..>)`, and also by `PathRecorder::new_with_accessor()` and `StateRecorder`. A visitor receives the full path to every evaluated state. — [src/checker/visitor.rs](https://github.com/stateright/stateright/blob/master/src/checker/visitor.rs)
- **Explorer:** `checker().serve("localhost:3000")` serves an HTML UI built with knockout.js and an HTTP API. Endpoints: `GET /.status`, `GET /.states`, and `GET /.states/{init_idx}/{action_idx}/...`, which follows a path of action indices. The `as_svg` hook renders sequence diagrams (ActorModel implements it). — [src/checker.rs](https://github.com/stateright/stateright/blob/master/src/checker.rs); [demo](http://demo.stateright.rs:3000/)

### Inferences
- The `fn`-pointer properties and the single 64-bit fingerprint are the main ergonomic and soundness caveats. Fingerprint collisions are improbable at the state counts piton will reach (≪10^9), but they are not zero.
- Piton's key properties are mostly safety properties (write-ahead ordering, one terminal completion per effect, id = intent offset, `from_log` replay equivalence). They fit `always` and `sometimes` well. Liveness ("every dispatched effect eventually completes") should be encoded as a safety check at terminal/boundary states, e.g. "at a quiescent state no effect is pending", rather than relying on `eventually`.

### Gaps
- I did not benchmark states/sec myself.

## 3. Actor layer: Actor, ActorModel, networks, timers, crashes, spawn, consistency testers

### Takeaway
`Actor` is itself a sans-IO handler: it gets `on_start`/`on_msg`/`on_timeout`/`on_random` callbacks with `&mut Cow<State>` and an `Out` command buffer. `ActorModel` turns a set of actors plus a network into a `Model`. Crashes now support storage-backed recovery, which makes crash/recover look like piton's `from_log` (the durable log). It does not, however, map piton's `Submission` / append-ack protocol directly. The linearizability and sequential-consistency testers are reusable stand-alone.

### Cited Findings (verified in source)
- `pub trait Actor: Sized` has these associated types:
  - `Msg: Clone + Debug + Eq + Hash`
  - `Timer: Clone + Debug + Eq + Hash`
  - `State: Clone + Debug + PartialEq + Hash`
  - `Storage: Clone + Debug + PartialEq + Hash` ("non-volatile state … should be a subset of the `State`")
  - `Random: Clone + Debug + Eq + Hash + Ord`
  
  Methods:
  - `fn on_start(&self, id: Id, storage: &Option<Self::Storage>, o: &mut Out<Self>) -> Self::State`
  - `fn on_msg(&self, id, state: &mut Cow<Self::State>, src: Id, msg, o: &mut Out<Self>)`
  - `fn on_timeout(&self, id, state, timer: &Self::Timer, o)`
  - `fn on_random(&self, id, state, random: &Self::Random, o)`
  - `fn name(&self) -> String`
  
  — [src/actor.rs](https://github.com/stateright/stateright/blob/master/src/actor.rs)
- `enum Command<Msg, Timer, Random, Storage> { CancelTimer(Timer), SetTimer(Timer, Range<Duration>), Send(Id, Msg), ChooseRandom(String, Vec<Random>), Save(Storage) }`. `Out<A>` has `send`, `broadcast`, `set_timer`, `cancel_timer`, `choose_random`, `remove_random` and `save`. Using `Cow` lets the checker detect no-ops cheaply (`is_no_op`). — [src/actor.rs](https://github.com/stateright/stateright/blob/master/src/actor.rs)
- `ActorModel<A, C = (), H = ()>` has these fields: `actors`, `cfg`, `init_history`, `init_network: Network<A::Msg>`, `lossy_network: LossyNetwork`, `max_crashes: usize`, `properties`, `record_msg_in`/`record_msg_out: fn(&C, &H, Envelope<&Msg>) -> Option<H>` (an auxiliary "history" variable for properties such as consistency testers) and `within_boundary`. It has builder methods of the same names. — [src/actor/model.rs](https://github.com/stateright/stateright/blob/master/src/actor/model.rs)
- `enum ActorModelAction<Msg, Timer, Random> { Deliver{src,dst,msg}, Drop(Envelope<Msg>), Timeout(Id, Timer), Crash(Id), Recover(Id), SelectRandom{actor, key, random} }`. `Crash` is offered only while the number of crashed actors is below `max_crashes`. — [src/actor/model.rs](https://github.com/stateright/stateright/blob/master/src/actor/model.rs)
- `LossyNetwork::{Yes, No}`. The docs say "as long as invariants do not check the network state, losing a message is indistinguishable from an unlimited delay, so in many cases you can improve model checking performance by not modeling message loss." — [src/actor/model.rs](https://github.com/stateright/stateright/blob/master/src/actor/model.rs)
- `Network<Msg>` has three variants:
  - `UnorderedDuplicating(HashableHashSet<Envelope>, Option<Envelope>)`
  - `UnorderedNonDuplicating(HashableHashMap<Envelope, usize>)`
  - `Ordered(BTreeMap<(Id, Id), VecDeque<Msg>>)` (FIFO per directed pair)
  
  It is constructed with `new_ordered`, `new_unordered_duplicating` and `new_unordered_nonduplicating`. — [src/actor/network.rs](https://github.com/stateright/stateright/blob/master/src/actor/network.rs)
- Timers carry no time in the model. `model_timeout()` returns `0..0`, and a set timer simply enables a nondeterministic `Timeout` action: "The specific timeout value is not relevant for model checking". — [src/actor/model.rs](https://github.com/stateright/stateright/blob/master/src/actor/model.rs)
- `actor::spawn(msg_serialize, msg_deserialize, storage_serialize, storage_deserialize, actors: Vec<(impl Into<Id>, A)>)` runs the same actors over **UDP**, blocking the current thread. `Id` encodes an IPv4 address and port. — [src/actor/spawn.rs](https://github.com/stateright/stateright/blob/master/src/actor/spawn.rs)
- Semantics module:
  - `SequentialSpec` defines a reference object (`Op`, `Ret`, `invoke`), with built-in `register`, `write_once_register` and `vec` specs.
  - `ConsistencyTester<T, RefObj>` has `on_invoke(thread_id, op)`, `on_return(thread_id, ret)`, `is_consistent()` and `on_invret`.
  - Implementations are `LinearizabilityTester::new(init_ref_obj)` (also has `serialized_history()`) and `SequentialConsistencyTester`.
  
  The testers are ordinary values that can be used outside the checker on any recorded history. — [src/semantics.rs](https://github.com/stateright/stateright/blob/master/src/semantics.rs); [src/semantics/consistency_tester.rs](https://github.com/stateright/stateright/blob/master/src/semantics/consistency_tester.rs); [src/semantics/linearizability.rs](https://github.com/stateright/stateright/blob/master/src/semantics/linearizability.rs)
- Bundled examples: 2pc, increment, increment_lock, interaction ("how to use an Actor to model user interaction or other external inputs"), linearizable-register (ABD), lww-register, paxos (single-decree), raft, single-copy-register and timers. Other components include `ordered_reliable_link` and the `register`/`write_once_register` actor wrappers. — [examples/](https://github.com/stateright/stateright/tree/master/examples)

### Inferences
- Piton is a single-node runtime whose nondeterminism is IO completion ordering, crashes, and append acks versus effect completions. It is not a network of peers, so `ActorModel` is a poor fit. The `Session`/shell boundary would have to be shoehorned into actors (for example a "log" actor, a "session" actor and an "effect executor" actor), and Stateright's UDP `spawn` runtime is irrelevant because piton has its own shell.
- A hand-written `Model` with an environment state (pending appends, in-flight effects, durable log prefix, crash count) is the natural design. `ActorModel` remains useful as reference code for how to model crash/recover with storage (compare `Crash`/`Recover` with `on_start(storage)`, which is analogous to `from_log`).
- `LinearizabilityTester` probably does not apply to piton, except perhaps for any concurrent client-facing API. For "effect results are applied exactly once", a custom history plus an `always` property is simpler.

### Gaps
- I did not read `model.rs` fully to check how `Recover` interacts with timers and network state.

## 4. Wrapping an existing Rust state machine inside `Model::next_state`

### Takeaway
Yes, this works and is the intended usage. Stateright's selling point is "the code you check is the code you run". `next_state` can clone the state, call `Session::handle`/`poll_submission` and return the result. The costs are: the state must be `Clone + Hash + Eq/PartialEq` (`+ Debug` for Explorer), the clone/hash happen on every transition, and the state space includes every irrelevant concrete detail unless it is abstracted away.

### Cited Findings
- The README says Stateright is "a Rust actor library … providing an embedded model checker … and a lightweight actor runtime", in contrast with TLA+, where systems must be reimplemented. — [README](https://github.com/stateright/stateright/blob/master/README.md)
- Book comparison with TLA+: "With Stateright your model and final implementation are encouraged to share code", which removes the risk that model and implementation deviate. It concedes "TLA+ is more concise than Rust". — [Comparison with TLA+](https://www.stateright.rs/comparison-with-tlaplus.html)
- The author on HN: Stateright verifies both design and implementation, but does not prove correctness. It explores nondeterminism exhaustively within bounds (e.g. a fixed number of servers and clients). — [HN thread](https://news.ycombinator.com/item?id=27413084)
- The state type requirements come from the checker bounds (`Hash + Send + Sync + Clone + PartialEq + 'static`; `Debug` for `serve`) and from the `PathRecorder` bound (`Eq + Hash`). — [src/checker.rs](https://github.com/stateright/stateright/blob/master/src/checker.rs); [src/checker/visitor.rs](https://github.com/stateright/stateright/blob/master/src/checker/visitor.rs)
- `HashMap`/`HashSet` are not `Hash`. Stateright provides `util::HashableHashSet` / `HashableHashMap`, which hash order-independently, but warns that iterating them is nondeterministic and breaks action-index path reconstruction. `BTreeMap`/`BTreeSet`/`Vec`/`VecDeque` already implement `Hash` deterministically. — [src/util.rs](https://github.com/stateright/stateright/blob/master/src/util.rs); [src/checker/path.rs](https://github.com/stateright/stateright/blob/master/src/checker/path.rs)
- Precedent: Liangrun Da used Stateright to check a real Rust Raft implementation ("Raft Lite"). His blog post (Feb 2024) was shared by Martin Kleppmann, and the work became the Raft example in 0.31.0. The crate `raft-lite` is a reverse dependency on crates.io. — [Kleppmann Mastodon post](https://nondeterministic.computer/@martin/112005567974060209); [CHANGES.md](https://github.com/stateright/stateright/blob/master/CHANGES.md); [crates.io reverse deps](https://crates.io/api/v1/crates/stateright/reverse_dependencies?per_page=100)

### Inferences (specific to piton)
- **Model state** = `(Session, Env)`. `Session` is the real no_std core state. `Env` is a small abstract environment: the durable log (`Vec<Record>`), unacked appends, dispatched-but-uncompleted effects, crash count and logical clock. Actions are environment events (`AckAppend(i)`, `Complete(effect_id, outcome)`, `Crash`, `Restart`, `Tick`, `UserInput(k)`). `next_state` clones, feeds the event into `Session::handle(now, input)`, drains `poll_submission()` into `Env`, and returns. `Crash` keeps only the durable log, and `Restart` calls `Session::from_log(log)`. This directly checks write-ahead ordering, exactly-once terminal completion, effect id = intent offset, and that `from_log(log)` equals the live state.
- **Requirements on the core:** `Session` and everything in it must derive `Clone + Hash + PartialEq/Eq + Debug`. `Vec` and `BTreeMap` are fine. Avoid `HashMap`, which piton's lints already ban (good alignment). Avoid `Instant`/opaque handles, and make `now` a logical value. These derives are no_std-compatible (`core::hash::Hash`), so the cost is mainly derive noise plus any non-hashable fields such as closures or `Box<dyn ...>`.
- **Pros:** the checked code is the real code, with no model/impl drift; one language; properties can call real helper functions; a counterexample is a real Rust test.
- **Cons:**
  - The state space blows up with irrelevant details such as payload bytes, counters and timestamps. Mitigate by bounding inputs to a tiny alphabet, using `within_boundary` (log length ≤ N, crashes ≤ K) and abstracting `now`.
  - Hashing large `Vec<Record>` logs on each step is O(log size).
  - There is no refinement mapping or abstraction layer, so bounded exhaustive coverage of the real core is the only guarantee.
- **A separate abstract model** would be more compact and could check design-level properties with larger bounds, but then needs a conformance bridge to the real core (see §5). A pragmatic hybrid: wrap the real core in the `Model` (implementation-level model checking), and treat the `Env` part as the "abstract model" of the IO shell. That `Env` model is what DST traces get validated against.

### Gaps
- I found no published example of Stateright wrapping a no_std sans-IO core specifically, and no measurements of clone/hash overhead for large states.

## 5. Replaying an externally recorded trace (DST) through a Stateright Model

### Takeaway
There is no built-in trace-validation or conformance feature. The building blocks are public:
- `Path::from_actions(&model, init_state, &actions) -> Option<Path>` replays a concrete action sequence and returns `None` at the first step that is not enabled.
- `Checker::assert_discovery` replays actions and checks that they witness a property.
- Calling `init_states`/`actions`/`next_state`/`properties` by hand gives full control, including state-observation matching and nondeterministic search.

Piton would need about 50–150 lines of its own replay harness. I found no issue or discussion in the repo about trace replay.

### Cited Findings (verified in source)
- `pub fn from_actions<'a, M>(model: &M, init_state: State, actions: impl IntoIterator<Item = &'a Action>) -> Option<Self> where State: PartialEq, Action: PartialEq + 'a`. It checks `model.init_states().contains(&init_state)`. Then, for each action, it finds the step in `model.next_steps(&prev_state)` whose action `==` the recorded action, or returns `None`. It records `(state, Some(action))` pairs and ends with `(last_state, None)`. — [src/checker/path.rs](https://github.com/stateright/stateright/blob/master/src/checker/path.rs)
- Consequences of the code:
  - (a) a replayed action must be **enabled**, i.e. produced by `actions()` **and** yield `Some` from `next_state`;
  - (b) the initial state must equal one of `init_states()`;
  - (c) `None` reveals neither which step failed nor why;
  - (d) the doc comment says "Panics for inputs unreachable via the model", but the code returns `None`;
  - (e) cost is O(branching) `next_state` calls per step, because it computes all successors.
  
  — [src/checker/path.rs](https://github.com/stateright/stateright/blob/master/src/checker/path.rs)
- `Path::encode(&model)` and the internal `from_action_indices` represent traces as `init_idx/action_idx/...`. This is the same format the Explorer URLs use since 0.31.0 (`GET /.states/{init}/{a1}/{a2}/...`), so a replayed trace can be inspected in the Explorer by visiting that URL. — [src/checker/path.rs](https://github.com/stateright/stateright/blob/master/src/checker/path.rs); [src/checker.rs](https://github.com/stateright/stateright/blob/master/src/checker.rs); [CHANGES.md](https://github.com/stateright/stateright/blob/master/CHANGES.md)
- `Checker::assert_discovery(name, actions)` replays `actions` from every init state and checks that the property condition holds or fails as expected. It is a ready-made way to turn a DST failure into a regression test, once the failing trace has been mapped to model actions. — [src/checker.rs](https://github.com/stateright/stateright/blob/master/src/checker.rs)
- The GitHub issue/PR list (#59–#86) has no item on trace replay, conformance, refinement or MBT. — [GitHub issues API](https://api.github.com/repos/stateright/stateright/issues?state=all&per_page=100)
- For contrast, the ecosystem precedent for trace validation is TLA+/Apalache, as in DFINITY. Their Rust code logs TLA traces during tests, and the logs are checked against the model, using Apalache rather than TLC because of large constant domains. HN commenter hwayne: TLA+ users "need to generate behavioral traces and refine them into code tests", which he called a bespoke process with no battle-tested tooling. — [tlaplus discuss](https://discuss.tlapl.us/msg06428.html); [HN thread](https://news.ycombinator.com/item?id=27413084)
- Quint has dedicated MBT tooling: `quint-connect` (Rust) generates traces from Quint specs (`quint run --mbt`, ITF format), replays them through a user driver, and compares spec state with implementation state at each step. That is the opposite direction (spec → impl), while piton needs impl → model. — [docs.rs quint-connect](https://docs.rs/quint-connect); [@firfi/quint-connect README](https://cdn.jsdelivr.net/npm/@firfi/quint-connect@2.0.1/README.md)

### Inferences: a recommended harness for piton
- **Trace format from the shell's DST:** record the sequence of *environment events*, i.e. exactly the model's `Action` enum (`AckAppend`, `Complete{id, outcome}`, `Crash`, `Restart`, `Input{..}`, `Tick`). Optionally record *observations* such as each `Submission` emitted and a digest of the durable log after each step.
- **Replay loop** (do not rely solely on `Path::from_actions`, because it gives no diagnostics):
  ```text
  let mut s = model.init_states()[0].clone();
  for (i, (a, obs)) in trace.enumerate() {
      let mut enabled = vec![]; model.actions(&s, &mut enabled);
      ensure!(enabled.contains(&a), "step {i}: action not enabled");
      s = model.next_state(&s, a).ok_or("step {i}: disabled")?;
      ensure!(project(&s) == obs, "step {i}: observation mismatch");
      for p in model.properties() { if p.expectation == Always { ensure!((p.condition)(&model, &s)) } }
  }
  ```
  On success, `Path::from_actions` can still produce a `Path` for `as_svg` or Explorer viewing.
- **If the DST trace is under-specified** (for example it records only externally visible submissions, not internal choices), replay becomes a search. Run BFS/DFS over a constrained model where `actions()` yields only actions consistent with trace step `i`, so that the state includes the trace cursor. This is the TLA+ "trace validation" pattern; Stateright can express it but has no helper.
- **Since piton's core is deterministic,** with only the environment nondeterministic, recorded shell events map 1:1 onto model actions. Replay is then a deterministic fold with no search needed. This is the easy case.
- **Two directions are possible:** (1) DST trace → Stateright model (validate the shell's behaviour against the model/core); (2) Stateright `Path` → shell replay (run a model-found counterexample against the real shell under the DST harness). `Path::into_actions()` gives (2) for free.

### Gaps
- I found no public example, blog post or issue where anyone replays production or DST traces through Stateright. I could not read Liangrun Da's Raft Lite blog post in full to check whether it involves trace replay.

## 6. Performance and state-explosion characteristics

### Takeaway
Stateright is an explicit-state, multi-threaded checker with 64-bit fingerprint dedup and optional symmetry reduction. It has **no partial-order reduction**. The bundled examples check thousands to tens of thousands of unique states in tests, and the author's 2PC benchmark ran 12.3× faster than TLC at 10 resource managers. For larger spaces you fall back to DFS (less memory), `target_max_depth`/`target_state_count`/`timeout`, or the simulation checker.

### Cited Findings
- Example unique-state counts asserted in tests:
  - 2pc: 288 / 665 / 8,832, depending on configuration
  - paxos: 16,668
  - linearizable-register (ABD): 544
  - single-copy-register: 54 / 93
  
  Verified in source. — [examples/2pc.rs](https://github.com/stateright/stateright/blob/master/examples/2pc.rs); [examples/paxos.rs](https://github.com/stateright/stateright/blob/master/examples/paxos.rs)
- Benchmark configurations in `bench.sh`: `2pc check 10`, `paxos check 6`, `single-copy-register check 4` and `linearizable-register check 2`. — [bench.sh](https://github.com/stateright/stateright/blob/master/bench.sh)
- The book's TLA+ comparison: "Stateright tends to be faster". The speedup over TLC grew from 1.8× at 7 resource managers to 12.3× at 10 (TLC 674 s vs Stateright 54.709 s at 10 RMs). These are the author's own numbers, not independently verified. — [Comparison with TLA+](https://www.stateright.rs/comparison-with-tlaplus.html)
- `CheckerBuilder::threads(n)`: "For maximum performance this should match the number of cores." BFS returns shortest paths only when single-threaded. — [src/checker.rs](https://github.com/stateright/stateright/blob/master/src/checker.rs)
- DPOR is unimplemented (code comment and 2021 roadmap). — [src/checker/bfs.rs](https://github.com/stateright/stateright/blob/master/src/checker/bfs.rs); [2021 talk](https://www.stateright.rs/2021-03-30/)
- The simulation checker was added to reach deep states in models that "cannot practically be checked exhaustively". A timeout and fail-fast option (`finish_when`) were added for long runs (issues #68/#69). — [CHANGES.md](https://github.com/stateright/stateright/blob/master/CHANGES.md); [GitHub issues](https://api.github.com/repos/stateright/stateright/issues?state=all&per_page=100)

### Inferences
- For piton, the explosion drivers are interleavings of independent completions (N effects in flight → N! orderings) times crash points times input sequences. Without POR, keep N small (2–3 concurrent effects, 1–2 crashes, short input alphabets). Use `symmetry_fn` where effects are interchangeable, and use simulation mode for deep runs. Expect 10^4–10^6 unique states to be comfortable on a laptop. This is an extrapolation from example sizes plus the fingerprint-only visited set, not measured.

### Gaps
- I found no published states/sec figures or memory-per-state measurements beyond the 2PC comparison, and no reports of very large (10^8+) Stateright runs.

## 7. Real-world users, case studies and materials

### Takeaway
Stateright is used mostly in education and research, plus a handful of OSS projects (18 crates.io reverse dependencies). I found no confirmed production deployment at a company. Contributors from Google and Microsoft (cjen1-msft) suggest some internal experimentation, but that is unconfirmed.

### Cited Findings
- The book "Building Distributed Systems With Stateright" (www.stateright.rs) covers ABD and other topics. Murat Demirbas (Apr 2021) reviewed it positively for its visualization and "time-travel debugging", noting the Paxos chapter was "coming soon". — [Murat's blog](http://muratbuffalo.blogspot.com/2021/04/building-distributed-systems-with.html); [README](https://github.com/stateright/stateright/blob/master/README.md)
- The 2021 talk by Jonathan Nadal lists ibc-rs (Informal Systems' IBC) as an industry example: a "partial TLA+-to-Stateright port by Vitor Enes Duarte … Incomplete experiment". The roadmap included DPOR, liveness improvements, Tokio/async runtimes and other-language adapters. — [2021 talk slides](https://www.stateright.rs/2021-03-30/)
- There are 18 reverse dependencies on crates.io, including raft-lite, rings-core, hydracache-cluster-raft, iroh-gossip-rendezvous, abyo-crdt, weft-raft/weft-chord and the krabka-* crates. — [crates.io reverse deps](https://crates.io/api/v1/crates/stateright/reverse_dependencies?per_page=100)
- Liangrun Da's Feb 2024 post on verifying the Raft Lite implementation with Stateright was boosted by Martin Kleppmann. — [Mastodon](https://nondeterministic.computer/@martin/112005567974060209)
- Community reception: HN thread (2021) and a Lobsters thread. In the Lobsters thread hwayne relays the author's explanation that actors are connected to a test orchestrator; they run normally in production or in a test mode that exposes possible actions to the checker. — [HN](https://news.ycombinator.com/item?id=27413084); [Lobsters](https://lobste.rs/s/ugylam/stateright_model_checker_for)

### Gaps
- I found no company publicly stating production use, and no conference talk after 2021.

## 8. Comparisons: TLA+/TLC, Quint, P, FizzBee, proptest-state-machine, shuttle/loom

### Takeaway
Stateright's niche is exhaustive, explicit-state model checking **of Rust code itself**, which suits piton's deterministic core. It is weaker on spec concision, temporal logic and fairness, refinement, POR, tooling maturity and community size, and it has no built-in MBT/trace-validation workflow (Quint, FizzBee and P have some).

### Cited Findings
- **TLA+/TLC:**
  - The book credits TLA+ with brevity, "arbitrarily complex temporal properties including fairness" and refinement mapping.
  - It credits Stateright with code sharing, ecosystem reuse, speed, `sometimes` properties and the Explorer.
  - The page lists symmetry reduction as a TLC advantage, but Stateright gained symmetry reduction in 0.29 (2022), so that page is outdated on this point.
  
  — [Comparison with TLA+](https://www.stateright.rs/comparison-with-tlaplus.html); [CHANGES.md](https://github.com/stateright/stateright/blob/master/CHANGES.md)
- **TLA+ trace validation** (impl → spec) has industrial precedent (DFINITY with Apalache), but requires a separate spec and log instrumentation. — [tlaplus discuss](https://discuss.tlapl.us/msg06428.html)
- **Quint:** a separate spec language with a `quint-connect` Rust MBT crate that replays spec-generated ITF traces against the implementation, comparing states step by step. — [docs.rs quint-connect](https://docs.rs/quint-connect)
- **proptest-state-machine:** randomized (not exhaustive) state-machine testing with a reference model, run via the `prop_state_machine!` macro. Concurrent execution is not yet supported. Its strength is shrinking and generator integration. — [docs.rs proptest-state-machine](https://docs.rs/proptest-state-machine/latest/proptest_state_machine/)

### Inferences
- **vs proptest-state-machine:** proptest gives random exploration with shrinking and no Hash/Clone requirement. Stateright gives exhaustive bounded exploration with shortest counterexamples (single-threaded BFS acts as natural "shrinking"). They are complementary. Proptest is better for big input domains, Stateright for interleavings, crash points and ack/complete orderings.
- **vs shuttle/loom:** these explore thread interleavings of real concurrent code (atomics, mutexes). Piton's core is single-threaded and sans-IO, so they are irrelevant to the core. They might matter only for the shell's internal concurrency. Stateright does not model the memory model or threads.
- **vs P and FizzBee:** both are separate modelling languages with their own checkers and MBT stories. FizzBee has a `fizz-mbt` adapter workflow for Rust (a skill exists in this environment). They are better at concise design-level specs. Stateright avoids a second language and spec drift. I did not research P or FizzBee deeply in this note; that is covered by other researchers.
- **Net fit for piton:**
  - Stateright is a good fit for exhaustive bounded checking of the real `Session` under an environment model, given piton's determinism, Hash-friendly BTreeMap/Vec state and safety-heavy properties.
  - It is a modest fit for DST trace replay: the replay must be hand-rolled, which is easy, using `next_state`/`actions`/`Path::from_actions`.
  - It is a poor fit for liveness checking (use terminal-state safety properties instead) and for its actor/UDP runtime (unneeded).
  - Maintenance risk is moderate. The surface piton needs (`Model`, `CheckerBuilder`, `Path`) is small and stable, so forking or vendoring is feasible, and the alternative of a ~300-line custom BFS over the same `Model`-shaped trait is also realistic.

### Gaps
- I found no head-to-head published comparison of Stateright with Quint, P or FizzBee. P/FizzBee/shuttle/loom points are from general knowledge and are not source-cited here (marked as inference).
