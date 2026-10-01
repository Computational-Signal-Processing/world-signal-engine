# Yayın Stüdyosu — web katmanının yeniden inşası

> **GEÇERSİZ — `docs/plans/broadcast-studio-v2.md` ile değiştirildi.**
> Bu v1 planı statik bir kompozisyon motoru tarif ediyordu. v2, yayını yöneten
> `BROADCAST DIRECTOR` + `EVENT DIRECTOR` katmanlarını ekliyor ve kapsamı tüm
> web katmanına genişletiyor. Yeni iş için **v2'yi** kullanın; bu dosya yalnız
> karar geçmişi için duruyor.

Durum: **geçersiz (v2 ile değiştirildi)** · Tarih: 2026-10-01
İlgili: `docs/decisions/0026-observatory-ui.md`, `docs/plans/observatory-v2.md`
Referans: `docs/design/reference/gemini-kozmik-yayin-studyosu.html`

---

## 1. Karar özeti

Mevcut web katmanı bir **web sitesi** gibi davranıyor: sayfalar arasında gezinilir,
içerik akış içinde aşağı kayar, her şey tek bir `app.js` içinde birbirine geçmiştir.
İstenen bu değil.

İstenen: **bir TV yayın ekranı.** Tam ekran, sabit, kaymaz. Gelen sinyale göre
ekran kendiliğinden başka bir sahneye geçebilir. Kanallar arasında TV menüsü gibi
geçilir. Bölgeler istenildiği zaman eklenir, çıkarılır, yerleri değişir. Her şey —
harita, kaynaklar, sistem, gözlemevi, ham veriye inen detay — görseldir.

Bu plan üç şeyi birlikte getiriyor:

1. **Yayın modeli** — ekran, veri değil *sahne* gösterir. Sahne = yerleşim + bölgeler + bloklar.
2. **Modülerlik** — her bölge bağımsız bir **blok**. Blok kayıt defteri (registry) var.
   Yeni bölge eklemek çekirdeğe dokunmaz; JSON ile açılır/kapanır/yer değiştirir.
3. **Kaymama** — hiçbir bölge içeriği sıkıştırmaz. İçerik bölgeye sığmazsa bölge
   **kendi içinde** döner (rotate/carousel) veya kendi içinde kayar. Sayfa asla kaymaz.

---

## 2. Neyi saklıyoruz, neyi çöpe atıyoruz

| Mevcut | Karar | Gerekçe |
|---|---|---|
| `web/app.js` (2393 satır, tek dosya) | **Parçala** → `web/js/` ES modülleri | "Her şey iç içe geçmiş" şikâyetinin teknik karşılığı bu |
| `web/index.html` topbar + nav + footer | **Çöpe** | Web sitesi iskeleti; yayın ekranında yeri yok |
| `#/world`, `#/signal`, `#/event`, `#/observation`, `#/source` sayfa düzeni | **Blok olur** | Drill-down zinciri korunur ama artık *sahne* içinde |
| Gözlemevi grid'i (`obs-*`) | **Çöpe** | Yerini blok/sahne sistemi alır |
| `?broadcast=1` modu | **Çöpe** | Artık tek mod var: yayın. Kontrol odası ayrı katman |
| SSE akışı (`/events`), `startActivityStream` | **Koru + güçlendir** | Yayın motorunun kalbi; sahne geçişini bu tetikler |
| `api()`, `fmt*`, `sparkline`, `timelineChart`, `ortho`/globe, i18n | **Koru, modülleştir** | Saf fonksiyonlar; yeniden yazmak israf |
| `#/system`, `#/sources`, `#/lenses`, `#/map` | **Görsel bloklara dönüştür** | Kullanıcı: "kaynaklarımız, sistem, dünya... hepsi görselleşecek" |
| ADR 0026 "build adımı yok" | **Koru** | Aşağıda §10'da gerekçe |

---

## 3. Çekirdek fikir

```
YAYIN  =  SAHNE  ×  BÖLGE  ×  BLOK
```

- **Blok** — tek bir görsel birim. Kendi verisini bilir, kendi çizimini yapar.
  Örnek: `globe`, `category-strip`, `signal-feed`, `timeline`, `source-health`,
  `activity-chart`, `metric`, `map`, `evidence-list`, `raw-viewer`.
- **Bölge** — sahnedeki bir dikdörtgen. Hangi bloğun oraya çizileceğini söyler.
  Bölge başına: blok tipi, veri kaynağı, boyut, öncelik.
- **Sahne** — ekranın o anki tam hâli. Bölgelerin düzeni + geçiş kuralları.
  Kanallar sahnelerdir: `genel-bakis`, `harita`, `kaynaklar`, `sistem`,
  `zaman-cizelgesi`, `kanit`.

Bir blok **eklenebilir, çıkarılabilir, taşınabilir** olmalı. Bunu mümkün kılan şey
blok kayıt defteri:

```js
// web/js/blocks/index.js
registerBlock("globe",  { mount(el, ctx) {...}, update(el, ctx) {...}, unmount(el) {...} });
registerBlock("signal-feed", { ... });
```

Çekirdek (`studio.js`) hiçbir blok tipini tanımaz. Sadece "şu bölgeye şu blok tipini
kur, verisini ver, güncelle" der. **Yeni bölge = yeni dosya + bir kayıt satırı.**

---

## 4. Yapılandırma — JSON ile düzen

Düzen kod değil, veri olacak. Kullanıcı: *"istediğimiz zaman istediğimiz layout'u
koyabilelim"*. Bunun yolu, düzeni çalışma anında değiştirilebilir bir JSON olarak
tutmak:

```jsonc
{
  "id": "genel-bakis",
  "name": { "tr": "GENEL BAKIŞ", "en": "OVERVIEW" },
  "grid": { "cols": 12, "rows": 6, "gap": 12 },
  "regions": [
    { "id": "r-strip", "area": "1 / 1 / 2 / 13", "block": "category-strip",
      "source": "/observatory.categories", "rotate": { "every": 8 } },
    { "id": "r-globe", "area": "2 / 5 / 6 / 9",  "block": "globe",
      "source": "/observatory.geo" },
    { "id": "r-feed",  "area": "2 / 9 / 6 / 13", "block": "signal-feed",
      "source": "/signals", "scroll": "auto" }
  ],
  "take": [
    { "when": "signal.type includes CONVERGENCE", "go": "kanit", "after": 25 },
    { "when": "signal.severity == HIGH",         "go": "harita", "after": 20 }
  ]
}
```

- `area` — CSS grid konumu. Bölgeyi taşımak = bu satırı değiştirmek.
- `rotate` — sığmayan içerik için **kendi içinde** dönüş. Sıkıştırma yok.
- `scroll` — bölge içi kaydırma; sayfa değil, bölge kayar.
- `take` — **otomatik sahne geçişi** ("ekran donmeye başlar başka ekrana geçer").

Düzenler `config/studio/scenes/*.json` altında durur, API bunları servis eder.
Kullanıcı kendi düzenini kaydedebilir (localStorage + gerekirse `POST /studio/layout`).

---

## 5. Blok kataloğu

| Blok | Ne gösterir | Veri |
|---|---|---|
| `category-strip` | Kategori kartları; sığmazsa kendi içinde döner | `/observatory` |
| `globe` | Ortografik küre, anomali noktaları, halkalar | `/observatory` + `/timeline` |
| `map` | Düz harita, gözlem koordinatları (deprem, hava) | `/timeline` (lat/lon) |
| `signal-feed` | Sinyal akışı, seviye rozetli | `/signals` + SSE |
| `signal-card` | Tek sinyalin büyük künyesi | `/signals/:id` |
| `metric` | Tek büyük sayı + delta + sparkline | `/observatory` |
| `activity-chart` | Zaman içinde gözlem/sinyal yoğunluğu | `/metrics` + `/activity` |
| `timeline` | Bir serinin geçmişi + baseline bandı | `/timeline` |
| `evidence-list` | Kanıt satırları → gözlem → kaynak → ham | `/signals/:id.evidence` |
| `source-health` | Kaynak sağlık ızgarası (renk + şekil + metin) | `/control` |
| `raw-viewer` | Ham yükün kendisi | `/observations/:id/raw` |
| `ticker` | Son dakika kayan şerit | SSE + `/signals` |
| `breaking` | Ekranı devralan flaş katmanı | SSE (`signal`) |

Her blok: `icon + label + shape + typography + state` ile anlaşılır. **Renk tek
başına anlam taşımaz** (proje kuralı).

---

## 6. Sahne akışı — "TV gibi"

### Kanal menüsü
Referanstaki `scene-selector` modeli: ekranın üstünde sahne düğmeleri.
Klavye ile de erişilir (`1`–`9`, `Tab`). Aktif sahne işaretli.

### Otomatik geçiş (`take`)
SSE'den gelen olaya göre motor sahneyi kendiliğinden değiştirir:
- Yeni `CONVERGENCE` → kanıt sahnesine geç, N saniye sonra geri dön.
- `severity == HIGH` → harita sahnesine geç.
- Sinyal `RESOLVED` → genel bakışa dön.
Geçişler `prefers-reduced-motion` saygılı; animasyon kapatılabilir.

### FLAŞ (breaking takeover)
Kullanıcının tarifi: *"breaking news gibi bir anda kayarak gelir, ekran donar,
başka ekrana geçer."* Mevcut "flash kartı" bu değil — **çöpe atılıyor.**
Yeni `breaking` bloğu:
1. Ekranın altından şerit kayarak girer (`translateY`), ekranı karartır.
2. Gerçek kanıtı gösterir: seviye, sapma (σ), süre, kaynak sayısı, ilk görülme.
3. İki eylem: **"RAPORU AÇ"** (kanıt sahnesine geçer) ve **"KAPAT"**.
4. Otomatik kapanma süresi sayılır (geri sayım görünür).
5. Uydurma yok: alanlar boşsa `—` ve gerekçe yazılır.

### Kaymayan içerik
Kullanıcı: *"bu daha fazla kategori gibi olan şeyler kayabilir, hiçbir şey
sıkıştırılmayacak."* Kural:
- Sayfa **asla** kaymaz (`overflow: hidden`).
- Bölge içeriği sığmazsa: `rotate` (döner) veya bölge içi `scroll` (kendi içinde kayar).
- "N kategori daha" gibi **gizleme yok** — hepsi sırayla görünür.
- Bölge köşesinde içerik sayacı (`3/13`) — kullanıcı döndüğünü bilir.

---

## 7. Yerleşim presetleri

Hazır düzenler; kullanıcı seçer, kendi düzenini kaydeder:

| Preset | Yapı | Kime |
|---|---|---|
| `wall` | Şerit + küre + feed + ticker | Duvara asılı ekran |
| `focus` | Tek büyük sinyal + kanıt + küçük harita | İnceleme |
| `geo` | Büyük harita + yan feed | Coğrafi izleme |
| `ops` | Kaynak sağlık + gecikme + sistem + feed | Operasyon |
| `research` | Timeline + kanıt + ham veri | Delil zinciri |
| `custom` | Kullanıcının kaydettiği | — |

---

## 8. Kontrol odası (ayar panelleri)

Kullanıcı: *"ayarlamaları sağ yandan açılan paneller yapabiliriz, soldan da
olabilir."* Karar: **sağdan açılan tek kontrol odası**, sekmeli:

- **SAHNE** — sahne seç, preset seç, otomatik geçişi aç/kapat
- **BÖLGE** — bölge ekle/çıkar/taşı, blok tipini değiştir, boyutlandır
- **VERİ** — lens seç, kaynak filtrele, zaman aralığı
- **SİSTEM** — bağlantı, gecikme, dil, anahtar

Panel **yayın ekranının üstünde** açılır, ekranı bozmaz. Kapalıyken tamamen
görünmez. Klavye ile `K` / `Esc`.

---

## 9. Görsel dil

Referans paleti temel alınır (`--bg-space #050811`, siyan `#38bdf8`, kritik
`#ef4444`, uyarı `#f59e0b`, normal `#10b981`), ince grid dokusu, cam panel,
tabular mono sayılar. **Tüm site** bu dile geçer (kullanıcı isteği), ADR ile
kayda geçer.

Erişilebilirlik korunur: kontrast AA, `prefers-reduced-motion`, `aria-live`,
klavye ile tam gezinme, renk körü kontrolü (renk + ikon + şekil + metin).

---

## 10. Drill-down korunuyor

Kullanıcı ham veriye kadar inebilmeli (projenin 2. başarı ölçütü). Zincir korunur,
ama artık **sahne içinde**:

```
SIGNAL → EVENT → OBSERVATION → SOURCE → RAW DATA
```

Her adım bir blok: `signal-card` → `event-timeline` → `evidence-list` →
`source-health` → `raw-viewer`. Aynı ekranda sağda kanıt, solda ham veri
gösterilebilir. URL `#/studio/kanit?signal=sig_...` gibi derin bağlantı verir.

---

## 11. Dosya yapısı (yeni)

```
web/
├── index.html          yayın kabuğu (topbar yok)
├── styles.css          çekirdek + tema
├── studio.css          sahne/bölge/blok stilleri
└── js/
    ├── main.js         giriş noktası
    ├── studio.js       sahne motoru (yerleşim, geçiş, take)
    ├── registry.js     blok kayıt defteri
    ├── data.js         api(), SSE, önbellek, lens
    ├── fmt.js          biçimleyiciler (saf)
    ├── i18n.js         çeviriler
    ├── theme.js        palet/tipografi değişkenleri
    ├── controls.js     kontrol odası paneli
    ├── scenes/         varsayılan sahne JSON'ları
    └── blocks/
        ├── category-strip.js
        ├── globe.js
        ├── map.js
        ├── signal-feed.js
        ├── signal-card.js
        ├── metric.js
        ├── activity-chart.js
        ├── timeline.js
        ├── evidence-list.js
        ├── source-health.js
        ├── raw-viewer.js
        ├── ticker.js
        └── breaking.js
```

**Build adımı yok** (ADR 0026 korunur): tarayıcının yerel ES modülleri
(`<script type="module">`) yeterli. Modülerlik gelir, npm gelmez.

---

## 12. Fazlar (her faz ayrı commit + doğrulama)

| Faz | İş | Çıktı |
|---|---|---|
| **B0** | İskele: `studio.js`, `registry.js`, `data.js`, boş sahne, tema | Ekranda boş ama kaymayan yayın kabuğu |
| **B1** | Blok çekirdeği + `metric`, `category-strip`, `ticker` | İlk gerçek bölgeler |
| **B2** | `signal-feed` + `globe` + SSE ile canlı güncelleme | Canlı yayın hissi |
| **B3** | Sahne motoru: kanal menüsü, geçiş animasyonu, `take` kuralları | TV gibi sahne değişimi |
| **B4** | `breaking` takeover (eski flash'ın yerine) | Flaş geliyor, ekran devralınıyor |
| **B5** | Kontrol odası: bölge ekle/çıkar/taşı, preset, kaydet | Düzen kullanıcı elinde |
| **B6** | Görsel drill-down: `evidence-list`, `timeline`, `source-health`, `raw-viewer` | Ham veriye iniş |
| **B7** | `map` (gerçek koordinatlar) + `activity-chart` | Coğrafi + tarihsel görünüm |
| **B8** | Eski sayfaların kaldırılması, tüm site teması, ADR | Temizlik |
| **B9** | Erişilebilirlik + performans + tam doğrulama | Yayına hazır |

Her faz sonunda §13'teki doğrulama koşar ve Türkçe rapor verilir.

---

## 13. Doğrulama (her fazda)

- `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --workspace`
- Tarayıcı ölçümü: 1920×1080 / 1600×900 / 1366×768 / 1280×720 → **sayfa kaymıyor**
  (`scrollHeight == viewport`, `scrollWidth == viewport`)
- Her sahne × her preset × TR/EN → `undefined`/`NaN` yok
- Bölge içeriği sığmadığında: sıkışma yok, dönüş/kaydırma çalışıyor
- Flaş: SSE olayı gelince ekran devralınıyor, kapatılabiliyor, otomatik kapanıyor
- Drill-down: sinyal → ham veri zinciri tıklanabilir
- Uydurma veri yok: boş alanlar `—` + gerekçe

---

## 14. Riskler ve dürüst sınırlar

1. **Harita verisi seyrek.** Şu an yalnız bazı gözlemlerde koordinat var
   (AFAD deprem: enlem/boylam dolu; NOAA xray: boş). Harita bloğu gerçek
   noktaları çizer; sinyal seviyesinde konum yoksa **"konum yok"** yazar,
   uydurma nokta koymaz. Bu, haritanın zaman zaman boş görünmesi demek — doğru
   davranış, ama beklenti yönetilmeli.
2. **`take` kuralları yanlış giderse.** Otomatik sahne geçişi rahatsız edici
   olabilir. Varsayılan: **kapalı**. Kullanıcı açar. Her kural ayrı kapatılabilir.
3. **`area` ile serbest yerleşim.** Kullanıcı üst üste binen bölge tanımlarsa
   çakışma olur. Motor çakışmayı tespit edip uyarır ve son geçerli düzene döner.
4. **Eski URL'ler.** `#/world`, `#/signal/:id` vb. yönlendirilir; kırılmaz.
5. **Tek büyük yeniden yazım riski.** Fazlara bölündü; her fazda çalışan bir
   ekran var. Geri dönüş noktası her commit.

---

## 15. Onay bekleyen sorular

1. **Model:** "sahne × bölge × blok" + JSON düzen + blok kayıt defteri doğru
   yaklaşım mı? (Öneri: evet — "istediğimiz zaman bölge ekle/çıkar" isteğinin
   tek teknik karşılığı bu.)
2. **Kontrol odası:** sağdan açılan tek panel, sekmeli (SAHNE/BÖLGE/VERİ/SİSTEM).
   Onay?
3. **Flaş:** mevcut flash kartı **tamamen kaldırılıp** yerine ekranı devralan
   `breaking` takeover geliyor. Onay?
4. **Otomatik geçiş varsayılanı:** kapalı açsın, kullanıcı isterse açsın.
   Onay? (Öneri: kapalı — sürpriz olmasın.)
5. **Build adımı:** yerel ES modülleri, npm yok (ADR 0026 korunur). Onay?
   Aksi istenirse Vite eklenir ama bu kabul edilmiş bir ADR'ı değiştirir.
6. **Kapsam:** eski sayfalar (`#/world`, `#/system`, `#/sources`, `#/lenses`,
   `#/map`) **kaldırılıp** bloklara mı dönüşsün, yoksa bir süre ikisi birlikte mi
   yaşasın? (Öneri: B8'e kadar birlikte, sonra kaldır.)
7. **Faz sayısı:** 10 faz (B0–B9) uygun mu, yoksa daha kaba mı bölünsün?

Onay sonrası **B0** ile başlanır; her faz ayrı commit ve Türkçe raporla teslim
edilir.
