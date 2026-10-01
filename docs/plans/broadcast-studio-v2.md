# KOZMİK SİNYAL MERKEZİ — Broadcast Studio yeniden inşası (v2)

Durum: **öneri — onay bekliyor. Kod yazılmadı.**
Tarih: 2026-10-01
Kapsam: `web/` katmanının tamamen yeniden inşası. **Backend'e dokunulmaz.**
İlgili: `docs/decisions/0026-observatory-ui.md`, `docs/plans/observatory-v2.md`,
`docs/design/reference/gemini-kozmik-yayin-studyosu.html`

Bu belge tek çıktıdır ve şu 14 başlığı içerir:

1. mevcut mimarinin teşhisi · 2. yeni mimari · 3. dosya yapısı · 4. sahneler ·
5. bloklar · 6. event director · 7. breaking · 8. control room · 9. layout ·
10. broadcast responsive · 11. migration · 12. test · 13. riskler ·
14. değişen kararlar

---

# 1. MEVCUT MİMARİNİN TEŞHİSİ

## 1.1 Ölçülen gerçek

| Öğe | Durum |
|---|---|
| `web/app.js` | **2393 satır, tek dosya**, ~90 fonksiyon, global mutable state |
| `web/styles.css` | 864 satır, tek tema, gözlemevi + sayfalar aynı dosyada |
| `web/index.html` | 67 satır: topbar + nav + controls + footer = **web sitesi iskeleti** |
| Sayfa modeli | `#/world`, `#/signal`, `#/event`, `#/observation`, `#/source`, `#/sources`, `#/lenses`, `#/map`, `#/system`, `#/timeline` |
| Gözlemevi | `body.obs-active` + `?broadcast=1` ile **ikinci bir mod**; grid `obs-*` |
| Canlı veri | `startActivityStream` (SSE) + `startWorldStream` (fetch poll) |
| Sunum | API `ServeDir` fallback, **build adımı yok**, yerel dosyalar |

## 1.2 Teşhis — kök neden

Sorun kozmetik değil, **mimari**. Dört yapısal kusur:

**Kusur 1 — Her şey tek dosyada ve birbirine bağlı.**
`worldView()` içinden `statChip()`, `sparkline()`, `paintWorldNotice()`,
`updateWorldHeader()` çağrılıyor. Bir bölgeyi çıkarmak istediğinizde kodu kesmeniz
gerekir. "Bölge ekle/çıkar" isteği bu yapıda **imkânsız**. Düzeltme denemesi
(CSS yeniden yazımı) bu yüzden tutmadı: sorun CSS'te değil, bağımlılık grafiğinde.

**Kusur 2 — İki mod var, biri diğerinin kopyası.**
Gözlemevi, mevcut sayfaların yanına eklenmiş ikinci bir render yolu. Aynı veriyi
iki farklı kod çiziyor (`obsCategories` ve `worldStatChips` aynı şeyi yapıyor).
Bu ikilik her değişikliği iki katına çıkarıyor.

**Kusur 3 — Ekran "sahne" değil "sayfa" düşünüyor.**
`route()` hash'e bakıp bir `*View()` çağırıyor ve `#view` içeriğini baştan
yazıyor. Yani ekran, veri değiştiğinde değil **kullanıcı tıkladığında** değişiyor.
Yayın hissi buradan gelmiyor: sistem kendi kendine sahne değiştiremiyor.

**Kusur 4 — Web sitesi kromu.**
Topbar, 6 nav sekmesi, footer, sayfa içi kaydırma. Ekranın ~%12'si gezinti için
harcanıyor ve sayfa kayıyor. Yayın ekranında gezinti çubuğu olmaz.

## 1.3 Kullanıcının şikâyeti ↔ teknik karşılığı

| Kullanıcı dedi | Teknik kök neden |
|---|---|
| "her şey iç içe geçmiş" | Tek dosya, global state, fonksiyonlar arası doğrudan çağrı |
| "bölgeler istenildiği zaman eklenebilmeli/çıkarılabilmeli" | Bölge kavramı yok; layout CSS'e gömülü |
| "istediğimiz zaman istediğimiz layout" | Layout kod, veri değil |
| "hiçbir şey sıkıştırılmayacak" | Grid `minmax` ile sıkıştırıyor; rotasyon kavramı yok |
| "flash öyle değil, breaking news gibi" | Flash bir modal; ekranı devralmıyor |
| "tv ekranı gibi, başka ekrana geçsin" | Otomatik sahne geçişi yok |
| "tüm menüleri değiştir" | Nav sekmeleri web sitesi kalıntısı |
| "hepsi görselleşecek" | Kaynaklar/sistem tablo; pipeline görselleştirmesi yok |

## 1.4 Kullanıcının hazırladığı `SAHNE × BÖLGE × BLOK` modeli

Model **doğru** ve korunuyor. Ancak tek başına yetersiz: o model *statik* bir
kompozisyon motoru tarif ediyor. Kullanıcının asıl istediği **yayını yöneten**
sistem. Bu yüzden v2 iki katman ekliyor:

```text
BROADCAST DIRECTOR          ← YENİ: yayını yönetir (ne, ne zaman, ne kadar)
      ↓
    SCENE                   ← korunuyor
      ↓
 REGION → BLOCK             ← korunuyor

SSE → EVENT BUS → EVENT DIRECTOR   ← YENİ: update / transition / ticker / takeover
```

**Kritik fark:** v1 planı "JSON ile layout değiştiren dashboard" tarif ediyordu.
Kullanıcı haklı olarak bunun yetmediğini söyledi. v2'de farkı yaratan şey
**director katmanıdır**.

---

# 2. ÖNERİLEN YENİ MİMARİ

## 2.1 Katmanlar

```text
        ┌─────────────────────────────────────────┐
        │  DATA ENGINE (Rust, mevcut — dokunulmaz) │
        └────────────────────┬────────────────────┘
                             │ REST + SSE
        ┌────────────────────▼────────────────────┐
        │  DATA BUS        js/data/               │  tek giriş noktası
        │  api · sse · store · adapters           │  bloklar fetch YAPMAZ
        └────────────────────┬────────────────────┘
                             │ normalize edilmiş state + events
        ┌────────────────────▼────────────────────┐
        │  EVENT BUS       js/studio/event-bus.js │  pub/sub
        └────────────────────┬────────────────────┘
                             │
        ┌────────────────────▼────────────────────┐
        │  EVENT DIRECTOR  js/studio/director.js  │  politika motoru
        │  ├─ update     → bölgeye veri it        │
        │  ├─ transition → sahne değiştir         │
        │  ├─ ticker     → son dakika besle       │
        │  └─ takeover   → breaking başlat        │
        └────────────────────┬────────────────────┘
                             │
        ┌────────────────────▼────────────────────┐
        │  SCENE MANAGER   js/studio/scene.js     │  sahne yaşam döngüsü
        │  REGION MANAGER  js/studio/region.js    │  bölge yerleşimi
        │  TRANSITION MGR  js/studio/transition.js│  geçiş animasyonu
        └────────────────────┬────────────────────┘
                             │ "şu bölgeye şu bloğu kur"
        ┌────────────────────▼────────────────────┐
        │  BLOCK REGISTRY  js/blocks/registry.js  │  mount/update/resize/unmount
        └────────────────────┬────────────────────┘
                             │
                     BROADCAST SCREEN
```

## 2.2 Değişmez kurallar (mimari sözleşme)

1. **Studio core hiçbir blok tipini tanımaz.** `"globe"` diye bir şey bilmez;
   sadece kayıtlı bir blok adı olduğunu bilir. Yeni blok = yeni dosya + bir kayıt.
2. **Bloklar fetch yapmaz.** Veriyi `ctx.data` ile alır, olayı `ctx.on()` ile
   dinler. Ağ erişimi yalnız `data/` katmanında.
3. **JSON = WHAT + WHERE. JS = HOW.** Sahne dosyası bölgenin nerede olduğunu ve
   hangi bloğu kullandığını söyler; nasıl çizileceğini asla.
4. **Sayfa asla kaymaz.** `overflow: hidden`. Taşan içerik bölge içinde döner
   veya bölge içinde kayar. Gizleme yok.
5. **Uydurma veri yok.** Alan boşsa `—` + gerekçe. Özellikle koordinat.
6. **Her blok tam yaşam döngüsüne sahip.** `mount/update/resize/unmount` +
   temizlik. `setInterval`/`rAF`/SSE/`ResizeObserver` unmount'ta kapatılır.
7. **Otomatik yayın varsayılan KAPALI.** Director açıkken yayın yapar.
8. **Renk tek başına anlam taşımaz.** Renk + ikon + şekil + etiket + hareket.

---

# 3. DOSYA YAPISI

```text
web/
├── index.html                    yayın kabuğu (topbar YOK, footer YOK)
├── styles/
│   ├── tokens.css                renk/ölçek/hareket değişkenleri
│   ├── base.css                  reset + tipografi + odak
│   ├── broadcast.css             ekran ızgarası, güvenli alan, krom
│   ├── studio.css                sahne/bölge/geçiş
│   ├── blocks.css                blok iç stilleri
│   ├── transitions.css           geçiş animasyonları
│   └── broadcast-responsive.css  aspect/4K/vertical uyarlaması
│
├── js/
│   ├── main.js                   giriş noktası, boot
│   ├── studio/
│   │   ├── studio.js             orkestratör
│   │   ├── scene.js              sahne yaşam döngüsü
│   │   ├── region.js             bölge yerleşimi + taşma yönetimi
│   │   ├── transition.js         geçiş animasyonları
│   │   ├── director.js           EVENT DIRECTOR (politika)
│   │   ├── event-bus.js          pub/sub
│   │   ├── layout.js             JSON → ızgara; preset; kaydet/yükle
│   │   └── priority.js           sinyal önceliği (INFO→CRITICAL)
│   ├── blocks/
│   │   ├── registry.js           registerBlock + yaşam döngüsü sarmalayıcı
│   │   ├── globe.js              ortho projeksiyon (mevcut koddan taşınır)
│   │   ├── world-map.js
│   │   ├── signal-feed.js
│   │   ├── signal-card.js
│   │   ├── metric.js
│   │   ├── metric-group.js
│   │   ├── category-strip.js
│   │   ├── activity-chart.js
│   │   ├── timeline.js
│   │   ├── evidence-chain.js
│   │   ├── source-health.js
│   │   ├── source-card.js
│   │   ├── system-pipeline.js
│   │   ├── raw-viewer.js
│   │   ├── telemetry.js
│   │   ├── status-grid.js
│   │   ├── ticker.js
│   │   └── breaking.js
│   ├── data/
│   │   ├── api.js                REST istemcisi (mevcut api() taşınır)
│   │   ├── sse.js                /events akışı (mevcut readStream/parseSseFrame)
│   │   ├── store.js              normalize edilmiş durum + abonelik
│   │   └── adapters.js           ham API → blok modeli dönüşümü
│   ├── controls/
│   │   ├── control-room.js       sağ panel kabuğu (K / Esc)
│   │   ├── scene-controls.js
│   │   ├── layout-controls.js
│   │   ├── data-controls.js
│   │   └── system-controls.js
│   ├── scenes/
│   │   └── load.js               config'ten sahne yükleyici
│   ├── sim/
│   │   └── simulation.js         DEV-only olay simülatörü
│   ├── fmt.js                    biçimleyiciler (mevcut koddan taşınır)
│   ├── i18n.js                   TR/EN (mevcut tablo genişletilir)
│   ├── dom.js                    el/svgEl yardımcıları
│   └── theme.js                  token erişimi
│
└── config/studio/
    ├── scenes/  overview · map · signal · sources · system · evidence
    ├── presets/ observatory · global-map · signal-focus · evidence · operations · broadcast
    └── policies/ takeover.json · priority.json
```

`config/studio/` kararı: kökteki `config/` ağacının parçası olur (lensler orada).
Sunum açısından **`web/config/studio/`** tercih edilir: API `ServeDir` fallback'i
zaten `web/` altını servis ediyor, böylece **backend'e hiç dokunulmaz**.

---

# 4. SAHNELER

Her sahne: amaç · ana görsel · ikincil · veri bağımlılığı · geçiş · boş durum ·
kritik durum · broadcast davranışı.

## SCENE 01 — GLOBAL OVERVIEW (varsayılan)

```
┌──────────────────────────────────────────────────────────┐
│ ◉ WORLD SIGNAL ENGINE   ● LIVE     GLOBAL OVERVIEW  12:04:32Z │
├──────────┬───────────────────────────────┬───────────────┤
│ METRICS  │                               │  LIVE FEED    │
│ (dikey)  │         GLOBE                 │  ───────────  │
│          │      (büyük, canlı)           │  ● CRITICAL   │
│ CATEGORY │                               │  ● HIGH       │
│ STRIP    │                               │  ● NORMAL     │
│ (döner)  │                               │  (bölge içi   │
├──────────┤                               │   kaydırma)   │
│ ACTIVITY │                               │               │
│ CHART    │                               │               │
├──────────┴───────────────────────────────┴───────────────┤
│ SON DAKİKA │ ~~~~ kayan şerit ~~~~                        │
└──────────────────────────────────────────────────────────┘
```

- **Amaç:** sistemin boşta duran canlı ekranı. Duvara asılır, sürekli çalışır.
- **Ana görsel:** küre (ekranın kalbi, en büyük alan).
- **İkincil:** metrik kolonu, kategori şeridi (döner), aktivite grafiği, feed.
- **Veri:** `/observatory` (kategoriler + sparkline + deviation), `/signals`
  (feed), `/activity` + SSE, `/metrics` (aktivite grafiği), `/world` (özet).
- **Geçiş:** fade.
- **Boş durum:** küre boş halka + "izlenecek veri yok"; feed "sinyal yok";
  kategori kartı "veri yok" + neden.
- **Kritik durum:** feed'de CRITICAL satırı kırmızı + ikon + etiket; kürede
  nabız halkası.
- **Broadcast:** 16:9'da üç kolon; 21:9'da metrik şeridi yataya döner.

## SCENE 02 — GLOBAL MAP

```
┌──────────────────────────────────────────────────────────┐
│ ● LIVE                              GLOBAL MAP      12:04Z │
├───────────────────────────────────────┬──────────────────┤
│                                       │  SIGNAL TYPES    │
│           WORLD MAP                   │  ──────────────  │
│    (equirectangular, gerçek koord.)   │  ◇ ANOMALY   3   │
│    ● deprem  ● hava  ○ sinyal bölgesi │  ⚡ NOW      2   │
│    pulse halkaları                    │                  │
│                                       │  TELEMETRY       │
│                                       │  ──────────────  │
│                                       │  REGION ACTIVITY │
├───────────────────────────────────────┴──────────────────┤
│ SON DAKİKA │ ~~~~                                         │
└──────────────────────────────────────────────────────────┘
```

- **Amaç:** nerede olduğunu göstermek.
- **Veri:** `/timeline?series=…` gözlemleri (lat/lon **dolu olanlar**), `/signals`.
- **Boş durum:** **kritik dürüstlük noktası.** Konumlu gözlem yoksa harita
  "KONUM VERİSİ YOK — 0 gözlem koordinat taşıyor" yazar. **Sahte nokta yok.**
- **Bilinen gerçek:** bugün yalnız bazı kaynaklar koordinat veriyor (AFAD deprem:
  `38.19/38.55` dolu; NOAA xray: `null`). Harita zaman zaman seyrek görünecek.
  Bu doğru davranış; beklenti yönetilmeli.

## SCENE 03 — SIGNAL / EVIDENCE (drill-down)

```
┌──────────────────────────────────────────────────────────┐
│ ● LIVE            SIGNAL INVESTIGATION      12:04Z        │
├──────────────────────────────────────────────────────────┤
│  ◇ ANOMALİ  ⚡ ŞİMDİ        doğrulandı                    │
│  Solar activity is rising sharply                         │
│  +16.3σ  ·  643 dk  ·  135 kanıt  ·  1 kaynak            │
├───────────────────────────┬──────────────────────────────┤
│ OBSERVATION               │  ANOMALY TIMELINE            │
│ (son değer + baseline)    │  normal ────╮                │
│                           │             ╰───● NOW        │
├───────────────────────────┴──────────────────────────────┤
│ EVIDENCE CHAIN                                            │
│ SIGNAL → EVENT → OBSERVATION → SOURCE → RAW DATA          │
│ (her adım tıklanabilir, sağda kanıt listesi)              │
└──────────────────────────────────────────────────────────┘
```

- **Amaç:** tek sinyali kanıtıyla incelemek.
- **Veri:** `/signals/:id` (evidence, quality, narrative, reasons), `/timeline`.
- **Boş durum:** bilinmeyenler bloğu (mevcut `reasons` alanı zaten bunu taşıyor).
- **Kritik durum:** yüksek σ'da timeline'da kırmızı bant.

## SCENE 04 — SOURCE OBSERVATORY

```
┌──────────────────────────────────────────────────────────┐
│ ● LIVE            SOURCE OBSERVATORY        12:04Z        │
├──────────────────────────────────────────────────────────┤
│  21 KAYNAK · 19 SAĞLIKLI · 1 BOZUK · 1 SINIRLI            │
├──────────────────────────────────────────────────────────┤
│  ● USGS          ● NASA NEO      ● GDELT                  │
│  ● AFAD          ● GOES X-ray    ▲ PyPI (degraded)        │
│  ● NWS           ● EONET         ● GitHub                 │
│  (durum ızgarası — tablo değil)                           │
├───────────────────────────┬──────────────────────────────┤
│  SEÇİLİ KAYNAK            │  KAYNAK TELEMETRİSİ          │
│  USGS Earthquake Feed     │  gecikme · son başarı        │
│  geophysics · 60s         │  kayıt · hata · sınırlama    │
└───────────────────────────┴──────────────────────────────┘
```

- **Amaç:** "kaynaklar" bir ayar ekranı değil, **gözlem istasyonu**.
- **Veri:** `/control` (sources: enabled/running/cadence/last_run/last_success/
  next_run/consecutive_failures), `/sources/:id` (health: records_received/
  changed/duplicate/errors/rate_limited/latency), `/metrics`.
- **Dürüstlük sınırı:** kaynak başına "olay oranı" endpoint'i **yok**. Uydurma
  oran çizilmeyecek; onun yerine gerçek olan **kadans + son başarı + ardışık hata**
  gösterilecek. "Gecikme" `/control.latency` ve `/sources/:id` health'ten gelir.
- **Boş durum:** hiç koşmamış kaynak "hiç çalışmadı" yazar (mevcut `/sources`
  listesinde PyPI `last_success: null` — gerçek örnek).
- **Kritik:** ardışık hata > 0 → amber; bozuk → kırmızı + neden.

## SCENE 05 — SYSTEM / OPERATIONS

```
┌──────────────────────────────────────────────────────────┐
│ ● LIVE              SYSTEM OPERATIONS       12:04Z        │
├──────────────────────────────────────────────────────────┤
│  SOURCE → COLLECT → OBSERVE → BASELINE → DETECT →        │
│  CORRELATE → SIGNAL                                      │
│  (her aşama: aktif · boşta · gecikmiş · hata)            │
├───────────────────────────┬──────────────────────────────┤
│  SAYAÇLAR                 │  GECİKME                     │
│  observations  2383       │  gözlem gecikmesi            │
│  anomalies      229       │  toplayıcı   118 ms          │
│  events          10       │  tespit        0 ms          │
│  signals         10       │  en yeni sinyal yaşı         │
├───────────────────────────┴──────────────────────────────┤
│  AKTİVİTE AKIŞI (SSE, canlı satırlar)                     │
└──────────────────────────────────────────────────────────┘
```

- **Amaç:** motorun kendisini göstermek.
- **Veri:** `/metrics` (gerçek sayaçlar: `wse_observations_total`,
  `wse_anomalies_total`, `wse_events_total`, `wse_signals_total`,
  `wse_collector_success_total/failure_total/rate_limited_total`,
  `wse_signal_types_total{type=…}`), `/control.latency`, `/activity`, SSE.
- **Dürüstlük sınırı:** **aşama başına ayrı metrik endpoint'i yok.** Pipeline
  görselleştirmesi gerçek sayaçların *türetilmiş* akışını gösterir; aşama
  durumu sayaç hareketinden çıkarılır, uydurulmaz. Veri olmayan aşama
  "ölçüm yok" der.

## SCENE 06 — EVIDENCE / RAW

```
┌──────────────────────────────────────────────────────────┐
│ ● LIVE                EVIDENCE / RAW        12:04Z        │
├───────────────────────────┬──────────────────────────────┤
│  KANIT ZİNCİRİ            │  HAM VERİ                    │
│  SIGNAL   ✓               │  ┌────────────────────────┐  │
│  EVENT    ✓               │  │ ham yük (biçimli,       │  │
│  OBSERV.  ✓               │  │ katlanabilir)           │  │
│  SOURCE   ✓               │  └────────────────────────┘  │
│  RAW      ●               │  meta: alındı · gözlendi     │
└───────────────────────────┴──────────────────────────────┘
```

- **Amaç:** delil zinciri. Projenin 2. başarı ölçütü.
- **Veri:** `/signals/:id`, `/events/:id`, `/observations/:id`,
  `/observations/:id/raw`, `/sources/:id`.
- **Kural:** ham JSON ekranın tamamına basılmaz; önce özet → kaynak → gözlem →
  ham sırası. Ham yük katlanmış gelir.

## SCENE 07 — BREAKING TAKEOVER (sahne değil, devralma)

Bkz. §7.

## SCENE 08 — CONTROL ROOM (sahne değil, katman)

Bkz. §8.

---

# 5. BLOK KATALOĞU

| Blok | Görev | Veri | Boş durum |
|---|---|---|---|
| `globe` | Ortografik küre, anomali noktaları, halkalar | `/observatory`, `/timeline` | boş halka + "veri yok" |
| `world-map` | Equirectangular harita, gerçek koordinatlar | `/timeline` (lat/lon) | "konum verisi yok" |
| `signal-feed` | Canlı sinyal akışı, öncelik rozetli | `/signals` + SSE | "sinyal yok" |
| `signal-card` | Tek sinyalin büyük künyesi | `/signals/:id` | 404 → neden |
| `metric` | Tek büyük sayı + delta + sparkline | `/observatory` | `—` + neden |
| `metric-group` | Metrik kolonu | `/observatory`, `/metrics` | kısmi doldurma |
| `category-strip` | Kategori kartları, **döner** (`3/13` sayacı) | `/observatory` | "veri yok" + neden |
| `activity-chart` | Zaman içinde gözlem/anomali yoğunluğu | `/metrics`, `/activity` | eksen + "veri yok" |
| `timeline` | Bir serinin geçmişi + baseline bandı | `/timeline` | "seri yok" |
| `evidence-chain` | Sinyal→event→gözlem→kaynak→ham zinciri | `/signals/:id` | eksik halka işaretli |
| `source-health` | Kaynak durum ızgarası | `/control`, `/sources` | "hiç çalışmadı" |
| `source-card` | Seçili kaynak künyesi + telemetri | `/sources/:id` | 404 → neden |
| `system-pipeline` | Aşama akışı, durum renkleri | `/metrics`, `/control` | "ölçüm yok" |
| `raw-viewer` | Ham yük, biçimli, katlanabilir | `/observations/:id/raw` | "ham yük yok" |
| `telemetry` | Gecikme/gecikme/uptime satırları | `/control.latency` | "ölçüm yok" |
| `status-grid` | Genel durum kutucukları | `/world`, `/control` | — |
| `ticker` | Son dakika kayan şerit | SSE + `/signals` | boş şerit gizlenir |
| `breaking` | Ekranı devralan flaş katmanı | SSE | tetiklenmezse görünmez |

## 5.1 Blok sözleşmesi

```js
registerBlock("globe", {
  mount(el, ctx)  {},   // DOM kur, dinleyici ekle
  update(el, ctx) {},   // yeni veri geldi, yeniden çiz
  resize(el, ctx) {},   // bölge boyutu değişti
  unmount(el)     {},   // interval/rAF/SSE/observer TEMİZLE
});
```

`ctx` = `{ data, on(event, fn), off, t(), theme, region }`.

## 5.2 Taşma davranışı (bölge seviyesinde)

Kullanıcı: *"hiçbir şey sıkıştırılmayacak."*

```jsonc
{ "id": "strip", "block": "category-strip",
  "overflow": "rotate",      // rotate | scroll | clip-with-count
  "rotate": { "every": 8, "count": true } }
```

- `rotate` — içerik sığmazsa otomatik döner, köşede `3/13` sayacı.
- `scroll` — bölge kendi içinde kayar (sayfa kaymaz).
- `clip-with-count` — yalnız gerçekten uygunsuz içerikte; **"N daha" gizlemesi
  yasak**, sayaç zorunlu.
- Blok sığdırma denemesi yapmaz; sıkıştırma **yasak** (motor `min-height` uygular).

---

# 6. EVENT DIRECTOR

Yeni mimarinin kalbi. `js/studio/director.js` + `priority.js`.

## 6.1 Veri akışı

```text
SSE (/events) ──┐
                ├──► EVENT BUS ──► EVENT DIRECTOR
fetch poll ─────┘                       │
                                        ├─► update      bölgelere veri it
                                        ├─► ticker      şeride satır ekle
                                        ├─► transition  sahne değiştir
                                        └─► takeover    breaking başlat
```

## 6.2 Öncelik (gerçek veriyle)

```text
INFO      normal gözlem akışı
LOW       düşük sapma
MEDIUM    ANOMALY, σ 3–5
HIGH      ANOMALY, σ ≥ 5  veya çok kaynaklı
CRITICAL  CONVERGENCE + IMPACT, veya σ ≥ 8
```

**Dürüst tespit:** bugün `/world` `by_type` yalnız `NOW` ve `ANOMALY` üretiyor;
`EARLY_SIGNAL`, `CONVERGENCE`, `IMPACT` sayaçları **0**. Yani CRITICAL tetikleyen
CONVERGENCE şu an pratikte hiç gelmiyor. Bu yüzden:
- v1 politika **σ tabanlı** çalışır (gerçek veriyle test edilebilir),
- CONVERGENCE/IMPACT kuralları **tanımlı ama uykuda** (veri gelince devreye girer),
- bu durum arayüzde açıkça yazılır, gizlenmez.

## 6.3 Takeover politikası (`config/studio/policies/takeover.json`)

```jsonc
{
  "enabled": false,                      // AUTO DIRECTOR default OFF
  "min_priority": "HIGH",
  "cooldown_seconds": 120,               // aynı olay için tekrar açma
  "dismiss_after_seconds": 30,
  "rules": [
    { "when": "priority >= CRITICAL",                      "action": "takeover" },
    { "when": "priority >= HIGH && types has CONVERGENCE", "action": "takeover" },
    { "when": "priority >= HIGH",                          "action": "transition", "to": "signal" },
    { "when": "priority <= MEDIUM",                        "action": "update" }
  ]
}
```

Kurallar **veri**, `if/else` değil. Kullanıcı kontrol odasından düzenler.
UI kodunda `if (signal.type === …)` **yasak**.

## 6.4 Simülasyon katmanı (zorunlu)

`js/sim/simulation.js` — **yalnız `?sim=1` veya DEV'de yüklenir.** Production
verisine sahte olay **karışmaz**; simülatör ayrı bir `EventBus` kanalına yazar ve
gerçek `store`'a dokunmaz.

```js
simulateSignal(priority)      simulateBreaking()
simulateConvergence()         simulateSceneTransition(sceneId)
simulateSourceFailure(id)     simulateResolved()
```

Bu olmadan breaking/director **gerçek SSE beklenerek geliştirilemez** — sistem
zaten sakin, saatlerce kritik sinyal gelmeyebilir.

---

# 7. BREAKING SYSTEM

Mevcut flash kartı (`openAlert`/`paintAlert`/`.obs-alert-*`) **tamamen kaldırılır**.

## 7.1 Akış

```text
NORMAL YAYIN
   ↓  (director: priority >= eşik)
PRE-ALERT     ekran kenarında ince amber/kırmızı çerçeve nabzı (~0.8s)
   ↓
TRANSITION    mevcut sahne %8 küçülür + kararır (broadcast-cut)
   ↓
BREAKING SCENE ekranı devralır
   ↓
EVIDENCE REVEAL  kanıt sayıları sırayla açılır
   ↓
OTOMATİK KAPANIŞ  geri sayım görünür, sonra normal sahneye döner
```

## 7.2 Kompozisyon

```
┌──────────────────────────────────────────────────────────┐
│  🔴 BREAKING · CANLI                    kapanış 00:24    │
├──────────────────────────────────────────────────────────┤
│                                                          │
│        BÜYÜK SİNYAL TESPİT EDİLDİ                        │
│        Solar activity is rising sharply                  │
│                                                          │
│              +16.3σ                                      │
│              sapma                                       │
│                                                          │
│        ┌──────────────────────────────────┐              │
│        │        WORLD MAP / GLOBE         │              │
│        └──────────────────────────────────┘              │
│                                                          │
│   KANIT 135    KAYNAK 1    SÜRE 643 dk    GÜVEN 0.64     │
├──────────────────────────────────────────────────────────┤
│  SON DAKİKA │ ~~~~~ canlı güncelleme ~~~~~               │
└──────────────────────────────────────────────────────────┘
```

## 7.3 Kurallar

- Animasyon **anlamlı**, abartısız. Epileptik flashing **yasak**: yanıp sönme
  ≤ 1 Hz, tercihen tek seferlik giriş animasyonu.
- `prefers-reduced-motion` → geçiş anında, animasyonsuz.
- Kapatma: **"RAPORU AÇ"** (SCENE 03'e geçer) ve **"KAPAT"** (`Esc`).
- Otomatik kapanış geri sayımı **görünür**.
- Tüm alanlar gerçek veriden; yoksa `—` + gerekçe.

---

# 8. CONTROL ROOM

## 8.1 Davranış

- Normalde **yok**. Ekranda sıfır web kromu.
- `K` ile sağdan kayarak açılır, `Esc` kapatır. Kapalıyken `display: none`.
- Açıkken yayın **arkasında çalışmaya devam eder** (SSE kesilmez).

## 8.2 Sekmeler

| Sekme | İçerik |
|---|---|
| **SCENE** | sahne seç, auto director ON/OFF, geçiş stili, takeover politikası |
| **LAYOUT** | bölge ekle/kaldır/taşı, blok değiştir, boyutlandır, preset, kaydet/sıfırla |
| **DATA** | lens, kategori, kaynak, zaman aralığı |
| **SYSTEM** | bağlantı durumu, SSE, API anahtarı, gecikme, dil, teşhis |

## 8.3 Amaç

Kullanıcıyı *web uygulaması operatörü* değil **yayın rejisi** yapar:
"Bu ekranda ne göstermek istiyorum?"

## 8.4 Kalıcı broadcast metadata

Yalnız şunlar kalır ve sahnenin parçasıdır:
`● LIVE` · `WORLD SIGNAL ENGINE` · `12:04:32Z` · sahne adı.

---

# 9. LAYOUT SYSTEM

## 9.1 Sahne tanımı (JSON)

```jsonc
{
  "id": "overview",
  "name": { "tr": "GENEL BAKIŞ", "en": "GLOBAL OVERVIEW" },
  "grid": { "cols": 12, "rows": 8, "gap": 12 },
  "safe_area": { "top": 56, "right": 24, "bottom": 44, "left": 24 },
  "transition": "fade",
  "regions": [
    { "id": "header",  "area": "1 / 1 / 2 / 13",  "block": "status-grid",    "priority": 100 },
    { "id": "metrics", "area": "2 / 1 / 6 / 4",   "block": "metric-group",   "priority": 90 },
    { "id": "globe",   "area": "2 / 4 / 7 / 10",  "block": "globe",          "priority": 100 },
    { "id": "feed",    "area": "2 / 10 / 7 / 13", "block": "signal-feed",
      "overflow": "scroll", "priority": 95 },
    { "id": "strip",   "area": "6 / 1 / 8 / 4",   "block": "category-strip",
      "overflow": "rotate", "rotate": { "every": 8 }, "priority": 70 },
    { "id": "chart",   "area": "7 / 4 / 8 / 10",  "block": "activity-chart", "priority": 60 },
    { "id": "ticker",  "area": "8 / 1 / 9 / 13",  "block": "ticker",         "priority": 80 }
  ]
}
```

- `area` — CSS grid satır/kolon. **Taşımak = bu satırı değiştirmek.**
- `priority` — çakışmada kim kazanır; ayrıca küçük ekranda kim düşer.
- Bölge alanları motor tarafından doğrulanır; çakışma tespit edilirse uyarı +
  son geçerli düzene dönüş.

## 9.2 Presetler

`observatory` · `global-map` · `signal-focus` · `evidence` · `operations` ·
`broadcast` (kromsuz, salt yayın).

## 9.3 Kullanıcı düzeni

- Değişiklikler `localStorage`'a yazılır (`wse.studio.layout.<scene>`).
- "SIFIRLA" varsayılana döner.
- İleri faz: sunucuya kaydetme (`POST /studio/layout`) — **backend işi, ayrı onay**.

---

# 10. BROADCAST RESPONSIVE STRATEJİSİ

Klasik `mobile/tablet/desktop` **yetersiz**. Eksen: **aspect ratio**.

| Format | Öncelik | Davranış |
|---|---|---|
| **16:9** (1920×1080, 2560×1440, 3840×2160) | **birincil** | tam kompozisyon |
| 16:10 / 3:2 | ikincil | kolon oranı hafif kayar |
| 21:9 / ultrawide | desteklenir | metrik şeridi yataya döner, harita genişler |
| 4:3 | desteklenir | yan kolonlar daralır, öncelik sırasına göre düşer |
| **9:16 / 1080×1920 (dikey)** | ayrı kompozisyon | **dikey sahne varyantı**, küçültme değil |

## 10.1 Kurallar

- Ölçekleme **`rem`/`clamp()`** ile akışkan; `transform: scale()` ile
  küçültme **yasak** (okunabilirliği bozar, hit-area'yı kaydırır).
- `safe_area` her sahnede tanımlı; TV overscan'i için kenar boşluğu korunur.
- Bölge `min-width`/`min-height` motor tarafından uygulanır; altına inen bölge
  `priority` sırasına göre bir alt presete düşer.
- Dikey ekran **kendi sahnesini** kullanır (`overview.vertical.json`), aynı
  bölgeleri tek kolona dizer.
- Test çözünürlükleri: 1920×1080, 1600×900, 1366×768, 1280×720, 3840×2160,
  2560×1440, 1080×1920, 3440×1440.

## 10.2 Değişmez

```text
scrollHeight === innerHeight
scrollWidth  === innerWidth
```

Her çözünürlükte. Browser scrollbar **asla** çıkmaz.

---

# 11. MIGRATION STRATEJİSİ

## 11.1 Yeniden kullanım matrisi

| Öğe (satır aralığı) | Karar | Nereye |
|---|---|---|
| `el`, `svgEl` (66–87) | **REUSE** | `dom.js` |
| `fmtTime…fmtValue`, `relative` (89–165) | **REUSE** | `fmt.js` |
| `api`, `post` (177–206) | **ADAPT** | `data/api.js` |
| `loadLenses`, `activeLens`, `withLens` (207–237) | **ADAPT** | `data/store.js` |
| I18N tablosu + `t`, `setLang`, `detectLang` (533–560) | **REUSE + genişlet** | `i18n.js` |
| `typeLabel`, `statusLabel`, `directionLabel` (555–559) | **REUSE** | `i18n.js` |
| `sparkline` (604–637) | **REUSE** | `blocks/_chart.js` |
| `qualityBars` (638–662) | **ADAPT** | `signal-card` içi |
| `ortho`, `obsGlobe` (944–1019) | **REUSE** | `blocks/globe.js` |
| `timelineChart` (1879–1907) | **REUSE** | `blocks/timeline.js` |
| `project` (1935) | **ADAPT** | `blocks/world-map.js` |
| `readStream`, `parseSseFrame`, `runStream` (2261–2293) | **REUSE** | `data/sse.js` |
| `addSseHandler`/`removeSseHandler` (1998–2000) | **ADAPT** | `event-bus.js` |
| `setConn` (2288) | **ADAPT** | `controls/system` |
| `toast` (166–176) | **DELETE** | yayında toast yok → ticker/breaking |
| Tüm `obs*` (695–1271, ~577 satır) | **DELETE** | bloklar devralır |
| `openAlert`/`closeAlert`/`paintAlert` (1273–1334) | **DELETE** | `blocks/breaking.js` |
| `worldView` + yardımcıları (1376–1578) | **REWRITE** | `metric-group`, `signal-feed`, `category-strip` |
| `signalView` (1579–1674) | **REWRITE** | `signal-card` + `evidence-chain` |
| `eventView` (1675–1712) | **REWRITE** | `timeline` |
| `observationView` (1713–1772) | **REWRITE** | `raw-viewer` |
| `sourceView`, `sourcesIndex` (1789–1846) | **REWRITE** | `source-health`, `source-card` |
| `lensesView` (1847–1878) | **REWRITE** | kontrol odası DATA sekmesi |
| `mapView` (1939–1997) | **REWRITE** | `world-map` |
| `systemView`, `activityRow`, `appendActivity` (2002–2152) | **REWRITE** | `system-pipeline`, `telemetry` |
| `render`, `crumbs`, `errorView` (1335–1375) | **ADAPT** | `scene.js` mount |
| `route`, `applyChrome` (2294–2346) | **REPLACE** | `scene.js` + URL eşlemesi |
| `styles.css` (864 satır) | **REPLACE** | `styles/` ağacı |
| `index.html` topbar/nav/footer | **DELETE** | yayın kabuğu |

**Yaklaşık:** ~350 satır REUSE, ~450 ADAPT, ~470 REWRITE, ~720 DELETE.
Hibrit "yeni studio + eski app.js" **oluşturulmaz**.

## 11.2 Sıra (geri dönüşümlü)

| Faz | İş | Çıktı |
|---|---|---|
| **B0** | Audit (bu belge) | onay |
| **B1** | Studio shell: tam ekran, kaymayan kabuk, tema tokenleri | boş ama kaymayan yayın ekranı |
| **B2** | Block engine + registry + yaşam döngüsü | 2 örnek blok çalışır |
| **B3** | Data bus: api/sse/store/adapters | bloklar veri alır, fetch etmez |
| **B4** | SCENE 01 GLOBAL OVERVIEW | gerçek yayın ekranı görünür |
| **B5** | SCENE 02 GLOBAL MAP | harita |
| **B6** | SCENE 03 + 06 SIGNAL / EVIDENCE / RAW | drill-down |
| **B7** | SCENE 04 + 05 SOURCES / SYSTEM | gözlem istasyonu + pipeline |
| **B8** | Event director + transition + simülasyon | sahne geçişi |
| **B9** | BREAKING takeover | ekran devralma |
| **B10** | CONTROL ROOM | sağ panel |
| **B11** | Preset + kullanıcı düzeni (ekle/çıkar/taşı/kaydet) | layout kullanıcı elinde |
| **B12** | Broadcast formats (4K, ultrawide, dikey) | format desteği |
| **B13** | Cila: tipografi, erişilebilirlik, performans | yayına hazır |
| **B14** | Eski sayfaların kaldırılması + ADR | temizlik |

Her faz: **tek commit**, çalışır durumda, Türkçe rapor (§39 formatı).
Eski UI B14'e kadar `#/legacy` altında erişilebilir kalır; sonra silinir.

## 11.3 URL eşlemesi

```text
#/studio/overview          → scene overview
#/studio/map               → scene map
#/studio/signal/<id>       → scene signal
#/studio/evidence/<id>     → scene evidence
#/studio/sources           → scene sources
#/studio/system            → scene system
```

Eski URL'ler yönlendirilir; kırılmaz. URL değişimi **sayfa yeniden yüklemez**.

---

# 12. TEST STRATEJİSİ

## 12.1 Çözünürlük matrisi (zorunlu)

`1920×1080` · `1600×900` · `1366×768` · `1280×720` · `3840×2160` ·
`2560×1440` · `1080×1920` · `3440×1440`

Her biri için: `scrollHeight === innerHeight`, `scrollWidth === innerWidth`,
taşan bölge yok, kırpılan kritik bilgi yok, çakışan bölge yok.

## 12.2 Blok testi (her blok için)

`mount` · `update` · `resize` · `unmount` · boş veri · kısmi veri · eksik
opsiyonel alan · canlı güncelleme · `unmount` sonrası sızıntı yok.

## 12.3 Yayın testi (simülasyonla)

`normal` · `medium` · `high` · `critical` · `çoklu olay` · `çözüldü` ·
tam akış: `overview → breaking → evidence → overview`.

## 12.4 Otomatik kontrol listesi

```text
no page scroll          no horizontal overflow
no clipped info         no overlapping regions
no NaN                  no undefined
no broken SVG           no missing-data fabrication
no leaked interval      no leaked rAF
no leaked SSE listener  no duplicate fetch
```

## 12.5 Rust tarafı (değişmez)

`cargo fmt --check` · `cargo clippy -- -D warnings` · `cargo test --workspace`.
Backend değişmediği için bunlar regresyon bekçisi.

## 12.6 Görsel kabul ölçütü

Ekrana bakan kişi **"bir web sitesindeyim"** dememeli.
**"Dünyayı canlı izleyen bir gözlem merkezinin yayın ekranına bakıyorum"** demeli.
Bu, diğer tüm UI kararlarından önce gelir.

---

# 13. RİSKLER

| # | Risk | Olasılık | Etki | Önlem |
|---|---|---|---|---|
| 1 | **Harita verisi seyrek.** Koordinat yalnız bazı gözlemlerde var; sinyal seviyesinde `location` bugün hep `null`. | Yüksek | Orta | Harita "konum verisi yok" der; **sahte nokta yok**. Beklenti açıkça yazılır. |
| 2 | **CRITICAL pratikte hiç tetiklenmez.** `CONVERGENCE`/`IMPACT` sayaçları 0. | Yüksek | Orta | Politika σ tabanlı; CONVERGENCE kuralları uykuda ve bu durum arayüzde yazılı. |
| 3 | **Aşama başına metrik yok.** Pipeline görselleştirmesi türetilmiş. | Orta | Orta | Yalnız gerçek sayaçlar; "ölçüm yok" durumu dürüst gösterilir. |
| 4 | **Kaynak başına olay oranı yok.** | Orta | Düşük | Oran uydurulmaz; kadans + son başarı + ardışık hata gösterilir. |
| 5 | **Otomatik geçiş rahatsız edici olabilir.** | Orta | Orta | **Varsayılan KAPALI**; her kural ayrı kapatılabilir; cooldown var. |
| 6 | **Büyük yeniden yazım.** ~2400 satır yerine ~20 modül. | Orta | Yüksek | 14 faza bölündü; her fazda çalışan ekran; eski UI B14'e kadar durur. |
| 7 | **Kullanıcı düzeni çakışabilir.** Üst üste bölge. | Orta | Düşük | Motor çakışma tespit eder, uyarır, son geçerli düzene döner. |
| 8 | **Build adımı baskısı.** Modülerlik "npm gerekir" beklentisi doğurur. | Orta | Orta | ADR 0026 korunur: yerel ES modülleri. Gerekirse ayrı ADR ile Vite — **ayrı onay**. |
| 9 | **Uzun açık ekranda sızıntı.** | Orta | Yüksek | Yaşam döngüsü zorunlu; unmount testi; sızıntı kontrol listesi. |
| 10 | **Dikey/4K kompozisyon** sonradan eklenirse büyük refactor. | Orta | Orta | `safe_area` + `priority` + dikey varyant **baştan** mimaride. |
| 11 | **Erişilebilirlik geriler.** Yayın estetiği klavye/ekran okuyucuyu bozabilir. | Orta | Orta | `aria-live`, odak yönetimi, `prefers-reduced-motion`, AA kontrast B1'den itibaren. |

---

# 14. MEVCUT PLANDAN DEĞİŞEN KARARLAR

`docs/plans/broadcast-studio.md` (bu oturumda yazıldı) **v2 ile değiştirilmiştir**;
iki çelişkili plan bırakılmaması için o dosya kaldırılır.

| Konu | v1 (önceki plan) | v2 (bu plan) | Neden |
|---|---|---|---|
| Yönetim katmanı | yok (statik kompozisyon) | **BROADCAST DIRECTOR + EVENT DIRECTOR** | Kullanıcı: "asıl farkı yaratacak olan yayını yönetebilmesi" |
| Olay yönlendirme | SSE doğrudan bloklara | **SSE → EVENT BUS → DIRECTOR** | `if (type===…)` yayılmasını önler |
| Sahne geçişi | `take` kuralları (JSON) | Director + **priority** + takeover policy | Öncelik mekanizması istendi |
| Breaking | "flash'ı kaldır, takeover ekle" | **PRE-ALERT → TRANSITION → TAKEOVER → EVIDENCE REVEAL → AUTO-CLOSE** | Tam yayın akışı istendi |
| Responsive | 4 kırılım noktası | **aspect-ratio eksenli**, 4K + dikey varyant | "4K TV / vertical display" istendi |
| Kontrol odası | 4 sekme | aynı + **LAYOUT** sekmesi (ekle/çıkar/taşı/kaydet) | Bölge yönetimi kullanıcıda olmalı |
| Test | tarayıcı ölçümü | **blok yaşam döngüsü + simülasyon + format matrisi** | Director simülasyonsuz test edilemez |
| Simülasyon | yok | **zorunlu katman** (`?sim=1`) | Gerçek kritik olay saatlerce gelmeyebilir |
| Krom | broadcast modu (`?broadcast=1`) | **krom tamamen yok**; yalnız LIVE/marka/saat | "sıfır web chrome" istendi |
| Eski kod | "parçala" | **açık KEEP/REUSE/ADAPT/REWRITE/DELETE matrisi** | "körlemesine silme" uyarısı |
| Dikey | yok | **dikey sahne varyantı** | Küçültme kabul edilmedi |
| Kapsam | yalnız gözlemevi | **tüm web katmanı** | "tüm menüleri değiştir, hepsi görsel" |

---

# 15. ONAY BEKLENEN NOKTALAR

1. **Audit ve teşhis (§1)** doğru mu? Kaçırdığım bir bağımlılık var mı?
2. **Director katmanı (§2, §6)** — kullanıcının eklediği iki katman doğru yere mi oturdu?
3. **Sahne listesi (§4)** — 6 sahne + breaking + control room yeterli mi, eksik var mı?
4. **Blok listesi (§5)** — 18 blok doğru mu? Eklenmesi/çıkarılması gereken var mı?
5. **Faz sırası (§11.2)** — B1–B14 uygun mu, yoksa daha kaba mı bölünsün?
6. **Config yeri** — `web/config/studio/` (backend'e dokunulmaz) onay mı?
7. **Kullanıcı düzeni kaydı** — yalnız `localStorage` mı, sunucuya kayıt da olsun mu
   (backend işi gerektirir, ayrı onay)?
8. **Eski UI** — B14'e kadar `#/legacy` altında dursun mu, yoksa B1'de hemen kaldırılsın mı?
9. **Build adımı** — yerel ES modülleri (ADR 0026 korunur) onay mı?
10. **Otomatik yayın** — varsayılan KAPALI onay mı?

Onay sonrası **B1 (studio shell)** ile başlanır. Kod yazılmadan önce bu belge
onaylanacak; değişiklik istenirse önce bu dosya güncellenir.
