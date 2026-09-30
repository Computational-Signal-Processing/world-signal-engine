# Deployment

This document covers running the engine as a long-lived service rather than a
demo. The demo path (`wse demo`) keeps everything in memory and forgets it on
exit. A deployment wants the opposite: state that survives a restart, a
detector that resumes warm, and an API that is not open to the internet.

## The short version

```bash
export WSE_API_KEYS="$(openssl rand -hex 32)"
export WSE_DATA_DIR=/var/lib/world-signal-engine
export WSE_RETENTION_DAYS=90
export WSE_RAW_MAX_BYTES=5368709120   # 5 GiB

wse serve --port 8080 --collect
```

Then put a TLS terminator in front of it. Nothing below is optional if the
process is reachable from anywhere but loopback.

## Persistence

`--data-dir` (or `WSE_DATA_DIR`) turns on the SQLite backend. Two things live
there:

| Path | Contents |
| --- | --- |
| `world-signal-engine.db` | observations, events, signals, sources, health, baselines, raw metadata |
| `raw/` | the retained raw payloads, sharded by hash prefix |

Without `--data-dir` the engine runs in memory. That is correct for the
synthetic world and the test suite, and wrong for anything that should still be
there tomorrow.

### What survives a restart

- **Observations, events and signals.** The API serves the same signals after a
  restart as before it. A signal's evidence still points at real observations.
- **Source health.** `last_success`, `last_failure`, counters and latency are
  restored, so a source that broke overnight is visibly broken rather than
  looking never-run.
- **Baselines.** The rolling statistics are reloaded, so the detector does not
  spend its first hour re-learning what "normal" means for every series.
- **Raw payloads.** `load_raw_index` runs at startup, so the drill-down's last
  step keeps working for payloads written by a previous process.

### Rehydration

`--rehydrate-history N` (default 500) replays the N most recent observations
per series into the detector's windows on startup. The startup log line reports
how many series were warmed:

```text
rehydrated detector state from storage series=14
```

If this reports `series=0` on a store that should have history, the store is
empty — not the same thing as the world being quiet.

## Retention

Two independent limits, both off by default:

| Flag | Env | Meaning |
| --- | --- | --- |
| `--retention-days` | `WSE_RETENTION_DAYS` | Delete observations older than this. `0` disables. |
| `--raw-max-bytes` | `WSE_RAW_MAX_BYTES` | Prune raw payloads, oldest first, until under this. `0` disables. |

Retention runs on a daily loop, not on the write path, so collection stays fast
and the policy is visible in one place.

Observations are aged out. **Events and signals are not.** A signal is the
conclusion a person was investigating; deleting it because its underlying
samples expired would be losing the conclusion to save the working. When a
signal's evidence is gone, the drill-down says so rather than pretending the
observation never existed.

Pruning raw payloads deletes the bytes and the metadata row, but leaves the
`RawReference` on the observation. The drill-down still reports *what* the
payload was and that it is no longer retained. Losing the bytes is a documented
policy; losing the reference would be data loss.

## Security

The engine was originally served open on a laptop. On a VM that is a mistake,
so the defaults now assume the worst.

### Authentication

Set `WSE_API_KEYS` (comma-separated) or `WSE_API_KEY` (single). Every route
requires a key except `/health`:

```bash
curl -H "Authorization: Bearer $WSE_API_KEYS" https://engine.example/signals
curl -H "X-API-Key: $WSE_API_KEYS" https://engine.example/signals
```

`/health` stays open so a load balancer can probe it; it returns counts, not
data. `/metrics` stays **behind** the key by default, because it exposes
operational detail useful for timing requests. `WSE_PUBLIC_METRICS=1` opens it
if a scraper cannot send credentials.

Key comparison is constant-time. A short-circuiting `==` leaks, through timing,
how many leading bytes of a guess are correct.

### Open mode

With no key configured the API is open. The CLI logs a warning at startup and
the process is meant to be reached only through loopback, a reverse proxy, or
an SSH tunnel. There is no flag that makes an open API safe on a public
address; if you need one, you need a key.

### CORS

No origins are allowed by default. The bundled UI is same-origin, so it needs
no CORS headers, and an unconfigured API tells no other page it may read the
responses. Set `WSE_CORS_ORIGINS` (comma-separated) to allow a separate front
end:

```bash
export WSE_CORS_ORIGINS="https://signals.example"
```

Only the listed origins receive an `Access-Control-Allow-Origin` header.

### Other limits

| Env | Default | Meaning |
| --- | --- | --- |
| `WSE_MAX_BODY_BYTES` | 1 MiB | Request body cap |
| `WSE_REQUEST_TIMEOUT_SECS` | 30 | Per-request timeout |
| `WSE_WEB_DIR` | `web` | Where the static UI is served from |

Responses carry `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY` and
`Referrer-Policy: no-referrer`.

### Graceful shutdown

`SIGTERM` and `SIGINT` stop the server cleanly: in-flight requests finish and
the database is closed, rather than being killed mid-write. This is what makes
`systemctl restart` safe.

## Running under systemd

```ini
[Unit]
Description=World Signal Engine
After=network-online.target

[Service]
Type=simple
User=wse
EnvironmentFile=/etc/world-signal-engine.env
ExecStart=/usr/local/bin/wse serve --port 8080 --collect
Restart=on-failure
RestartSec=5
# The engine only needs to write its data directory.
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/world-signal-engine

[Install]
WantedBy=multi-user.target
```

`/etc/world-signal-engine.env`:

```text
WSE_DATA_DIR=/var/lib/world-signal-engine
WSE_API_KEYS=<32 random bytes, hex>
WSE_RETENTION_DAYS=90
WSE_RAW_MAX_BYTES=5368709120
```

Keep that file mode `0600` and owned by the service user. It holds the API key.

## Backups

The whole state is two paths: the database file and the raw directory. SQLite
in WAL mode should be backed up with `sqlite3 ... ".backup"` (or with the
service stopped) rather than by copying the file out from under a running
process.

```bash
sqlite3 /var/lib/world-signal-engine/world-signal-engine.db \
  ".backup '/backup/wse-$(date +%F).db'"
```

The `raw/` directory can be copied with `rsync`; a payload missing from a
backup costs a drill-down step, not a signal.

## Verifying a deployment

```bash
# Liveness, no key needed.
curl -sf https://engine.example/health

# Authenticated data.
curl -sf -H "Authorization: Bearer $WSE_API_KEYS" \
  "https://engine.example/signals?active=true&limit=5"

# Metrics, behind the key.
curl -sf -H "Authorization: Bearer $WSE_API_KEYS" \
  https://engine.example/metrics | grep wse_signals_total

# An unauthenticated read must fail.
curl -s -o /dev/null -w '%{http_code}\n' https://engine.example/signals
# 401
```
