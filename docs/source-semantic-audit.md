# Source semantic audit

A per-source audit of the 21 connected sources. It answers, for each source,
what one observation *means* — and, more importantly, what the resulting time
series does **not** mean even though the pipeline will happily compute on it.

This is not a restatement of [SOURCE_CATALOG.md](../SOURCE_CATALOG.md). Every
claim below was reached by reading the collector implementation and its fixture
and tracing the value through:

```text
SOURCE -> COLLECTOR -> NORMALIZATION -> OBSERVATION -> STORAGE -> BASELINE -> DETECTION
```

Audit run: 2026-09-30, `main` @ `399c880`, Rust 1.88.0.
Last reconciled: 2026-10-01 — every enumerated fix (F1–F11) is **DONE**, and the
eight sources added in the network-widening pass (14–21) are audited below, so
all 21 sources are now DETECTABLE.

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

**Eligibility: DETECTABLE** — per-record id de-duplication is handled by the
upstream `id` record key (F1).

## 2. afad_earthquakes — AFAD Turkey Earthquake Catalogue

**Collector:** `crates/sources/src/afad.rs`, a filter window per poll.
**Observation:** one earthquake, entity `province_<province>`,
`metric=earthquake_magnitude`, `observed_at` = local time − 3h (source).

1. **What one observation represents:** a single AFAD catalogue event.
2. **Type:** event (geophysical), regional.
3. **Population stable?** Yes — events are historical facts.
4. **Aggregate usable?** Yes — the id is derived from the upstream `eventID`
   record key (F1), so a window-retained or revised event keeps one identity.
5. **What an increase means:** more recorded seismicity in Turkish provinces.
6. **False increases:** (a) ~~the id bug~~ **fixed (F1)**; (b) AFAD's window
   overlapping between polls — de-duplicated by the `eventID` record key
   (F1); (c) ~~`isEventUpdate=true` rows appended as new events~~ **fixed
   (F1)** — a revision keeps the event's identity, so a correction is not read
   as new activity.
7. **Appropriate baseline:** per-province rolling statistics; province is a
   stable administrative key, which is better than USGS's free-text region.
8. **Temporal resolution:** event time (UTC+3 → UTC), window-based polling.
9. **Independent?** Yes, and complementary to USGS. Institutional (AFAD) vs
   international (USGS) networks — a good convergence pair for TURKEY.
10. **Same real-world event as another source?** Yes, with USGS (Turkey quakes).
11. **Lenses:** TURKEY, EARTH.
12. **Never infer:** that a province's rise is a national rise (provinces are
    independent series); that an `isEventUpdate` row is a new earthquake.

**Eligibility: DETECTABLE** — per-record id de-duplication and event-update
semantics are handled by the `eventID` record key (F1).

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
11. **Lenses:** EARTH — reached by provenance, not by category: NWS declares
    `feeds_lenses: [lens_earth]` and the engine routes a signal to the lenses its
    sources declare (F11), so the Tier-1 real-time feed reaches a domain view.
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

**Eligibility: DETECTABLE** — id de-duplication is handled by the
`series|date` record key (F1). Treat it as an *attention* series, never as the
phenomenon.

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

**Eligibility: DETECTABLE** — `kev_catalog_total` is differenced, not detected on
as a level (CAP-2B); `kev_added` is a non-overlapping daily count (CAP-2D).

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
4. **Aggregate usable?** Yes — the id contract is fixed (F1), so the re-fetched
   window de-duplicates to one point per business day.
5. **What an increase means:** the euro strengthened against the dollar that day.
6. **False increases:** (a) ~~the id bug re-inserting the window~~ **fixed
   (F1)**; (b) the ECB publishes on TARGET business days only —
   weekends/holidays are **absent**, not zero; detection is count-based, so a
   gap does not distort a deviation, and a missing day inserts no observation
   at all (**verified, CAP-2D/0021**); (c) the rate is a *fixing*, not a live
   market price.
7. **Appropriate baseline:** rolling statistics over the daily rate. Detection
   is count-based over the window, so the gap needs no special handling; a
   future time-based rate would need it explicitly. For finance, a
   relative-change baseline is more natural than a level z-score.
8. **Temporal resolution:** daily, business days only.
9. **Independent?** Yes — the only market source present.
10. **Same event as another source?** No.
11. **Lenses:** FINANCE.
12. **Never infer:** that a missing day is a flat day; that this single USD/EUR
    pair is "the markets"; that a reference-rate move equals a tradable move.

**Eligibility: DETECTABLE** — the id contract is fixed (F1) and business-day
gaps are handled (CAP-2D/0021). See
`docs/decisions/0021-business-day-gaps.md`.

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

**Eligibility: DETECTABLE** — id de-duplication is handled by the `time_tag`
record key (F1). The bounded Kp scale is a baseline caveat, not an eligibility
blocker.

---

## 14. open_meteo_weather — Open-Meteo Weather

**Collector:** `crates/sources/src/open_meteo.rs`, `current` block for a fixed
city set, every 1800s.
**Observation:** one city's `temperature_2m` (and `precipitation`); entity
`weather`, dimension `city`, `observed_at` = the product's `current.time`.

1. **What one observation represents:** the model's current surface temperature
   (or precipitation) at one named city.
2. **Type:** physical measurement (model analysis/nowcast).
3. **Population stable?** Yes — the city set is fixed in code.
4. **Aggregate usable?** Yes, after de-duplication: each city is its own series
   (`city` dimension), so a city is compared against its own history, never a
   pool of unrelated climates.
5. **What an increase means:** a city warmer (or wetter) than its own recent
   norm.
6. **False increases:** (a) `current` is a model field, so a model upgrade can
   shift the level of every city at once — a step, not weather; (b) a synoptic
   system legitimately moves many cities at once, which is convergence, not a
   bug.
7. **Appropriate baseline:** rolling statistics per city. Temperature is
   seasonal, so a long window will eventually need a seasonal adjustment; the
   MVP window is short enough that this is a caveat, not a blocker.
8. **Temporal resolution:** the product's current timestep (~15 min), polled
   30-minutely.
9. **Independent?** Yes — independent of the NWS bulletin feed (this is a
   physical measurement, that is an official warning).
10. **Same event as another source?** Shares a *cause* with NWS alerts (weather),
    but is not the same observation.
11. **Lenses:** EARTH, AGRICULTURE, TURKEY (by bbox).
12. **Never infer:** that a temperature anomaly is a forecast, or that a city
    anomaly is a regional one.

**Eligibility: DETECTABLE** — per-city series, fixed universe, real record key.

## 15. open_meteo_air_quality — Open-Meteo Air Quality

**Collector:** `crates/sources/src/open_meteo.rs`, `current` block (CAMS) for the
same city set, every 3600s.
**Observation:** one city's `pm2_5` (and `pm10`); entity `air_quality`,
dimension `city`.

1. **What one observation represents:** current fine/coarse particulate
   concentration at one city.
2. **Type:** physical measurement (CAMS reanalysis/forecast field).
3. **Population stable?** Yes.
4. **Aggregate usable?** Yes, per city.
5. **What an increase means:** worse air quality than that city's recent norm
   (dust, wildfire smoke, stagnation).
6. **False increases:** CAMS is a model, so a model revision moves levels;
   particulates are strongly diurnal and weather-driven.
7. **Appropriate baseline:** rolling statistics per city; the same seasonal
   caveat as temperature.
8. **Temporal resolution:** hourly, polled hourly.
9. **Independent?** Yes — a physical environment measurement.
10. **Same event as another source?** A wildfire can move PM2.5 *and* appear in
    EONET; that is convergence across an observation and an event list.
11. **Lenses:** EARTH, AGRICULTURE.
12. **Never infer:** that a PM2.5 rise has one cause.

**Eligibility: DETECTABLE** — per-city series, fixed universe.

## 16. noaa_goes_xray — NOAA GOES X-ray Flux

**Collector:** `crates/sources/src/noaa_goes.rs`, GOES primary 1-minute X-ray
flux, every 900s.
**Observation:** one 1-minute `xray_flux` (0.1–0.8 nm); entity `space_weather`,
`observed_at` = the point's `time_tag`.

1. **What one observation represents:** the solar X-ray flux in one minute.
2. **Type:** physical measurement (instrument count).
3. **Population stable?** Yes.
4. **Aggregate usable?** Yes, after de-duplication by `time_tag` — the product
   is a rolling 1-day window, so the record key is essential.
5. **What an increase means:** a solar flare.
6. **False increases:** the flux spans ~7 orders of magnitude and the quiet
   baseline is ~1e-7, so a plain z-score is dominated by the flare tail; the
   value is also stored at full precision (the reason text no longer prints it
   as `0.00`).
7. **Appropriate baseline:** robust statistics (MAD) on the log-ish flux; a
   flare is a *level* change of orders of magnitude, not a small σ move.
8. **Temporal resolution:** 1 minute, polled 15-minutely.
9. **Independent?** Yes, and independent of `noaa_kp_index`: Kp measures
   disturbance at Earth, X-ray measures the solar driver.
10. **Same event as another source?** A flare (X-ray) often *precedes* a storm
    (Kp) — a genuine cross-source convergence with a lead time.
11. **Lenses:** SPACE.
12. **Never infer:** that a flare magnitude is a Kp magnitude, or that every
    flare reaches Earth.

**Eligibility: DETECTABLE** — 1-minute record key; bounded-scale baseline
caveat, as with Kp.

## 17. gdacs_disasters — GDACS Disaster Alerts

**Collector:** `crates/sources/src/gdacs.rs`, multi-hazard event list over a
rolling 30-day window, every 1800s.
**Observation:** active alerts per hazard × alert level; entity `disasters`,
dimension `hazard`/`level`, `observed_at` = the collection time.

1. **What one observation represents:** how many alerts of a given hazard at a
   given alert level are currently open.
2. **Type:** official impact assessment (a judgement, not a raw measurement).
3. **Population stable?** Yes — the hazard × level grid is fixed.
4. **Aggregate usable?** Yes, as a gauge; the count falls as well as rises as
   events close.
5. **What an increase means:** more/severer official alerts open than usual.
6. **False increases:** a batch ingestion can move a count; GDACS itself is an
   assessment layer, so a level change can be a re-assessment, not a new event.
7. **Appropriate baseline:** rolling statistics on the gauge; the fixed grid
   avoids pooling across hazards with different base rates.
8. **Temporal resolution:** event-driven, polled 30-minutely.
9. **Independent?** Yes, and independent of EONET/USGS: EONET observes events,
   GDACS *assesses impact*.
10. **Same event as another source?** Frequently — a quake is in USGS, EONET and
    GDACS. That is the designed convergence case, never a merge.
11. **Lenses:** EARTH, HUMANITARIAN.
12. **Never infer:** that an alert count is a casualty count.

**Eligibility: DETECTABLE** — fixed hazard × level grid, collection-time gauge.

## 18. who_outbreaks — WHO Disease Outbreak News

**Collector:** `crates/sources/src/who_outbreaks.rs`, newest-first feed, every
86400s.
**Observation:** the count of outbreak announcements published in the last 24h;
entity `health`, `observed_at` = the collection day.

1. **What one observation represents:** how many outbreak announcements WHO
   published in the last 24 hours.
2. **Type:** institutional publication activity.
3. **Population stable?** Yes.
4. **Aggregate usable?** Yes — a non-overlapping daily count.
5. **What an increase means:** more outbreak announcements than usual.
6. **False increases:** an announcement is not an outbreak *starting* — it is
   WHO *saying* so, and the publication process can batch.
7. **Appropriate baseline:** a daily count series; most days zero, so the
   distribution is sparse and a robust baseline matters.
8. **Temporal resolution:** daily, polled daily.
9. **Independent?** Yes — a new domain.
10. **Same event as another source?** Rarely; the health domain is otherwise
    empty.
11. **Lenses:** HEALTH.
12. **Never infer:** that zero announcements mean zero outbreaks.

**Eligibility: DETECTABLE** — non-overlapping daily count (same discipline as
`kev_added`).

## 19. coingecko_market — CoinGecko Crypto Prices

**Collector:** `crates/sources/src/coingecko.rs`, simple-price for a fixed coin
set, every 900s.
**Observation:** one coin's spot price; entity `crypto`, dimension `coin`.

1. **What one observation represents:** a coin's spot price in USD.
2. **Type:** market price.
3. **Population stable?** Yes — the coin set is fixed in code.
4. **Aggregate usable?** Yes, per coin; never pool coins of different price
   levels.
5. **What an increase means:** the price moved more than the coin's recent norm.
6. **False increases:** markets are volatile by nature, so a σ threshold must
   be tuned; the price is a level, so a *relative* move matters more than an
   absolute one.
7. **Appropriate baseline:** rolling statistics per coin; consider log returns
   for a future detector.
8. **Temporal resolution:** spot, polled 15-minutely.
9. **Independent?** Yes, and independent of the ECB reference rate (different
   market, different mechanism).
10. **Same event as another source?** A macro shock can move crypto and FX
    together — convergence.
11. **Lenses:** FINANCE, MARKETS.
12. **Never infer:** that a price move has a single cause, or that it is
    permanent.

**Eligibility: DETECTABLE** — per-coin series, fixed universe.

## 20. npm_downloads — npm Package Downloads

**Collector:** `crates/sources/src/npm.rs`, registry download point per package,
every 86400s.
**Observation:** one package's weekly downloads; entity `software`, dimension
`package`.

1. **What one observation represents:** a package's downloads over the last
   week.
2. **Type:** platform usage measurement.
3. **Population stable?** Yes — the package set is fixed in code.
4. **Aggregate usable?** Yes, per package; never pool.
5. **What an increase means:** a package being adopted faster than its own norm.
6. **False increases:** a CI/release can spike downloads mechanically; the
   weekly total is a trailing window, so consecutive polls overlap.
7. **Appropriate baseline:** per-package rolling statistics; the overlapping
   weekly window is a caveat — the *change* between polls is meaningful, the
   level is a smoothed trailing figure.
8. **Temporal resolution:** daily, polled daily.
9. **Independent?** Yes, and independent of GitHub stars (attention) and HN
   (discussion) — this is *usage*.
10. **Same event as another source?** A release can move downloads *and* stars
    *and* HN discussion — a three-way convergence.
11. **Lenses:** SOFTWARE.
12. **Never infer:** that downloads are users.


**Eligibility: DETECTABLE** — per-package series, fixed universe; trailing-window
caveat noted.

## 21. pypi_downloads — PyPI Package Downloads

**Collector:** `crates/sources/src/pypi.rs`, pypistats recent downloads per
package, every 86400s.
**Observation:** one package's `last_week` downloads; entity `software`,
dimension `package`; `last_day` is a drill-down attribute.

Same semantics and caveats as `npm_downloads` (20), for the Python ecosystem.
Independent of npm: a different community and package set, so a rise in both is
convergence, not one sensor counted twice.


**Eligibility: DETECTABLE** — per-package series, fixed universe.

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
| open_meteo_weather | one city's temperature/rain | physical measure | yes (fixed cities) | per city | product time | 1800s | no |
| open_meteo_air_quality | one city's PM2.5/PM10 | physical measure | yes (fixed cities) | per city | product time | 3600s | no |
| noaa_goes_xray | one minute's X-ray flux | physical measure | yes | after dedup | point time | 900s / 1m | **yes** |
| gdacs_disasters | open alerts per hazard×level | impact assessment | yes | yes | collection time | 1800s | no |
| who_outbreaks | announcements in last 24h | publication activity | yes | yes | collection day | 86400s | no |
| coingecko_market | one coin's spot price | market price | yes (fixed coins) | per coin | collection time | 900s | no |
| npm_downloads | one package's weekly downloads | usage | yes (fixed set) | per package | collection time | 86400s | no |
| pypi_downloads | one package's weekly downloads | usage | yes (fixed set) | per package | collection time | 86400s | no |

# 2. DETECTION ELIGIBILITY MATRIX

| Source | Eligibility | Condition |
| --- | --- | --- |
| nws_alerts | **DETECTABLE** | — |
| nasa_eonet | **DETECTABLE** | — |
| usgs_earthquakes | **DETECTABLE** | id contract fixed (F1) |
| afad_earthquakes | **DETECTABLE** | id contract fixed (F1); `isEventUpdate` revisions de-duplicated by `eventID` |
| gdelt_news_volume | **DETECTABLE** | id contract fixed (F1); treat as attention, not phenomenon |
| ecb_exchange_rates | **DETECTABLE** | id contract fixed (F1); count-based detection handles business-day gaps (CAP-2D/0021) |
| noaa_kp_index | **DETECTABLE** | id contract fixed (F1); bounded-scale baseline caveat |
| cisa_kev | **DETECTABLE** | `kev_added` non-overlapping daily (CAP-2D); `kev_catalog_total` differenced (CAP-2B) |
| crossref_works | **DETECTABLE** | single completed day; no overlap, no partial latest point (CAP-2D) |
| github_rust_activity | **DETECTABLE** | per-repository series; cold start is a per-repo guard (CAP-2D) |
| nasa_neo | **DETECTABLE** | daily approach count is coherent (CAP-2D) |
| arxiv_submissions | **DETECTABLE** | detection runs on the derived `preprint_new`; raw level evidence-only (CAP-2A) |
| hackernews_frontpage | **DETECTABLE** | fixed universe committed and reused (CAP-2C) |
| open_meteo_weather | **DETECTABLE** | fixed city set; per-city series; model-revision caveat |
| open_meteo_air_quality | **DETECTABLE** | fixed city set; per-city series |
| noaa_goes_xray | **DETECTABLE** | 1-minute record key; bounded-scale baseline caveat |
| gdacs_disasters | **DETECTABLE** | fixed hazard×level grid; impact-assessment caveat |
| who_outbreaks | **DETECTABLE** | non-overlapping daily count |
| coingecko_market | **DETECTABLE** | fixed coin set; per-coin series |
| npm_downloads | **DETECTABLE** | fixed package set; trailing-window caveat |
| pypi_downloads | **DETECTABLE** | fixed package set; trailing-window caveat |

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

1. ~~**Payload-hash observation ids re-mint unchanged records** (USGS, AFAD, ECB,
   NOAA Kp, GDELT).~~ **RESOLVED (F1).** Each of the five now derives its id from
   a stable upstream record key (`event id`, `time_tag`, `series|date`, SDMX
   date), so an unchanged record re-collected in a sliding window keeps one id.
2. ~~**Hacker News catalog/implementation contradiction.**~~ **RESOLVED (CAP-2C).**
   The collector now commits the universe on first resolution and reuses it, so
   the population no longer churns and the engine detects on the series the
   catalog declares.
3. ~~**arXiv stores a cumulative level where the doc claims a velocity.**~~
   **RESOLVED (CAP-2A).** The catalog declares `preprint_new = Delta(preprint_total)`;
   detection runs on the derived increment and the raw cumulative level is
   evidence-only.
4. ~~**NASA NEO pools all objects into `neo_class_all`.**~~ **RESOLVED (CAP-2D).**
   The series is now the UTC day's count of close approaches; the day's closest
   object is kept as drill-down attributes.
5. ~~**CISA `kev_added` is a trailing 7-day sum sampled daily.**~~ **RESOLVED
   (CAP-2D).** It is now a non-overlapping daily count.
6. ~~**AFAD `isEventUpdate` rows are appended as new events.**~~ **RESOLVED
   (F1).** The record key is the upstream `eventID`, so a revised magnitude
   keeps the same identity and is de-duplicated instead of appended; the event
   id survives as drill-down. Guarded by `afad_an_event_update_keeps_identity`.
7. ~~**Crossref latest-day partial deposit.**~~ **RESOLVED (CAP-2D).** The
   measured day is now a completed day (yesterday), not a window ending today.
8. ~~**ECB business-day gaps.**~~ **RESOLVED (CAP-2D/0021).** Detection is
   count-based over the window, not a time-elapsed rate; a missing day inserts
   no observation, so absence never becomes zero. Guarded by
   `crates/engine/tests/business_day_gaps.rs`.
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
| F6 | ~~Supersede AFAD events with `isEventUpdate=true` rather than appending~~ **DONE (F1)** — the `eventID` record key gives a revised event the same identity, so it is de-duplicated, not appended; the event id survives as drill-down | `crates/sources/src/afad.rs` | `afad_an_event_update_keeps_identity` |
| F7 | ~~Mark the Crossref latest point as partial (quality flag) or shift the window back a day~~ **DONE (CAP-2D)** — the collector measures one completed day (yesterday), so consecutive polls never overlap and the latest point is fully deposited | `crates/sources/src/crossref.rs` | `crates/sources/tests/semantic_regression.rs`, `crates/sources/src/crossref.rs` tests |
| F8 | ~~Handle business-day gaps for ECB (absence ≠ zero)~~ **DONE (CAP-2D/0021)** — detection is count-based, a missing day inserts no observation, and a normal move across a weekend gap yields no anomaly while a real move is still caught | `crates/engine/tests/business_day_gaps.rs` | `a_normal_move_across_a_weekend_gap_is_not_anomalous`, `a_genuine_move_across_a_gap_is_still_caught`, `a_missing_business_day_is_absent_not_zero` |
| F9 | ~~Cold-start guard: no level deviation before a per-repo baseline exists~~ **DONE (CAP-2D)** — each repository is its own series via the `repo` dimension, so a first appearance is judged against that repository's own history, not a pooled baseline | `crates/sources/src/github.rs` | `crates/sources/tests/semantic_regression.rs`, `crates/sources/src/github.rs` tests |
| F10 | ~~Add a domain lens (or explicit membership) for NWS/EONET so Tier-1 real-time sources reach a domain view~~ **DONE (F11)** — NWS and EONET both declare `lens_earth`, and the engine routes a signal to the lenses its sources declare, so both reach EARTH by provenance; covered by the generic runtime check | `config/lenses/earth.yaml` | `a_declared_lens_is_reachable_at_runtime` in `lens_coverage.rs` |
| F11 | ~~Make `feeds_lenses` enforced~~ **DONE** — the engine routes a signal to the lenses its sources declare, by provenance, alongside the lens filters; a declared lens is now one the source's signals actually reach | `crates/signals`, `crates/engine` | `a_declared_lens_is_reachable_at_runtime` in `lens_coverage.rs`, plus `lens_routing.rs` |

F1 was the only fix that touched the shared id contract and it landed first and
alone, because every other source's regression suite depends on stable ids.

# 6. SOURCES READY FOR REAL SIGNAL DETECTION

Every source is now detectable. The shared id contract (F1) that the sources
below depended on has landed, so the constraint that held them back is met:

- **nws_alerts** — a true gauge with a per-severity dimension.
- **nasa_eonet** — a true gauge with a per-category dimension.
- **usgs_earthquakes, afad_earthquakes, ecb_exchange_rates, noaa_kp_index,
  gdelt_news_volume** — per-record id de-duplication (F1); ECB business-day gaps
  are handled (CAP-2D/0021).

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
  `crates/sources/tests/semantic_regression.rs`. All are active and green —
  F1 (identity), F2 (fixed universe), F4 (coherent daily count), F5
  (non-overlapping daily count), F7 (single completed day) and F9
  (per-repository series) — and each fails again if its contract is reverted.
- No source was added, no lens was added, no UI was changed, and Phase 13 was
  not started.
