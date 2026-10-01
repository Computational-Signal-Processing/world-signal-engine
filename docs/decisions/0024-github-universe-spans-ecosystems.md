# 0024 — The GitHub universe spans ecosystems, not one language

- **Status:** accepted
- **Date:** 2026-10-01
- **Supersedes (in part):** the fixed-universe *membership* of
  [0020](0020-github-per-repo-series.md)

## Context

`github_repo_universe` (then `github_rust_activity`) measured the star counts of
a fixed universe of repositories. The universe was **Rust only**: 16 repos drawn
from the Rust ecosystem (`rust-lang/*`, `tokio-rs/*`, `serde-rs/*`, ...).

That is a sensor of one corner of software, not of "the software world". The
brief's objective is that the machine observes the *world*; a single-language
universe is blind to the most interesting software change there is — attention
moving from one ecosystem to another. It also made the SOFTWARE lens narrower
than its own description ("developer attention, releases and ecosystem
activity") claimed.

## Decision

The universe now spans the major language ecosystems, foundational
infrastructure, AI/ML and developer tooling:

- **Languages:** Rust, Go, Python, Node, Deno, Vue, Next, React.
- **Infrastructure:** Kubernetes, Docker (moby), Terraform, Redis, Postgres,
  Kafka, Grafana, Prometheus.
- **AI/ML:** PyTorch, Transformers, LangChain, Ollama.
- **Tooling:** VS Code, Neovim, Ruff, DuckDB.

The entity changes from `ecosystem_rust` to `ecosystem_open_source`, and the
source id from `github_rust_activity` to `github_repo_universe`. The
per-repository series discipline (dimension `repo`) is unchanged: each
repository is still its own baseline.

## Consequences

- A change in the aggregate now reflects a change in *the open-source
  ecosystem*, and a shift between language communities is visible.
- The universe grows from 16 to 24 repositories. At one request per repository
  per hour that is 24 requests/hour, still inside the unauthenticated 60/hour
  limit; a token raises it further.
- **Canonical names matter.** Renamed repositories (e.g. `facebook/react` →
  `react/react`) return a 301 that the transport does not follow, so a stale
  entry would silently fail every collection. Every entry is the canonical
  `owner/name`, and the source treats "all requests failed" as a source failure
  rather than zero activity.
- Existing stored observations under the old entity id remain in the database;
  new observations use the new entity. This is a rename, not a migration: the
  old series simply stops and the new one begins, which is honest for a sensor
  whose subject has changed.

## Alternatives considered

- **Keep Rust, add a second language-specific source.** Rejected: it multiplies
  sources to express one idea (ecosystem-wide attention) and would still require
  a union query to answer "is software attention moving".
- **Search-based universe (`language:* sort:updated`).** Rejected earlier in
  0020 and still rejected: membership churn makes the aggregate meaningless.
