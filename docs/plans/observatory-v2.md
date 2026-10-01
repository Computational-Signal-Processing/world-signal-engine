# Gözlemevi v2 — "canlı yayın" düzeni

Durum: öneri (onay bekliyor)
Tarih: 2026-10-01
İlgili: `docs/decisions/0026-observatory-ui.md`

## 1. Referanslar

Kullanıcı üç görsel/referans verdi:

- **Görsel A** — "KOZMİK SİNYAL MERKEZİ / GLOBAL EVENT MONITOR" (ekran görüntüsü).
- **Görsel B** — bizim şu anki gözlemevi.
- **`gemini-code-1790858891725.html`** ve **`gemini-code-1790859484787.html`** —
  Gemini'nin aynı düzeni kodladığı iki HTML. İkisi de aynı yapı:

```
header   logo + rozet | nav sekmeleri | ● CANLI YAYIN + UTC saat
────────────────────────────────────────────────────────────────
şerit    5 kategori kartı: başlık + rozet(ANOMALİ/UYARI/NORMAL)
         + büyük sayı + "▲ +53% baseline sapması"
────────────────────────────────────────────────────────────────
sol      Küresel Aktivite Yoğunluğu (grafik) + Sinyal Metrikleri
orta     Küresel Sinyal Haritası (küre) + ACİL DURUM flash kartı
sağ      GÜNCEL SİNYAL AKIŞI (seviye rozetli satırlar)
────────────────────────────────────────────────────────────────
alt      SON DAKİKA + kayan şerit
```

Palet: zemin `#050811`–`#06090e`, panel `#0f172a`/`rgba(13,22,40,.65)`,
siyan vurgu `#38bdf8`, kritik `#ef4444`, uyarı `#f59e0b`, normal `#10b981`.

İstenen: *"mevcut UI'mizi tam sağlam güncelleyelim, gerçek zamanlı bir TV gibi."*

## 2. Kritik ayrım: düzeni al, uydurma veriyi alma

Gemini çıktılarındaki **her sayı uydurma**:

```
79 +53%   110   420 -11%   612   560   +2.8σ   %85 güven
12 Aktif   3s 15d   +98% above baseline   4 Bağımsız İstasyon
Norveç / NOAA · 10sn önce   SOHO / NASA   Global CISA
```

Bunların hiçbiri bir kaynaktan gelmiyor; mockup için yazılmış. Ayrıca
Gemini `setInterval` ile **sahte feed satırı üretiyor** ve her 8 sn'de ekrana
sahte "KRİTİK" kart ekliyor. Projenin kuralı (`docs/philosophy.md`,
`AGENTS.md`) bunu yasaklıyor:

> Her sayı gerçek bir ölçüm olmalı. Uydurma "risk skoru" / "endeks" eklenmez.
> "Veri yok" ile "sıfır" ayrıdır ve ayrı gösterilir.

**Plan: düzeni ve görsel dili birebir hedefliyoruz; sayıların tamamı gerçek
`/observatory` verisinden geliyor.** Hedefteki `+53%` bizde gerçek
`change_pct`; yoksa `—` / "veri yok" yazar. Sahte satır üretilmez.

## 3. Bugünkü hâlimiz ile hedef arasındaki fark

| Hedefte | Bizde | Yapılacak |
|---|---|---|
| Şerit = **kategori kartları** (büyük sayı + rozet + delta) | Ayrı bir metrik şeridi **ve** ayrı kategori kartları var | İki şerit birleşir: tek şerit, kart = kategori. Sayı = son değer, alt satır = `change_pct` + `deviation_sigma` |
| Rozet: `ANOMALİ / UYARI / NORMAL` | `LOW/MEDIUM/CRITICAL` metni | Rozet görselleşir; **metin kalır** (renk tek taşıyıcı değil) |
| Sol: grafik + "Sinyal Metrikleri" | Aktivite grafiği + kategori listesi | Sol = aktivite grafiği + seçili sinyalin metrikleri |
| Orta: küre + **satır içi** flash kartı | Küre + ayrı modal | Flash kartı kürenin altına **satır içi** gelir; modal yalnız FLAŞ için |
| Sağ: seviye rozetli feed | Var, rozet zayıf | Sol kenar renk şeridi + rozet + göreli zaman |
| Nabız atan `● CANLI YAYIN` | Sade | Nabız rozeti; `prefers-reduced-motion` saygılı |
| Lacivert-siyan + grid dokusu | Nötr gri | `--obs-*` palet katmanı, ince grid, hafif glow |

Alınmayacak: `AY ÜSSÜ / MARS` gibi karşılığı olmayan sekmeler (bizim gerçek
rotalarımız kalır), uydurma yüzdeler, sahte feed üretimi.

## 4. Uygulama adımları

Her adım ayrı commit; her adımda tarayıcı doğrulaması + `cargo test`.

### Adım 1 — Palet ve tipografi (yalnız CSS)
`--obs-*` değişkenleri, `body.obs-active` zemin + grid dokusu, tabular mono
sayılar, büyük ölçek. Kontrast AA; renk körü kontrolü.

### Adım 2 — Şerit: kategori kartları
`obsCategories()` yeniden: kategori adı + tip rozeti / büyük değer + birim /
büyük `change_pct` + `±σ` / sparkline. Boş kategori gerekçesini yazar.
Ayrı metrik şeridi kaldırılır (verisi şeride taşınır).

### Adım 3 — Gövde: üç sütun
- Sol: aktivite grafiği (mevcut) + `obsSignalMetrics()` (seçili sinyalin
  `deviation_sigma`, `duration`, `confidence`, kanıt/kaynak sayısı).
- Orta: küre (büyütülür) + `obsFlashPanel()` — seçili sinyalin satır içi
  künyesi + sparkline + "TAM RAPOR" → `#/signal/:id`.
- Sağ: feed (rozet + kenar şeridi güçlendirilir).

### Adım 4 — Header ve ticker görsel dili
Nabız atan CANLI rozeti, UTC saat, marka rozeti; ticker'a tip rozeti.
Klavye erişilebilirliği ve `aria-label` tamamlanır.

### Adım 5 — Doğrulama
`cargo test --workspace`, `clippy`, `fmt`; 1920×1080 ve 1366×768'de tek
ekran, kaydırma yok; TR + EN; `?broadcast=1` filigranı korunur.
ADR 0026 güncellenir.

## 5. Değişecek dosyalar

```
web/styles.css     palet, grid dokusu, şerit, sütunlar, feed, ticker
web/app.js         obsCategories, obsBody, obsSignalMetrics (yeni),
                   obsFlashPanel (yeni), obsFeedRow, obsTickerBar, obsGlobe
docs/decisions/0026-observatory-ui.md   güncelleme
```

Backend değişikliği **gerekmiyor**: şerit, küre, feed, seçili sinyal künyesi
mevcut `/observatory` alanlarından besleniyor.

## 6. Onay bekleyen sorular

1. **Düzen:** yukarıdaki "canlı yayın" düzeni onaylanıyor mu?
2. **Sayılar:** dürüst yol (her sayı gerçek) — onay? Aksi istenirse mockup
   sayıları kullanılır ama felsefe ihlali olur ve ADR gerekir.
3. **Palet kapsamı:** lacivert-siyan yalnız gözlemevi mi, tüm site mi?
   (Öneri: önce yalnız gözlemevi.)
