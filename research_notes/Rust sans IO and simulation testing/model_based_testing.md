# Model-based testing, stateful property-based testing, and lightweight formal methods in Rust

Research date: 2026-10-18. Links marked (verified) were fetched or returned by search this session. Links marked (unverified) are from prior knowledge and were not re-checked; treat them as leads.

## Rust tools: what each does and when to use it

### Takeaway
Rust has a layered toolbox. Use proptest plus proptest-state-machine (or arbtest/arbitrary or the newer Hypothesis-backed `hegeltest`) to check an implementation against an executable reference model over random operation sequences. Use loom (exhaustive) or shuttle (randomized, scales further) to explore thread interleavings, Kani for bounded proofs of small functions, stateright for model checking actor protocols written in Rust, and Quint plus `quint-connect` when the spec lives outside Rust and you want traces replayed against the implementation. The canonical industrial combination, from ShardStore, is proptest conformance against a Rust reference model, plus Loom for small primitives and Shuttle for large harnesses.

### Cited Findings
**proptest + proptest-state-machine**
- proptest-state-machine (0.9.0 on docs.rs, depends on proptest ^1.12) gives strategies and a runner for abstract state machine tests. Its traits are `ReferenceStateMachine` (the model, which generates transitions) and `StateMachineTest` (applies them to the SUT). The `prop_state_machine!` macro takes a transition-count range; only `sequential` mode is supported, not concurrent. — [docs.rs proptest-state-machine](https://docs.rs/proptest-state-machine/latest/proptest_state_machine/) (verified)
- Tomáš Zemanovič wrote it, first as a blog post (used at Anoma/Namada), and it shipped as a crate in June 2023. — [State machine testing with Proptest](https://tzemanovic.codeberg.page/posts/state-machine-testing-with-proptest/) (verified)
- The usage docs are the Proptest Book's "State Machine testing" chapter. — [docs.rs](https://docs.rs/proptest-state-machine/latest/proptest_state_machine/) (verified); book at https://proptest-rs.github.io/proptest/proptest/state-machine.html (unverified)
- Jujutsu (jj) uses proptest-state-machine in `lib/testutils` and jj-cli, modelling the working copy as a reference state machine. — [jj mirror, lib/testutils](https://git.joshthomas.dev/mirrors/jj/src/commit/ade2aea0159eb5488813588ddac26840e400cdee/lib/testutils) (verified via search snippet); [lib.rs](https://www.lib.rs/crates/proptest-state-machine) (verified)

**arbitrary / arbtest / cargo-fuzz**
- matklad's `arbtest` is "a powerful property-based testing library with a tiny API and a small implementation", built on the `arbitrary` crate. Tests pull values with `u.arbitrary()` from a finite byte buffer. — [github.com/matklad/arbtest](https://github.com/matklad/arbtest) (verified)
- In matklad's "Properly Testing Concurrent Data Structures" (2024-07-05), finite-entropy PBT (arbtest) gives shrinking almost for free, because a shorter byte input means a shorter test. `exhaustigen` enumerates every interleaving for small inputs. A hand-rolled "managed threads" harness with a `pause()` hook in each atomic operation acts as a toy loom, and the result is compared against a sequential model. — [matklad 2024](https://matklad.github.io/2024/07/05/properly-testing-concurrent-data-structures.html) (verified)
- Because arbtest and cargo-fuzz both take an `arbitrary::Unstructured` input, one harness can serve as a fast PBT test and as a coverage-guided libFuzzer target. — Inference from [arbtest](https://github.com/matklad/arbtest) and [libFuzzer](https://llvm.org/docs/LibFuzzer.html) (verified)

**hegeltest (Hegel), Hypothesis-style PBT for Rust (new in 2026)**
- Antithesis announced Hegel on 2026-03-24 (author David MacIver, creator of Hypothesis). The Hypothesis engine generates all data and a thin Rust client turns it into values over the "Hegel protocol". This gives Hypothesis-quality shrinking, with no invalid test cases and no hand-written shrinkers. Rust shipped first; Go is planned and C++, OCaml and TypeScript are in progress. — [Antithesis blog](https://antithesis.com/blog/2026/hegel/) (verified)
- The crate is `hegeltest`, imported as `hegel`. It is pre-1.0 beta, needs `uv` on PATH, and has an experimental pure-Rust `native` engine that is weaker than the Python one. — [docs.rs hegeltest](https://docs.rs/hegeltest) (verified via search)
- Bugs found: `fraction` `from_str("0/0")` panics; a zero-handling bug in `rust_decimal`; a title-case idempotence failure on "ß" in `heck`; a `get_prev` divergence from `BTreeMap` in `im` (a model-based check). — [Antithesis blog](https://antithesis.com/blog/2026/hegel/) (verified)
- The announcement does not describe Hegel's stateful testing API. — [Antithesis blog](https://antithesis.com/blog/2026/hegel/) (verified)

**loom and shuttle (concurrency)**
- Shuttle (awslabs) runs randomized concurrency tests, including a PCT-style probabilistic scheduler. It is unsound, but it "scales to much larger test cases than Loom" and replays failures deterministically. You swap `std` sync imports for Shuttle's and wrap tests in `check_random`. Releases: 0.7.0 (Mar 2024), 0.8.0 (Oct 2024). — [github.com/awslabs/shuttle](https://github.com/awslabs/shuttle); [lib.rs/crates/shuttle](https://lib.rs/crates/shuttle) (verified)
- ShardStore uses Loom "to soundly check all interleavings of small, correctness-critical code such as custom concurrency primitives, and Shuttle to randomly check interleavings of larger test harnesses to which Loom does not scale such as end-to-end" tests. — [ShardStore paper §6](https://jamesbornholt.com/papers/shardstore-sosp21.pdf) (verified, text extracted)
- A practitioner write-up on Shuttle: [grantslatton.com/shuttle](https://grantslatton.com/shuttle) (verified exists; not read)

**Kani (bounded model checking)**
- Kani is a CBMC-based bounded model checker for Rust that supports safe and unsafe code. It "cannot guarantee unbounded verification in all cases". — [AWS Open Source blog](https://aws.amazon.com/blogs/opensource/verify-the-safety-of-the-rust-standard-library/) (verified via search)
- A 2026 Kani paper (ASE 2026 industry showcase, arXiv 2607.01504) reports over 16,000 harnesses verified per change in the Rust std verification campaign, plus function and loop contracts that go beyond bounded checks. — [arXiv 2607.01504](https://arxiv.org/abs/2607.01504); [ASE 2026](https://conf.researchr.org/details/ase-2026/ase-2026-industry-showcase/37/Kani-A-Model-Checker-for-Rust) (verified via search)
- As of Oct 2025, 6 of the 9 completed std-verification challenges used Kani. — [arXiv 2510.01072](https://arxiv.org/pdf/2510.01072) (verified via search)

**stateright (actor model checker)**
- Stateright is a Rust actor library with an embedded model checker, a web UI for exploring behaviour, and a lightweight actor runtime. It checks "always", "sometimes" and experimental "eventually" properties, has a linearizability tester and a sequential-consistency tester, and supports symmetry reduction. Example: a linearizable ABD register (`cargo run --release --example linearizable-register check 2 ordered`). Latest is 0.31.0. — [docs.rs stateright](https://docs.rs/crate/stateright/latest) (verified via search)
- Its distinctive feature: the checked model is the same Rust actor code you run in production, so there is no separate spec language. — [docs.rs stateright](https://docs.rs/crate/stateright/latest) (verified via search)

**Quint + quint-connect (Informal Systems)**
- `quint-connect` (announced 2025-12-23 by Gabriela Moreira and Erick Pintor) is a Rust MBT library. It takes traces from Quint (random simulation, witnesses, or named runs) and replays them against a Rust implementation. You implement a `Driver` trait with `step`. The `switch!` macro maps each spec action and its nondeterministic picks to Rust methods, and reports missing transitions or parameters. A `State` trait's `from_driver` projects implementation state into spec-shaped state, which is compared after every step. `#[quint_run]` (for example `max_samples = 1000`) runs random simulations, `#[quint_test]` replays a named spec test, and everything runs under plain `cargo test`. The glue was designed so LLMs can generate and maintain it. — [quint.sh/posts/quint_connect](https://quint.sh/posts/quint_connect) (verified); [docs.rs quint-connect](https://docs.rs/quint-connect) (verified via search)
- Example projects: a two-phase commit example in the quint-connect repo; Emerald (Malachite consensus plus Reth) under `tests/mbt`, built in under a person-week, which found a bug the team already knew about; and an earlier Malachite experiment testing a Fast Tendermint change, which drove the API design. — [quint.sh](https://quint.sh/posts/quint_connect) (verified); [Emerald case study](https://quint-lang.org/posts/quint_connect_emerald) (verified via search)
- Malachite has a `cargo mbt` model-based test target that requires Quint. — search summary of [circlefin/malachite](https://tomevault.io/tome/circlefin/malachite) (aggregator, weak source). Primary repo: https://github.com/circlefin/malachite (unverified)
- quint-lang.org now redirects to quint.sh. — observed 301 (verified)

**FizzBee MBT**
- FizzBee MBT generates test sequences from a Python-like formal spec aimed at concurrent, non-linearizable systems, and runs them against the real system. — [fizzbee.io/testing](https://fizzbee.io/testing/) (verified via search)
- Rust status conflicts between sources. The official getting-started and quick-start guides are Go-only and say Rust is "coming soon". A Show HN post said "only Go is supported. Java and Rust are next." A third-party mirror lists Rust as supported. — [quick start](https://fizzbee.io/testing/tutorials/quick-start/); [Show HN](https://hn.svelte.dev/item/45503780); contradicted by [tomevault mirror](https://tomevault.io/tome/fizzbee-io/fizzbee) (verified via search)

**quickcheck / bolero**
- Not re-verified this session. Background: `quickcheck` is the original Haskell-style Rust port, with type-based `Arbitrary` and simpler shrinking. `bolero` (from the s2n-quic team, https://github.com/camshaft/bolero) is a single harness API that runs as a PBT test, under libFuzzer/AFL/honggfuzz, or under Kani. (unverified)

### Inferences
- Rough decision rule:
  - Single-threaded component with a simple model (map, log, index): proptest-state-machine or arbtest/hegel, with a Rust reference model.
  - Want coverage-guided depth: put the same `arbitrary`-based harness under cargo-fuzz, or use bolero.
  - Custom sync primitives: loom. Whole-subsystem concurrency: shuttle.
  - Panic-freedom or memory safety of small parsers or unsafe code: Kani.
  - Protocol design you want exhaustively checked in Rust: stateright.
  - Protocol spec owned by researchers or designers: Quint plus quint-connect, or TLA+ trace validation.
- For a sans-IO design, a pure state machine core makes every one of these tools cheaper. The proptest or quint-connect driver calls `step(input) -> outputs` directly, with no I/O mocking.

### Gaps
- I did not verify quickcheck, bolero, P language, or the current state of a FizzBee Rust adapter. The user's environment has a `fizz-mbt` skill that mentions Rust, which suggests an adapter may now exist; it is not confirmed in public docs.
- Hegel's stateful or rule-based testing API (Hypothesis has `RuleBasedStateMachine`) is not documented in the announcement.

## Canonical writeups: what to read and what each teaches

### Takeaway
Read the ShardStore paper first: it is Rust, industrial, and shows the whole method. Then read Hughes for the theory and practice of state-machine PBT, Goldstein et al. for empirical evidence of how practitioners use PBT, and matklad's posts for small, dependency-light Rust harness patterns.

### Cited Findings
- **Bornholt et al., "Using Lightweight Formal Methods to Validate a Key-Value Storage Node in Amazon S3", SOSP 2021 (Best Paper).** — [PDF](https://jamesbornholt.com/papers/shardstore-sosp21.pdf); [slides](https://www.cs.utexas.edu/~bornholt/papers/shardstore-sosp21.slides.pdf); [Amazon Science](https://www.amazon.science/blog/aws-team-wins-best-paper-award-for-work-on-automated-reasoning) (verified). What it teaches, with quotes and numbers from the PDF text:
  - ShardStore is more than 40,000 lines of Rust. Reference models are executable Rust and are tiny ("1% of the implementation code"). For example, the model of an LSM tree is a hash map. The models are reused as mocks in unit tests, which keeps them current.
  - Correctness is decomposed into three parts: (1) sequential crash-free conformance by PBT; (2) crash consistency, where the reference model is extended to define what data may be lost after a crash; (3) concurrent executions, checked for linearizability against separate models with stateless model checking.
  - The PBT pattern (`#[proptest] fn proptest_index(ops: Vec<IndexOp<..>>)`) uses an operation-alphabet enum that includes background operations (reclamation, clean reboots). These are no-ops in the model but validate that they do not corrupt state. Each operation is applied to both model and implementation, the results are compared, and invariants are checked. The disk is an in-memory user-space disk so runs are deterministic.
  - Coverage techniques: argument bias (for example, Get keys biased toward previously Put keys), with the caution "only bias where we have quantitative evidence"; code coverage metrics to find blind spots; and failure injection with a relaxed conformance check.
  - Minimization: one failure minimized to 6 operations including 1 crash. Design for minimization by removing or controlling nondeterminism and by choosing the operation alphabet with shrinking in mind.
  - Results: prevented 16 issues from reaching production. Checking code is about 12-13% of the codebase. 18% of model and harness code was written by the engineering team, not the formal methods experts. Prusti was tried for proving model properties, and Crux was used to prove deserialization panic-free.
- **AWS follow-up: "How automated reasoning helps us innovate at S3 scale"** — [AWS Storage Blog](https://aws.amazon.com/blogs/storage/how-automated-reasoning-helps-us-innovate-at-s3-scale) (verified via search; not read)
- **Goldstein, Cutler, Dickstein, Pierce, Head, "Property-Based Testing in Practice", ICSE 2024.** Based on 30 in-depth interviews with experienced PBT users at Jane Street. It covers how developers use PBT, where it is strong or weak, and which improvements would help, and notes PBT traction at Amazon, Volvo and Stripe. — [ICSE page](https://conf.researchr.org/details/icse-2024/icse-2024-research-track/90/Property-Based-Testing-in-Practice); [PDF](https://cis.upenn.edu/~bcpierce/papers/icse24-pbt-in-practice) (verified via search). Pierce gave a related talk, "Properties for the People" (Feb 2026), listed at [Pierce publications](https://www.cis.upenn.edu/~bcpierce/papers/chronological.html) (verified via search)
- **John Hughes, "Experiences with QuickCheck: Testing the Hard Stuff and Staying Sane"** (2016, LNCS 9600, pp. 169–186, DOI 10.1007/978-3-319-30936-1_9). A 15-year experience report covering Quviq's stateful-testing extensions (state machine models), AUTOSAR acceptance testing for Volvo Cars, and race detection (PULSE/linearizability) for a bug at Klarna. — [Chalmers record](https://research.chalmers.se/en/publication/232550) (verified). Talk: [InfoQ "Testing the Hard Stuff and Staying Sane"](https://infoq.com/presentations/testing-techniques-case-study) (verified via search); [GitHub Next speaker series 2020](https://githubnext.com/speakerseries/JohnHughes) (verified via search)
- **Hughes, "How to Specify It!"** (TFP 2019), a guide to choosing properties, including model-based properties. — (unverified)
- **matklad posts:**
  - "Properly Testing Concurrent Data Structures" (2024) covers a deterministic interleaving harness, arbtest, exhaustigen, and model comparison. — [link](https://matklad.github.io/2024/07/05/properly-testing-concurrent-data-structures.html) (verified)
  - "Finding Bugs" (2026-09-19) is a regex fuzzer case study. It covers oracle and differential testing (`regex` vs `regex_lite`), small tricky inputs, swarm testing (randomizing the feature distribution per run), recursive size-bounded generators, and "fix the fuzzer first" when a bug escapes. It found a previously unknown regex bug. — [link](https://matklad.github.io/2026/09/19/finding-bugs.html) (verified)
  - "256 Lines or Less: Test Case Minimization" (2026-04-20): [link](https://matklad.github.io/2026/04/20/test-case-minimization.html); "Generate All the Things" (2021): [link](https://matklad.github.io/2021/11/07/generate-all-the-things.html); "How to Test" (2021): [link](https://matklad.github.io/2021/05/31/how-to-test.html) (verified, in blog index)
- **TigerBeetle (Zig, but the canonical DST reference):** swarm testing data structures — [TigerBeetle blog 2025-04-23](https://tigerbeetle.com/blog/2025-04-23-swarm-testing-data-structures/); protocol-aware DST — [TigerBeetle blog 2026-08-20](https://tigerbeetle.com/blog/2026-08-20-protocol-aware-dst/); VOPR internals — [vopr.md](https://github.com/tigerbeetle/tigerbeetle/blob/47aeb2212a255273dda508288412e537d11e4b7c/docs/internals/vopr.md) (all cited by matklad, verified as links)
- **Tzemanovič, "State machine testing with Proptest"**, the design rationale for proptest-state-machine. — [link](https://tzemanovic.codeberg.page/posts/state-machine-testing-with-proptest/) (verified)
- **Hillel Wayne:** not re-verified this session. Known leads: "Property Testing with Complex Inputs" and "Finding Property Tests" (hillelwayne.com, unverified), and his Increment piece "In praise of property-based testing" — [increment.com](https://increment.com/testing/in-praise-of-property-based-testing/) (verified via search; author not confirmed in snippet)

### Inferences
- The ShardStore method (Rust model, op-alphabet PBT, crash and concurrency extensions) maps almost directly onto proptest-state-machine plus loom/shuttle. It is the best template for a Rust project.
- Hughes and ShardStore agree on two points: models must be much simpler than the implementation, and minimization quality drives adoption.

### Gaps
- I did not verify Tyler Neely (sled), Jane Street blog posts, Datadog, or Hillel Wayne URLs. Tyler Neely's sled simulation and PBT writings and the sled repo are likely relevant but unconfirmed.

## Exemplar Rust codebases and files to study

### Takeaway
Confirmed exemplars: ShardStore (paper only, closed source), Jujutsu (proptest-state-machine), the quint-connect two-phase-commit example, Emerald/Malachite (`tests/mbt`), stateright's examples, and matklad's regex-fuzz. Others in the brief (rust-analyzer, Materialize, sled, TiKV) were not verified this session.

### Cited Findings
- ShardStore Fig. 3 (proptest conformance harness for the index) and Fig. 4 (Loom harness for the LSM-tree index, which caught a bug before code review). — [paper](https://jamesbornholt.com/papers/shardstore-sosp21.pdf) (verified)
- Jujutsu: `lib/testutils` and the jj-cli dev-deps use proptest-state-machine to model the working copy. — [jj mirror](https://git.joshthomas.dev/mirrors/jj/src/commit/ade2aea0159eb5488813588ddac26840e400cdee/lib/testutils) (verified via search); upstream https://github.com/jj-vcs/jj (unverified)
- quint-connect two-phase commit example (in the quint-connect repo), and Emerald `tests/mbt`. — [quint.sh](https://quint.sh/posts/quint_connect) (verified)
- stateright `examples/` (linearizable-register / ABD, Paxos). — [docs.rs](https://docs.rs/crate/stateright/latest) (verified via search)
- matklad/regex-fuzz, a small differential fuzzer. — [github](https://github.com/matklad/regex-fuzz) (cited by matklad, verified as link)
- The proptest repo's `proptest-state-machine/examples` (`state_machine_heap.rs`, `state_machine_echo_server.rs`). — (unverified file names)

### Inferences
- For a sans-IO Rust project, study jj's proptest-state-machine usage for idiom, ShardStore Fig. 3 for alphabet design, and quint-connect's `Driver`/`State` split for state projection.

### Gaps
- Materialize, sled, TiKV and rust-analyzer usage were not checked. rust-analyzer is known for fuzz targets and `arbitrary`-driven tests, but no file paths were confirmed.

## Linking specs and model checking to implementations (trace checking, conformance)

### Takeaway
There are two directions. Spec-to-implementation means generating traces from the spec and replaying them, as quint-connect and FizzBee MBT do. Implementation-to-spec means logging implementation traces and checking that the spec admits them, as TLA+ trace validation does. Both catch spec/implementation drift; neither is exhaustive.

### Cited Findings
- "Validating Traces of Distributed Programs Against TLA+ Specifications" (arXiv 2404.16075, 2024, Univ. Lorraine/Inria and Microsoft Research) reduces trace validation to constrained model checking with TLC. Traces need only contain updates to some variables, and TLC reconstructs the missing ones. It ships a Java instrumentation API. — [arXiv](https://arxiv.com/abs/2404.16075) (verified via search; author names not confirmed in snippet)
- TLA+ trace validation docs map recorded implementation states to spec states, and note that trace validation "is not exhaustive". — [docs.tlapl.us](https://docs.tlapl.us/_export/xhtml/using:tlc:trace_validation) (verified via search)
- TraceLink (OOPSLA/SPLASH 2025) links implementations to verified designs. It criticizes prior trace validation for needing manual logging and manual event-to-model mapping. — [SPLASH 2025](https://2025.splashcon.org/details/OOPSLA/149/TraceLinking-Implementations-with-their-Verified-Designs); [PDF](https://www.cs.ubc.ca/~bestchai/papers/oopsla25-trace-link.pdf) (verified via search)
- MongoDB's conformance checking (eXtreme Modelling, 2020): model-based trace checking against a highly abstract spec proved impractical for the server, while test-case generation worked for a smaller component. — [MongoDB blog](https://mongodb.com/blog/post/engineering/conformance-checking-at-mongodb-testing-our-code-matches-our-tla-specs) (verified via search)
- Multi-grained specifications for model checking distributed systems (arXiv 2409.14301). — [arXiv](https://arxiv.org/pdf/2409.14301) (verified via search; not read)
- quint-connect compares a projected implementation state to the spec state after every step. — [quint.sh](https://quint.sh/posts/quint_connect) (verified)

### Inferences
- The MongoDB lesson and the quint-connect design point the same way: conformance works best when spec actions map close to one-to-one onto implementation inputs. A sans-IO state machine whose `handle(event)` mirrors spec actions makes this nearly free. Emerald reports that this one-to-one mapping also made LLM-generated glue easy.

### Gaps
- I found no Rust-specific TLA+ trace-validation tooling. The P language's PObserve/PVerifier Rust integration was not verified.

## Developments 2024–2026

### Takeaway
The main recent changes: Hypothesis-quality PBT came to Rust (Hegel, March 2026); spec-to-Rust MBT became productized (quint-connect, December 2025, with LLM-maintained glue); Kani scaled to std-verification CI; and Shuttle, arbtest and DST-style swarm testing matured.

### Cited Findings
- Hegel/hegeltest released 2026-03-24, still beta. — [Antithesis](https://antithesis.com/blog/2026/hegel/) (verified)
- quint-connect released 2025-12-23, with an Emerald case study. — [quint.sh](https://quint.sh/posts/quint_connect) (verified)
- Kani: more than 16,000 harnesses per change in std verification, and contracts beyond bounded checking (2026). — [arXiv 2607.01504](https://arxiv.org/abs/2607.01504) (verified via search)
- Rust std verification challenges, with 6 of 9 completed challenges using Kani (Oct 2025). — [arXiv 2510.01072](https://arxiv.org/pdf/2510.01072) (verified via search)
- proptest-state-machine went from 0.3.0 (Mar 2024) to 0.9.0 (docs.rs, 2026). — [lib.rs](https://www.lib.rs/crates/proptest-state-machine); [docs.rs](https://docs.rs/proptest-state-machine/latest/proptest_state_machine/) (verified)
- FizzBee MBT launched (Go first; Rust announced as next). — [Show HN](https://hn.svelte.dev/item/45503780) (verified via search)
- matklad's 2026 posts on minimization and finding bugs, and TigerBeetle's protocol-aware DST (Aug 2026). — [matklad](https://matklad.github.io/2026/09/19/finding-bugs.html); [TigerBeetle](https://tigerbeetle.com/blog/2026-08-20-protocol-aware-dst/) (verified as links)

### Inferences
- The trend is toward cheap glue between models and code: LLM-written drivers, one harness that serves PBT, fuzzing and verification, and pure state-machine cores.

### Gaps
- I could not confirm the precise current versions of shuttle, loom or stateright beyond those listed, or any 2025–2026 stateright activity.
