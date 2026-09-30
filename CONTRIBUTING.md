# Contributing

Thanks for helping build the World Signal Engine.

## Before you start

Read [docs/philosophy.md](docs/philosophy.md). Most disagreements about this
project are really disagreements about the philosophy, and it is cheaper to have
that conversation before the code is written.

The two rules that shape everything else:

- **No LLM in the detection path.** Detection is mathematics. An LLM may later
  explain a signal; it may never decide one exists.
- **Absence is not an event.** A failed collector means *no data*, never *zero
  activity*.

## Getting set up

```bash
cargo build --workspace
cargo test  --workspace
```

No network access is required for the test suite.

## Before opening a pull request

Run all four and make sure they are clean:

```bash
cargo fmt   --all
cargo clippy --workspace --all-targets
cargo test  --workspace
cargo build --workspace --release
```

## What makes a change acceptable

1. **It is tested.** A bug fix comes with a test that fails before the fix. A
   feature comes with tests that exercise it through real code paths.
2. **It is a vertical slice.** Prefer one stage working end to end over five
   stages half-built.
3. **It is minimal.** Fix the problem in front of you; do not refactor the
   neighbourhood on the way past.
4. **It does not hide failure.** Never let an error path return an empty result
   that looks like success.
5. **It keeps sources independent.** One broken collector must not affect any
   other.

## Style

- Comments explain *why*. A comment earns its place by recording a non-obvious
  invariant, a workaround, or a deliberate trade-off — not by restating the code.
- Prefer a new type over a new boolean parameter.
- Do not create a second file with a suffix to hold a variant of an existing
  file. Edit the original.
- Keep the model in `wse-model` free of I/O and detection. It is the shared
  vocabulary, and everything else depends on it.

## Adding a source or a detector

Both are extension points and both are meant to be easy. See
[DEVELOPMENT.md](DEVELOPMENT.md) — a new source is a module, a fixture, a test
and two registrations, with no core code changes.

## Commit messages

Explain the change and why it was needed. If it fixes a bug, say what the bug
actually was; "fix bug" tells a future reader nothing.

## Reporting a problem

Include what you ran, what you expected, and what happened. If a collector
failed, include the source id and the error — but never paste credentials or API
keys into an issue.

## License

Contributions are accepted under AGPL-3.0-or-later.
