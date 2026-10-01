# Source semantic audit

A per-source audit of the 13 connected sources. It answers, for each source,
what one observation *means* — and, more importantly, what the resulting time
series does **not** mean even though the pipeline will happily compute on it.

This is not a restatement of [SOURCE_CATALOG.md](../SOURCE_CATALOG.md). Every
claim below was reached by reading the collector implementation and its fixture
and tracing the value through:

```text
SOURCE -> COLLECTOR -> NORMALIZATION -> OBSERVATION -> STORAGE -> BASELINE -> DETECTION
```

Audit run: 2026-09-30, `main` @ `399c880`, Rust 1.88.0.

Legend for detection eligibility:

| Class | Meaning |
| --- | --- |
| **DETECTABLE** | The series is a real measurement of a stable subject; anomaly/early detection is meaningful as-is. |
| **DETECTABLE_WITH_CONSTRAINTS** | Detectable, but only under a stated condition (cadence, aggregation, or de-duplication) that must hold. |
| **EVIDENCE_ONLY** | The value is real and worth storing for drill-down and convergence, but its own series is not a trustworthy basis for an anomaly. |
| **NOT_DETECTABLE** | The series does not mean what detection would assume. Detection must not run. |

## Cross-cutting finding: observation identity was derived from the whole payload

> **Status: FIXED.** The identity contract now derives an observation's id from
> the record's **stable record key**, not the whole-body payload hash. The five
> probes below were promoted to active regression tests in
> `crates/sources/tests/semantic_regression.rs` and pass. The finding is kept
> here as the record of what was wrong and why.

The single most important semantic bug in the network was in the *shared* id
derivation, not in any one collector. It is documented here once and referenced
by each affected source.

`ObservationId::deterministic` seeded the id with the **whole-body payload hash**:

```text
series_key | observed_at | payload_hash | identity
```

For a *single-record* source this is harmless: the body changes only when the
record changes. But for every source that returns a **multi-record window or
feed**, the body changes whenever *any* record in it changes, so the payload
hash changes, so **the ids of the records that did not change were re-minted**.

The dedup check in `Engine::ingest_observations` is `contains_observation(id)`.
Because the id changed, the "unchanged" record was not recognised as a duplicate
and was ingested as a brand-new observation. It was then pushed into the rolling
window as a *second* point at the *same* `observed_at` (the window dedups on
nothing — see `RollingWindow::push`), which double-counts and distorts the
baseline.

Reproduced by probe (each failed against `main` before the fix):

| Source | Probe | Result before fix |
| --- | --- | --- |
| USGS | same quake, feed with one extra quake | id changed `obs_8293…` → `obs_4aff…` |
| AFAD | same event, window slid by one | id changed `obs_0f3e…` → `obs_3cbb…` |
| ECB | same rate, `lastNObservations=7` slid | id changed `obs_8064…` → `obs_2195…` |
| NOAA Kp | same 3-hourly point, window slid | id changed `obs_7815…` → `obs_09db…` |
| GDELT | same bucket, 1-day window slid | id changed `obs_7f50…` → `obs_3c4b…` |

### The fix

Identity is now the record's own stable key:

```text
series_key | observed_at | record_key
```

where `record_key` is the strongest stable identity the source offers. The
payload hash is used *only* as the fallback for a source with exactly one record
per series per timestamp, where the hash already is the record identity.

| Source | Record key used |
| --- | --- |
| USGS | the GeoJSON feature id (the quake's upstream id) |
| AFAD | `eventID` |
| ECB | the SDMX period (`YYYY-MM-DD`) |
| NOAA Kp | the point's `time_tag` |
| GDELT | `series \| bucket date` |
| NWS | the full series key, so each (entity, severity) count is distinct |

Each key excludes the measured value, so a record keeps its identity while its
measurement changes (a revised magnitude is a change in value, not a new quake).

### Follow-up: NWS severity collision (F1.1)

The F1 change surfaced a second, *dimension-level* collision in NWS. The id is
seeded with the **base** series key (`source::entity::metric::unit`), which does
not include dimensions. NWS emits one count per (entity, severity) — the four
severities of one entity share a base key — so all four collapsed onto one id:
20 parsed observations produced only 5 ids, and `contains_observation` dropped
three of every four counts before they could reach storage or the baseline.

Fixed the same way: NWS passes its full `series_key()` (base key *plus*
dimensions) as the record key. The dimensions stay out of the series key, so
each (entity, severity) count is its own series **and** its own identity. The
same audit of the other dimensioned sources shows they already encode their
dimension in the entity (EONET `natural_<category>`, AFAD `province_<x>`, GDELT
`topic_<query>`), so NWS was the only source affected.

Impact by source character:

- **Append-only event feeds** (USGS, AFAD, NASA NEO): every re-poll of the same
  quake re-inserts it. On a 60-second poll over a 1-hour feed, a single
  earthquake can be counted up to ~60 times. This is a direct, silent corruption
  of the magnitude series.
- **Sliding aggregate windows** (ECB, NOAA Kp, GDELT): every retained point is
  re-minted each poll. The window fills with duplicates of the same timestamp.
- **Whole-catalog snapshots** (CISA KEV, Crossref, arXiv): the observation is
  *one* value per collection timestamped at collection time, so a fresh id per
  poll is correct. These are **not** affected.

The fix is to derive the id from the *record*, not the payload: seed the hash
with the record's stable key (`event_id`, `time_tag`, `date`, story/repo id) via
the existing `identity` mechanism, or hash only the record's own bytes. That is
a change to the id-derivation contract and must be made deliberately, with a
regression test per affected source, before any fix is proposed.

## Cross-cutting finding: `feeds_lenses` is now enforced at runtime

`Source::feeds_lenses` was previously checked by `crates/cli/tests/lens_coverage.rs`
for *existence* (a source may not name a lens that does not exist) but was
**never consulted at runtime**: a signal's lens matches were computed by
`Lens::matches` from the signal's category, entities, keywords and location only.
So a source's declared lens coverage and the lens a signal actually landed in
could disagree, and the coverage test could not see it.

Concretely: `nasa_eonet` declares `feeds_lenses: ["lens_earth"]` but its source
category is `earth`, while `lens_earth` filters on `categories: [geophysics,
environment, weather]`. EONET's signals therefore **never appeared in EARTH**,
despite the declaration.

**Fixed.** The engine now reads the source→lens declarations from the catalog and
routes a signal to the lenses its sources declare, by provenance, in addition to
the lens's own filters (`SignalEngine::with_source_lenses`,
`assign_lens_matches`). The union is deliberate: provenance can only *add*
visibility, never replace a filter match, and a declared-but-unconfigured lens
is dropped rather than invented. Because the routing is derived from the catalog
in `Engine::register_source`, nothing in the core names a particular source or
category, and a new source is routed automatically.

The regression is now enforced two ways:

* `crates/cli/tests/lens_coverage.rs::a_declared_lens_is_reachable_at_runtime`
  loops over the whole catalog and asserts every declared lens is reachable.
* `crates/cli/tests/lens_routing.rs` drives the real EONET and AFAD fixtures
  through the real engine with the shipped lens set, and checks the signal is
  reachable through its declared lens *and* that provenance is preserved
  (`SIGNAL → EVENT → OBSERVATION → SOURCE`, value and raw reference intact).

## 1. usgs_earthquakes — USGS Earthquake Feed

**Collector:** `crates/sources/src/usgs.rs`, poll `all_hour.geojson` every 60s.
**Observation:** one earthquake. `metric=earthquake_magnitude`, `unit=magnitude`,
entity `region_<place-after-last-comma>`, value = magnitude, `observed_at` =
quake time (source), location = epicentre.

1. **What one observation represents:** a single seismic event's magnitude.
2. **Type:** event (geophysical), with a physical quantity attached.
3. **Population stable?** Yes. Earthquakes are events that happened; they never
   un-happen. A new quake is a new record.
4. **Aggregate usable for anomaly detection?** Only after de-duplication. As
   implemented, no (see cross-cutting finding): re-polls duplicate quakes.
5. **What an increase means:** more seismic energy released, in that region, in
   the feed's 1-hour window — *if* de-duplicated.
6. **False increases:** (a) the payload-hash id bug re-inserting the same quake
   on every poll; (b) the region key is the free-text `place` after the last
   comma, so USGS re-phrasing a location splits or merges regions; (c) the feed
   is "past hour" only, so a quiet hour followed by a busy hour is a window
   artifact, not a world change.
7. **Appropriate baseline:** rolling statistics over magnitude *per region*,
   ideally with a rate term (count per hour). Mean/σ of magnitude is weakly
   meaningful; magnitude is already log-scaled.
8. **Temporal resolution:** event time, irregular; poll every 60s over a 1h
   rolling feed.
9. **Independent?** Yes — a distinct seismic network. Overlaps AFAD for Turkey
   (see §13).
10. **Can two sources represent the same event?** Yes — a Turkey quake appears in
    both USGS and AFAD. Different event ids, so they are never merged; this is a
    convergence case, not a dedup case.
11. **Lenses:** EARTH, TURKEY.
12. **Never infer:** that a *magnitude* rise is a rise in *activity* (magnitude
    is not frequency); that "no observations this hour" means "no earthquakes"
    (the feed is a window, and a failed poll is not a quiet planet).

**Eligibility: DETECTABLE_WITH_CONSTRAINTS** — constraint: per-record id
de-duplication must be fixed first.

## 2. afad_earthquakes — AFAD Turkey Earthquake Catalogue

**Collector:** `crates/sources/src/afad.rs`, a filter window per poll.
**Observation:** one earthquake, entity `province_<province>`,
`metric=earthquake_magnitude`, `observed_at` = local time − 3h (source).

1. **What one observation represents:** a single AFAD catalogue event.
2. **Type:** event (geophysical), regional.
3. **Population stable?** Yes — events are historical facts.
4. **Aggregate usable?** Only after de-duplication. `with_identity(event_id)` is
   set, but the payload hash still seeds the id, so the same event re-collected
   in a slid window gets a new id (probe-confirmed).
5. **What an increase means:** more recorded seismicity in Turkish provinces.
6. **False increases:** (a) the id bug; (b) AFAD's window overlapping between
   polls; (c) `isEventUpdate=true` rows — a revised magnitude for an existing
   event is parsed as a *new* observation, so a correction can look like
   activity. This is a genuine semantic gap: updates should supersede, not
   append.
7. **Appropriate baseline:** per-province rolling statistics; province is a
   stable administrative key, which is better than USGS's free-text region.
8. **Temporal resolution:** event time (UTC+3 → UTC), window-based polling.
9. **Independent?** Yes, and complementary to USGS. Institutional (AFAD) vs
   international (USGS) networks — a good convergence pair for TURKEY.
10. **Same real-world event as another source?** Yes, with USGS (Turkey quakes).
11. **Lenses:** TURKEY, EARTH.
12. **Never infer:** that a province's rise is a national rise (provinces are
    independent series); that an `isEventUpdate` row is a new earthquake.

**Eligibility: DETECTABLE_WITH_CONSTRAINTS** — constraints: per-record id
de-duplication, and event-update semantics.

## 3. nasa_neo — NASA Near-Earth Object Feed

**Collector:** `crates/sources/src/nasa.rs`, daily feed (DEMO_KEY or `NASA_API_KEY`).
**Observation:** one UTC day's count of close approaches. Entity is
**`neo_class_all`** (the population, the right subject for a rate);
`metric=neo_close_approaches`, `unit=approaches`.

1. **What one observation represents:** the number of close approaches the feed
   records on one UTC day.
2. **Type:** physical measurement of activity (a rate).
3. **Population stable?** Yes — the entity is the whole near-Earth-object
   population, and the day is the record key, so a day retained across polls
   keeps its identity.
4. **Aggregate usable?** Yes. The series is a per-day count, so day-to-day
   comparison is meaningful. (The old per-object miss-distance series was not —
   it interleaved unrelated rocks.)
5. **What an increase means:** more objects passed close by that day than usual.
6. **False increases:** the feed's seven-day window and the day's partial count
   (approaches can still arrive later today); both are low-end noise the
   baseline absorbs.
7. **Appropriate baseline:** the day's count over time; a positive deviation is
   the interesting direction, which is now the one the detector measures.
8. **Temporal resolution:** per-day, `observed_at` = the day's UTC midnight.
9. **Independent?** Yes.
10. **Same event as another source?** No.
11. **Lenses:** SPACE.
12. **Never infer:** that a rising count is a threat; anything about a single
    object from the population series (the closest object is a drill-down
    attribute, not the series).

**Eligibility: DETECTABLE (CAP-2D)** — the daily approach count is a coherent
series. The day's closest object is preserved as attributes for drill-down. See
`docs/decisions/0017-neo-daily-approach-count.md`.

## 4. nws_alerts — NWS Active Weather Alerts

**Collector:** `crates/sources/src/nws.rs`, active-alert snapshot every 600s.
**Observation:** count of *active* alerts, per severity (`dimension=severity`),
nationally (`weather_united_states`) and per state (`weather_us_<code>`).

1. **What one observation represents:** how many alerts of a severity are active
   right now, nationally or in a state.
2. **Type:** activity proxy / snapshot count of a live state.
3. **Population stable?** Yes *as a snapshot* — the set is re-counted each poll,
   not sampled.
4. **Aggregate usable?** Yes. It is a true gauge: it can rise and fall.
5. **What an increase means:** more people currently under a hazard of that
   severity.
6. **False increases:** (a) NWS bulk-issuing alerts for one weather system (a
   single storm names dozens of counties → the national count jumps without the
   world getting worse); (b) alert *expiry* and *re-issue* churn; (c) a state
   code parse miss would drop or move counts.
7. **Appropriate baseline:** rolling mean/σ of the count *per severity* (the
   severity dimension already separates the series). Extreme/Severe are the
   series worth watching.
8. **Temporal resolution:** snapshot at poll time (10 min).
9. **Independent?** Yes.
10. **Same event as another source?** Possibly overlaps EONET (a storm appears in
    both as an NWS alert and an EONET storm event) and USGS (an earthquake can
    trigger alerts). Not mergeable — different granularity.
11. **Lenses:** none configured today (category `weather` is claimed by the
    unfed AGRICULTURE lens, and EARTH lists `weather` but has no bbox). **Gap:**
    NWS is a Tier-1 real-time source that reaches the WORLD view but no domain
    lens.
12. **Never infer:** that the national count is a sum of human impact; that a
    zero count is calm if the collector failed (rule 29 — a failed poll is not a
    zero count).

**Eligibility: DETECTABLE** — the strongest "true gauge" in the network.

## 5. nasa_eonet — NASA EONET Natural Events

**Collector:** `crates/sources/src/eonet.rs`, open-events snapshot every 1800s.
**Observation:** count of open events per category (`dimension=category`), entity
`natural_<category>`, `observed_at` = **collection time**.

1. **What one observation represents:** how many natural events of a category
   are currently open.
2. **Type:** activity proxy / snapshot count.
3. **Population stable?** Yes as a snapshot; the category set is fixed in code so
   a category falling to zero stays visible.
4. **Aggregate usable?** Yes.
5. **What an increase means:** more open events of that category (e.g. more open
   wildfires).
6. **False increases:** (a) EONET's reporting lag — a fire that started hours ago
   may be added late, so the count jumps on *ingestion*, not on ignition; (b)
   closure latency makes the count sticky; (c) a category with a single long-lived
   event dominates the count.
7. **Appropriate baseline:** rolling statistics per category; watch the
   *derivative* (new events per interval) more than the level.
8. **Temporal resolution:** snapshot at poll time (30 min). Because `observed_at`
   is collection time, the series is well-formed (no payload-hash duplication).
9. **Independent?** Yes.
10. **Same event as another source?** Yes — overlaps NWS (storms) and GDELT
    (news about wildfires/storms). Convergence only.
11. **Lenses:** EARTH.
12. **Never infer:** that an open-event count is a count of *new* events; that a
    late addition is a new ignition.

**Eligibility: DETECTABLE**

## 6. gdelt_news_volume — GDELT News Volume

**Collector:** `crates/sources/src/gdelt.rs`, `timelinevol` for a fixed query
(`oil supply`) over `timespan=1d`, every 900s.
**Observation:** share of global news coverage matching the query, per 15-min
bucket. Entity `topic_oil supply`.

1. **What one observation represents:** the fraction of world news coverage, in
   one time bucket, that matched "oil supply".
2. **Type:** news activity (a *proxy* for attention, not for oil supply).
3. **Population stable?** The *query* is fixed, but GDELT's global corpus and its
   indexing lag are not; the corpus is continuously revised.
4. **Aggregate usable?** Yes for *attention*, with constraints. Each poll
   re-fetches the whole 1-day window, so it is affected by the payload-hash id
   bug (probe-confirmed).
5. **What an increase means:** a larger share of news coverage is about the
   tracked topic.
6. **False increases:** (a) the id bug re-ingesting the whole day each poll; (b)
   GDELT's own coverage sparsity for a topic at a given hour (the denominator is
   "all news", which itself varies); (c) rate limiting returns a **plain-text
   200** — the collector correctly rejects this as a parse error, which is the
   one place the "no data ≠ zero" rule is well handled; (d) a query that is too
   broad drifts with unrelated news.
7. **Appropriate baseline:** rolling statistics over the share, but *attention*
   baselines are spiky; a percentile/EWMA view is more honest than a z-score.
8. **Temporal resolution:** 15-minute buckets, 1-day window, 15-minute poll.
9. **Independent?** Yes as a feed, but it is the *least* independent in spirit:
   news volume about a topic is downstream of the events other sources measure
   directly.
10. **Same event as another source?** Frequently — this is the classic
    convergence partner for EONET/NWS/USGS.
11. **Lenses:** GLOBAL EVENTS.
12. **Never infer:** that rising coverage means the underlying condition worsened
    (coverage can rise because of *discussion*); that a fall means improvement;
    that the topic's series is about "oil" — it is about news *mentioning* oil
    supply.

**Eligibility: DETECTABLE_WITH_CONSTRAINTS** — constraint: id de-duplication,
and it should be treated as an *attention* series, never as the phenomenon.

## 7. hackernews_frontpage — Hacker News Front Page

**Collector:** `crates/sources/src/hackernews.rs`, top-stories list then per-item
fetch, every 600s. `measurement: fixed_universe`, tier 3.

1. **What one observation represents:** the current score (points) of one tracked
   story, by durable story id.
2. **Type:** platform behavior (attention on a link).
3. **Population stable?** **Yes (fixed, CAP-2C).** The collector now commits the
   universe on its first successful resolution and carries it across polls in
   `HackerNewsCollector::universe`. A tracked story that leaves the front page is
   **kept** — its score is still a comparable measurement, and evicting it would
   make the series move with membership rather than with attention. A slot is
   freed only when an item genuinely disappears (a 404); a transient failure
   holds the slot, so a network blip is never read as a story vanishing.
4. **Aggregate usable?** Per story, yes: each tracked id is a real series
   (score over time). The *sum* across the universe still mixes stories of
   different ages, so the per-story series is the meaningful one.
5. **What an increase means:** a known story is gaining attention.
6. **False increases:** a story's score climbing naturally, and time-of-day
   variation in the top list (which no longer changes membership).
7. **Appropriate baseline:** per-story score over time — a real series for each
   durable story id.
8. **Temporal resolution:** 10-minute poll; `observed_at` = collection time.
9. **Independent?** Yes.
10. **Same event as another source?** Occasionally (a story about a GDELT topic).
11. **Lenses:** SOFTWARE.
12. **Never infer:** that the series is "developer attention" in general — it is
    the score of the specific stories committed to the universe.

**Eligibility: DETECTABLE** (CAP-2C). The catalog/implementation contradiction
is resolved: the collector honours `fixed_universe` by committing and reusing the
universe, so the engine's detection now runs on a stable population. See
`docs/decisions/0016-hackernews-committed-universe.md`.

## 8. github_rust_activity — GitHub Rust Repository Universe

**Collector:** `crates/sources/src/github.rs`, one request per repo in a
16-repo `UNIVERSE` constant, every 3600s. `measurement: fixed_universe`, tier 3.

1. **What one observation represents:** the star count of one named repository,
   by `identity = owner/name`, and — since CAP-2D — its own series
   (`dimension: repo = owner/name`).
2. **Type:** platform behavior (attention on a project).
3. **Population stable?** **Yes, genuinely** — the universe is a hard-coded
   constant, re-measured each poll. This one *does* honour the contract.
4. **Aggregate usable?** Yes, and it is the well-built half of the pair.
5. **What an increase means:** the tracked project gained stars.
6. **False increases:** (a) a star-bot wave on one repo; (b) ~~the *one-time*
   ~1527σ cold-start~~ **fixed (CAP-2D/0020)**: each repository is now its own
   series, so a first appearance is judged against that repository's own
   history, not a pool of the others' star counts; (c) `observed_at` =
   collection time, so a poll delay shifts the point, not its value.
7. **Appropriate baseline:** rolling statistics **per repository** — now
   achieved by the `repo` dimension, so `repo_stars` is a per-repo series and a
   level z-score is judged within that repository's own history.
8. **Temporal resolution:** hourly, `observed_at` = collection time.
9. **Independent?** Yes.
10. **Same event as another source?** Weakly, with Hacker News (a repo trending
    on HN may also gain stars). Convergence only.
11. **Lenses:** SOFTWARE.
12. **Never infer:** that a star rise is adoption or usage; that the aggregate is
    "the Rust ecosystem" — it is 16 chosen projects.

**Eligibility: DETECTABLE (CAP-2D)** — the cold-start constraint is resolved by
the per-repository series; a repository is not judged until its own baseline has
the minimum samples. See `docs/decisions/0020-github-per-repo-series.md`.

## 9. cisa_kev — CISA Known Exploited Vulnerabilities

**Collector:** `crates/sources/src/cisa_kev.rs`, whole catalog, every 86400s.
**Two observations per poll:** `kev_added` (count dated to the collection day)
and `kev_catalog_total` (catalog size). Entity `cyber_kev`.

1. **What one observation represents:** `kev_added` = vulnerabilities added to
   the catalog **that UTC day**; `kev_catalog_total` = size of the catalog.
2. **Type:** registry activity (an authoritative institutional list).
3. **Population stable?** Yes — the catalog is append-only and authoritative.
4. **Aggregate usable?** Yes for both.
5. **What an increase means:** `kev_added` rising = more vulnerabilities newly
   *known to be exploited* that day; `kev_catalog_total` rising = the catalog
   grew.
6. **False increases:** (a) CISA batch-adds (one big ingestion day looks like a
   spike); (b) ~~`kev_added` is a trailing 7-day window sampled daily, so
   consecutive values overlap by 6 days~~ **fixed (CAP-2D/0018)**: `kev_added` is
   now a non-overlapping daily count, so a daily z-score is meaningful; (c)
   `kev_catalog_total` is monotonic, so a level z-score only ever fires upward.
7. **Appropriate baseline:** for `kev_added`, the day's count over time (no
   window overlap to correct for); for `kev_catalog_total`, the *first
   difference* (additions per day), not the level.
8. **Temporal resolution:** daily poll; `observed_at` = collection time, the
   counted day is the `day` attribute and the record key.
9. **Independent?** Yes.
10. **Same event as another source?** No.
11. **Lenses:** CYBER.
12. **Never infer:** that `kev_added` rising is a same-day surge (it is a
    single-day count); that `kev_catalog_total` ever measuring "activity" — it is
    a cumulative level; that a quiet catalog means a quiet threat landscape (KEV
    only lists *confirmed* exploitation).

**Eligibility: DETECTABLE_WITH_CONSTRAINTS** — constraint: `kev_catalog_total`
only as a differenced series (done, CAP-2B); `kev_added` is a non-overlapping
daily count (done, CAP-2D).

**Update (CAP-2B):** `kev_catalog_total` is now differenced, not detected on as
a level. The catalog declares `kev_catalog_growth = Delta(kev_catalog_total)` and
the raw total is **evidence-only** (stored, not detected on); detection runs on
the derived growth series. This is the same declared-derivation mechanism as
arXiv. See `docs/decisions/0015-cisa-derived-growth.md`.

**Update (CAP-2D):** `kev_added` is now the count of vulnerabilities dated to
the collection day, not a trailing 7-day window. Consecutive daily points no
longer overlap, so a daily z-score is meaningful and F5 is closed. `kev_added`
and `kev_catalog_growth` are two independent measurements of the same daily
additions. See `docs/decisions/0018-kev-daily-additions.md`.

## 10. ecb_exchange_rates — ECB Euro Reference Rates

**Collector:** `crates/sources/src/ecb.rs`, `D.USD.EUR.SP00.A`,
`lastNObservations=7`, every 86400s.
**Observation:** one daily reference rate; entity `fx_usd_eur`,
`metric=exchange_rate`, `observed_at` = the SDMX date.

1. **What one observation represents:** the official USD/EUR reference rate for
   one business day.
2. **Type:** physical/institutional measurement (an official fixing).
3. **Population stable?** Yes — it is the same published series every day.
4. **Aggregate usable?** Yes, but it is affected by the payload-hash id bug
   (probe-confirmed): the 7-day window is re-minted each poll.
5. **What an increase means:** the euro strengthened against the dollar that day.
6. **False increases:** (a) the id bug re-inserting the window; (b) the ECB
   publishes on TARGET business days only — weekends/holidays are **absent**, not
   zero, so the series has gaps that a naive time-based baseline misreads; (c)
   the rate is a *fixing*, not a live market price.
7. **Appropriate baseline:** rolling statistics over the daily rate, but the
   detector should be *gap-aware* (missing business days must not read as
   staleness). For finance, a relative-change baseline is more natural than a
   level z-score.
8. **Temporal resolution:** daily, business days only.
9. **Independent?** Yes — the only market source present.
10. **Same event as another source?** No.
11. **Lenses:** FINANCE.
12. **Never infer:** that a missing day is a flat day; that this single USD/EUR
    pair is "the markets"; that a reference-rate move equals a tradable move.

**Eligibility: DETECTABLE_WITH_CONSTRAINTS** — constraints: id de-duplication and
business-day gap handling.

## 11. crossref_works — Crossref Scholarly Works

**Collector:** `crates/sources/src/crossref.rs`, five fixed topics, `rows=0`
`total-results` over a `WINDOW_DAYS = 1` created-date window, every 86400s.
**Observation:** count of works registered on the measured day, per topic
(`research_<slug>`).

1. **What one observation represents:** how many works matching the topic were
   *registered with Crossref* on the measured day (the day before collection).
2. **Type:** registry activity (metadata registration).
3. **Population stable?** Yes — fixed topic, fixed window length.
4. **Aggregate usable?** Yes.
5. **What an increase means:** more works registered — but note *registration* is
   not *publication*.
6. **False increases:** (a) registration lag — publishers deposit in batches, so
   a batch day spikes; (b) the query is `query.bibliographic`, a fuzzy match, so
   a topic's count can drift with wording; (c) ~~window overlap~~ **fixed
   (CAP-2D/0019)**: the window is now a single completed day, so consecutive
   polls share no day; (d) ~~the newest day is always partially deposited~~ **fixed
   (CAP-2D/0019)**: the measured day is yesterday, fully deposited, so the latest
   point is no longer structurally depressed.
7. **Appropriate baseline:** the day's count over time; a daily z-score is
   meaningful now that the series is a non-overlapping, completed-day count.
8. **Temporal resolution:** daily poll, one completed day (yesterday).
9. **Independent?** Yes, but overlaps arXiv (see §12).
10. **Same event as another source?** Yes — a preprint often becomes a Crossref
    work. **This is the strongest same-event pair in the network.**
11. **Lenses:** SCIENCE, AI.
12. **Never infer:** that a count is *publication* (it is registration); that AI
    topic growth is global research growth.

**Eligibility: DETECTABLE (CAP-2D)** — the series is a single completed day, so
the window-overlap and partial-latest-point constraints are both resolved. See
`docs/decisions/0019-crossref-single-day-window.md`.

## 12. arxiv_submissions — arXiv Preprint Velocity

**Collector:** `crates/sources/src/arxiv.rs`, four fixed categories, `max_results=1`
to read `opensearch:totalResults`, every 86400s.
**Observation:** the **cumulative total** number of preprints in a category,
entity `arxiv_<slug>`, `metric=preprint_total`, `observed_at` = collection time.

1. **What one observation represents:** the total number of preprints ever in
   that arXiv category.
2. **Type:** registry activity (a cumulative level).
3. **Population stable?** Yes — the category is fixed.
4. **Aggregate usable?** **Not as a level.** It is a near-monotonic cumulative
   count; a level z-score on it is close to meaningless (it will slowly trend up
   and rarely deviate).
5. **What an increase means:** the category grew — but the *doc comment claims a
   velocity* ("differenced over a fixed window it becomes a submission-velocity
   series") while the **code never differences it**. The stored metric is the
   total, not the velocity. This is a doc/implementation mismatch.
6. **False increases:** (a) arXiv's daily bulk announcement — all of a day's
   submissions appear at once, so the difference is a step, not a rate; (b) the
   total only moves when arXiv indexes, so the derivative is a batch signal.
7. **Appropriate baseline:** the **first difference** (new preprints per day) or
   a per-day delta; the level itself should never be z-scored.
8. **Temporal resolution:** daily poll; cumulative level.
9. **Independent?** Partly — it overlaps Crossref (preprints deposited as works).
10. **Same event as another source?** Yes, with Crossref (§11).
11. **Lenses:** SCIENCE, AI.
12. **Never infer:** that the stored value is a *rate*; that a rise is a research
    surge (it is a cumulative count that only ever rises).

**Eligibility: DETECTABLE via a derived series.** As stored (a monotonic
cumulative level) the raw `preprint_total` is not an anomaly target, so the
catalog declares `preprint_new = Delta(preprint_total)` and the raw level is
**evidence-only** (stored, not detected on). The derived `preprint_new` series
is what detection runs on. See `docs/decisions/0014-derived-metrics.md`.

**Update (CAP-2A):** the doc/implementation mismatch noted above is resolved —
the collector still emits the raw total, and the increment is now computed by
the engine from the declared derivation rather than claimed in a comment.

## 13. noaa_kp_index — NOAA Planetary K-index

**Collector:** `crates/sources/src/noaa_kp.rs`, 3-hourly Kp product, every 3600s.
**Observation:** one 3-hourly Kp value; entity `geomagnetic_kp`,
`observed_at` = the point's `time_tag`.

1. **What one observation represents:** the planetary geomagnetic Kp index for
   one 3-hour interval.
2. **Type:** physical measurement (official space-weather index).
3. **Population stable?** Yes.
4. **Aggregate usable?** Yes, after de-duplication — the product is a rolling
   window, so the payload-hash id bug re-mints every retained point each poll
   (probe-confirmed).
5. **What an increase means:** a geomagnetic storm is developing (affects
   satellites, grids, radio).
6. **False increases:** (a) the id bug duplicating the window; (b) Kp is a
   bounded 0–9 quasi-log scale, so a σ-based z-score on it is not linear in
   physical severity; (c) the product is revised after the fact, so a re-poll may
   legitimately change a value (a revision, not a new storm).
7. **Appropriate baseline:** rolling statistics on the Kp level are reasonable;
   the Kp scale's boundedness means a percentile/EWMA view is preferable to a raw
   z-score. The 3-hourly cadence against an hourly poll means ~1/3 of polls add
   nothing new.
8. **Temporal resolution:** 3-hourly, polled hourly.
9. **Independent?** Yes.
10. **Same event as another source?** No.
11. **Lenses:** SPACE.
12. **Never infer:** that a Kp rise measured in σ maps linearly to impact; that
    an hourly poll is hourly *resolution*.

**Eligibility: DETECTABLE_WITH_CONSTRAINTS** — constraints: id de-duplication and
bounded-scale-aware baselining.

---

# 1. SOURCE SEMANTIC MATRIX

| Source | One observation is… | Type | Population stable | Aggregatable | `observed_at` | Cadence | Identity bug |
| --- | --- | --- | --- | --- | --- | --- | --- |
| usgs_earthquakes | one quake's magnitude | event (physical) | yes | after dedup | quake time | 60s / 1h feed | **yes** |
| afad_earthquakes | one AFAD event | event (physical) | yes | after dedup | local−3h | window poll | **yes** |
| nasa_neo | one day's close approaches | physical activity | yes | yes | day (UTC) | daily | yes |
| nws_alerts | active alerts per severity | gauge/snapshot | yes | yes | collection time | 600s | no |
| nasa_eonet | open events per category | gauge/snapshot | yes | yes | collection time | 1800s | no |
| gdelt_news_volume | news share per bucket | news activity | corpus churns | yes (attention) | bucket time | 900s / 1d | **yes** |
| hackernews_frontpage | one story's score | platform behavior | yes (fixed, CAP-2C) | no | collection time | 600s | no |
| github_rust_activity | one repo's stars | platform behavior | yes (constant) | yes | collection time | 3600s | no |
| cisa_kev | 7-day additions / catalog size | registry activity | yes | yes | collection time | 86400s | no |
| ecb_exchange_rates | one day's reference rate | institutional measure | yes | yes | SDMX date | 86400s | **yes** |
| crossref_works | works registered in 2d | registry activity | yes | yes | collection time | 86400s | no |
| arxiv_submissions | cumulative category total | registry activity | yes | **not as level** | collection time | 86400s | no |
| noaa_kp_index | one 3-hourly Kp value | physical measure | yes | after dedup | point time | 3600s / 3h | **yes** |

# 2. DETECTION ELIGIBILITY MATRIX

| Source | Eligibility | Condition |
| --- | --- | --- |
| nws_alerts | **DETECTABLE** | — |
| nasa_eonet | **DETECTABLE** | — |
| usgs_earthquakes | DETECTABLE_WITH_CONSTRAINTS | fix per-record id dedup |
| afad_earthquakes | DETECTABLE_WITH_CONSTRAINTS | fix id dedup; handle `isEventUpdate` |
| gdelt_news_volume | DETECTABLE_WITH_CONSTRAINTS | fix id dedup; treat as attention, not phenomenon |
| ecb_exchange_rates | DETECTABLE_WITH_CONSTRAINTS | fix id dedup; business-day gaps |
| noaa_kp_index | DETECTABLE_WITH_CONSTRAINTS | fix id dedup; bounded-scale baseline |
| cisa_kev | **DETECTABLE** | `kev_added` non-overlapping daily (CAP-2D); `kev_catalog_total` differenced (CAP-2B) |
| crossref_works | **DETECTABLE** | single completed day; no overlap, no partial latest point (CAP-2D) |
| github_rust_activity | **DETECTABLE** | per-repository series; cold start is a per-repo guard (CAP-2D) |
| nasa_neo | **DETECTABLE** | daily approach count is coherent (CAP-2D) |
| arxiv_submissions | **DETECTABLE** | detection runs on the derived `preprint_new`; raw level evidence-only (CAP-2A) |
| hackernews_frontpage | **DETECTABLE** | fixed universe committed and reused (CAP-2C) |

# 3. CROSS-SOURCE INDEPENDENCE MATRIX

`I` = independent sensors of different things. `O` = overlaps (may describe the
same real-world event; convergence, never merge).

| | usgs | afad | neo | nws | eonet | gdelt | hn | gh | kev | ecb | cross | arxiv | kp |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| usgs | — | O | I | O | O | O | I | I | I | I | I | I | I |
| afad | O | — | I | I | O | O | I | I | I | I | I | I | I |
| neo | I | I | — | I | I | I | I | I | I | I | I | I | I |
| nws | O | I | I | — | O | O | I | I | I | I | I | I | I |
| eonet | O | O | I | O | — | O | I | I | I | I | I | I | I |
| gdelt | O | O | I | O | O | — | O | I | I | I | I | I | I |
| hn | I | I | I | I | I | O | — | O | I | I | I | I | I |
| gh | I | I | I | I | I | I | O | — | I | I | I | I | I |
| kev | I | I | I | I | I | I | I | I | — | I | I | I | I |
| ecb | I | I | I | I | I | I | I | I | I | — | I | I | I |
| cross | I | I | I | I | I | I | I | I | I | I | — | O | I |
| arxiv | I | I | I | I | I | I | I | I | I | I | O | — | I |
| kp | I | I | I | I | I | I | I | I | I | I | I | I | — |

Notable: **Crossref ↔ arXiv** is the strongest same-event pair (a preprint
becomes a work), and **USGS ↔ AFAD** the strongest geophysical pair. **GDELT** is
an `O` against almost every event source by nature — it reports *on* them.

# 4. TOP 10 SEMANTIC RISKS

1. **Payload-hash observation ids re-mint unchanged records** (USGS, AFAD, ECB,
   NOAA Kp, GDELT). Silent duplication of event feeds and sliding windows;
   corrupts baselines. Probe-confirmed for all five.
2. ~~**Hacker News catalog/implementation contradiction.**~~ **RESOLVED (CAP-2C).**
   The collector now commits the universe on first resolution and reuses it, so
   the population no longer churns and the engine detects on the series the
   catalog declares.
3. **arXiv stores a cumulative level where the doc claims a velocity.** A
   monotonic total is not an anomaly target; the intended series is never
   computed.
4. ~~**NASA NEO pools all objects into `neo_class_all`.**~~ **RESOLVED (CAP-2D).**
   The series is now the UTC day's count of close approaches; the day's closest
   object is kept as drill-down attributes.
5. ~~**CISA `kev_added` is a trailing 7-day sum sampled daily.**~~ **RESOLVED
   (CAP-2D).** It is now a non-overlapping daily count.
6. **AFAD `isEventUpdate` rows are appended as new events.** A magnitude
   correction looks like new seismic activity.
7. ~~**Crossref latest-day partial deposit.**~~ **RESOLVED (CAP-2D).** The
   measured day is now a completed day (yesterday), not a window ending today.
8. **ECB business-day gaps.** Weekends/holidays are absent, not zero; a
   time-based baseline misreads the gap.
9. ~~**GitHub cold-start level deviation.**~~ **RESOLVED (CAP-2D).** Each
   repository is now its own series (the `repo` dimension), so a first
   appearance is judged against that repository's own history, not a pooled
   baseline.
10. **NWS/EONET national counts are batch-driven.** One weather system or one
    EONET ingestion can move a national count without the world changing.
11. ~~**`feeds_lenses` is declared but never enforced.**~~ **Fixed.** Lens
    matching used the signal's category/entity/keyword only, so EONET declared
    EARTH yet its signals never reached it. The engine now routes a signal to the
    lenses its sources declare, by provenance, alongside the filters.

# 5. REQUIRED FIXES

Ordered by risk. Each is documented above and needs a regression test before the
fix (per the project rule: document, then a minimal failing test, then fix).

| # | Fix | Scope | Test to add first |
| --- | --- | --- | --- |
| F1 | ~~Derive observation ids from the **record key**, not the payload hash~~ **DONE** — ids are now `series_key \| observed_at \| record_key`; the five sources pass `event_id`/`period`/`time_tag`/`series\|date` | `crates/model/src/ids.rs`, `observation.rs`, + per-source `parse` | five probes promoted to active tests in `crates/sources/tests/semantic_regression.rs` |
| F2 | ~~Make Hacker News honour `fixed_universe`: persist and reuse the universe via `next_universe`, or declare `unstable_population`~~ **DONE (CAP-2C)** — the collector commits the universe on first resolution, keeps a story that leaves the front page, and refills a slot only on a genuine 404 | `crates/sources/src/collectors.rs`, `crates/sources/src/hackernews.rs` | `crates/sources/tests/semantic_regression.rs`, `crates/sources/src/collectors.rs` tests |
| F3 | ~~Emit arXiv as a **difference** (`new preprints/day`), not the cumulative total~~ **DONE (CAP-2A)** — the catalog declares `preprint_new = Delta(preprint_total)`; the raw total is evidence-only | `crates/sources/src/arxiv.rs` | `crates/engine/tests/derived_metrics.rs` |
| F4 | ~~Give NASA NEO a coherent series (count below a distance threshold, or per-object) instead of pooled distance~~ **DONE (CAP-2D)** — one observation is one UTC day's count of close approaches; the day's closest object is a drill-down attribute | `crates/sources/src/nasa.rs` | `crates/sources/tests/semantic_regression.rs`, `crates/sources/src/nasa.rs` tests |
| F5 | ~~Baseline `kev_added` on a weekly cadence / overlapping-window-aware baseline~~ **DONE (CAP-2D)** — `kev_added` is now the count dated to the collection day, a non-overlapping daily series; no `WindowCount` kind was needed | `crates/sources/src/cisa_kev.rs` | `crates/sources/tests/semantic_regression.rs`, `crates/sources/src/cisa_kev.rs` tests |
| F5a | ~~Difference `kev_catalog_total` instead of detecting the level~~ **DONE (CAP-2B)** — the catalogue declares `kev_catalog_growth = Delta(kev_catalog_total)`; the raw total is evidence-only | `crates/sources/src/cisa_kev.rs` | `crates/cli/tests/cisa_derived_metric.rs` |
| F6 | Supersede AFAD events with `isEventUpdate=true` rather than appending | `crates/sources/src/afad.rs` | update replaces, not adds |
| F7 | ~~Mark the Crossref latest point as partial (quality flag) or shift the window back a day~~ **DONE (CAP-2D)** — the collector measures one completed day (yesterday), so consecutive polls never overlap and the latest point is fully deposited | `crates/sources/src/crossref.rs` | `crates/sources/tests/semantic_regression.rs`, `crates/sources/src/crossref.rs` tests |
| F8 | Handle business-day gaps for ECB (absence ≠ zero) | `crates/sources/src/ecb.rs` / baseline | gap-aware baseline test |
| F9 | ~~Cold-start guard: no level deviation before a per-repo baseline exists~~ **DONE (CAP-2D)** — each repository is its own series via the `repo` dimension, so a first appearance is judged against that repository's own history, not a pooled baseline | `crates/sources/src/github.rs` | `crates/sources/tests/semantic_regression.rs`, `crates/sources/src/github.rs` tests |
| F10 | Add a domain lens (or explicit membership) for NWS/EONET so Tier-1 real-time sources reach a domain view | `config/lenses/*` | lens-coverage test update |
| F11 | ~~Make `feeds_lenses` enforced~~ **DONE** — the engine routes a signal to the lenses its sources declare, by provenance, alongside the lens filters; a declared lens is now one the source's signals actually reach | `crates/signals`, `crates/engine` | `a_declared_lens_is_reachable_at_runtime` in `lens_coverage.rs`, plus `lens_routing.rs` |

F1 is the only fix that touches the shared id contract; it should land first and
alone, because every other source's regression suite depends on stable ids.

# 6. SOURCES READY FOR REAL SIGNAL DETECTION

Meaningful today, no fix required:

- **nws_alerts** — a true gauge with a per-severity dimension.
- **nasa_eonet** — a true gauge with a per-category dimension.

Meaningful after the named constraint is met (all hinge on F1 first):

- usgs_earthquakes, afad_earthquakes, ecb_exchange_rates, noaa_kp_index,
  gdelt_news_volume (F1).

# 7. SOURCES THAT SHOULD ONLY PROVIDE EVIDENCE

None, as of CAP-2D. Every source that was originally in this category has been
given a coherent series and is now detectable:

- **hackernews_frontpage** — no longer in this category (CAP-2C): the fixed
  universe is committed and reused, so each tracked story is a real, detectable
  series. The summed score is still not meaningful — use the per-story series.
- **arxiv_submissions** — no longer evidence-only (CAP-2A): detection runs on
  the derived `preprint_new` series; the raw cumulative level is stored for
  evidence only.
- **nasa_neo** — no longer evidence-only (CAP-2D): the series is the UTC day's
  count of close approaches, a coherent rate. The day's closest object stays as
  a drill-down attribute.

---

## Method and limits

- Every source was read from its collector module and fixture; the id bug was
  confirmed by executing probes, not by inspection alone.
- The probes are committed as regression specs in
  `crates/sources/tests/semantic_regression.rs`. The F1 probes (identity) and the
  F2 probe (fixed universe) are now active and green; each fix turned one green,
  and each fails again if its contract is reverted.
- No source was added, no lens was added, no UI was changed, and Phase 13 was
  not started.
