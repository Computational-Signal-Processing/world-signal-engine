# Gözlemevi v2 — hedef tasarıma yaklaştırma planı

Durum: öneri (onay bekliyor)
Tarih: 2026-10-01
İlgili: `docs/decisions/0026-observatory-ui.md`

## 1. Ne istendi

Kullanıcı iki görsel gönderdi:

- **Görsel A — hedef ("KOZMİK SİNYAL MERKEZİ / GLOBAL EVENT MONITOR")**: lacivert-siyan
  zemin, üstte logo + yatay menü, altında **tam genişlikte bir sinyal şeridi**
  (her kategori bir kart: `EARTHQUAKES / ANOMALY / 79 / +53% / TOP PRIORITY · HIGH`),
  gövdede **üç sütun** (küre grafiği · "ACİL DURUM" flash kartı · "CANLI YAYIN
  MODU" olay listesi), altta **BREAKING ticker**.
- **Görsel B — bizim şu anki hâlimiz**: aynı bilgi mimarisi, ama şerit küçük
  kartlar hâlinde tek satır, orta sütunlar sığmıyor, tipografi seyrek.

İstek: *"tamamiyle buna benzetemez miyiz, bu ayarda bizimkisi çok garip olmuş."*

## 2. Önce bir uyarı: hedef görselin içeriği uydurma

OCR ile çıkardığım hedef metinlerde şunlar var:

```
EARTHQUAKES 79 +53%    SPACEWEATHER 110 -12%   GEOMAGNETIC 420 -11%
CYBER 420 -12%         GLOBALNEWS 560 -52%     SEVİYE: YÜKSEK (MEDIUM-HIGH)
SAPMA: +98%   SÜRE: 3s 15d   GÜVEN: %85        30 EVENTS
California, USA +72% above baseline             CVE ACTIVITY INCREASED CRITICAL
DDoS Attack Pattern — South America             Aftershock Cluster — California
```

Bu sayıların hiçbiri bir kaynaktan gelmiyor. `+53%`, `+98%`, `%85 güven`,
`3s 15d`, `30 EVENTS` — hepsi tasarım mockup'ı için uydurulmuş. Bizim
`docs/philosophy.md` ve `AGENTS.md`'de yazılı kural bu:

> Her sayı gerçek bir ölçüm olmalı. Uydurma "risk skoru" / "endeks" eklenmez.
> "Veri yok" ile "sıfır" ayrı şeylerdir ve ayrı gösterilir.

Hedef görseli **birebir** kopyalarsak, sistemin en gürültülü yüzeyinde
uydurma sayılar gösteririz. Bu, projenin varlık sebebine aykırı.

Bu yüzden plan iki parçalı: **görsel dili al, uydurma veriyi alma.**

## 3. Hedefin gerçekten alınacak yönleri (veri uydurmadan)

| # | Hedefte olan | Bizde eksik olan | Nasıl yapılır (gerçek veriyle) |
|---|---|---|---|
| 1 | Sinyal şeridi tam genişlikte, büyük sayı + büyük yüzde | Kartlar küçük, şerit sıkışık | Şerit yeniden: kart başına büyük sayı, büyük `change_pct`, üstte kategori adı + tip rozeti, altta tek satır gerekçe. Zaten var olan `/observatory` kart alanları yeterli |
| 2 | Gövde üç sütun, küre solda büyük | Orta sütunlar sığmıyor, küre küçük | `obs-body` grid'i `minmax(0,1.6fr) minmax(0,1fr) minmax(0,1.2fr)` yap; küre viewBox'ını büyüt |
| 3 | Orta sütunda "flash" kartı (sarımsı, çerçeveli) | Bizde sinyal detayı yok | **`FeedItem`'ın tamamı zaten var.** Şeritten seçili sinyal orta sütunda: başlık, `deviation_sigma`, `duration`, `confidence`, kanıt sayısı, sparkline, "TAM RAPOR" düğmesi |
| 4 | Sağ sütun "CANLI YAYIN MODU" olay listesi, önem rozetli satırlar | Bizde liste var ama rozet zayıf | `obs-feed-row`'a seviye rozeti (`data-sev`) büyütülür, `+%` sapma satır içinde gösterilir |
| 5 | Lacivert-siyan palet, glow, tarama çizgileri | Paletimiz nötr gri | Yeni bir `--obs-*` palet katmanı: zemin `#0b1220`, kart `#111a2e`, çizgi `#1e2a44`, vurgu siyan `#38bdf8`, uyarı amber `#f0b429`, kritik `#ef4444` |
| 6 | Üstte logo + menü, sağda CANLI + saat | Var ama sade | Rozet + nabız animasyonu; `prefers-reduced-motion` saygılı |
| 7 | Altta BREAKING ticker | Var | Tip rozetleri + önem rengi, aynı kural: renk tek taşıyıcı değil |

Alınmayacak: `SEVİYE: YÜKSEK (MEDIUM-HIGH)` gibi bizde karşılığı olmayan
kategoriler, `30 EVENTS` gibi sabit sayaçlar, "güven %85" gibi türetilmemiş
yüzdeler.

## 4. Karar noktası (önce bunu netleştirelim)

Hedef görselin "uydurma sayı" kısmı için iki yol var:

**Yol 1 — Sadık ama dürüst (önerilen).** Yerleşim ve görsel dil birebir
hedeflenir; her sayı gerçek ölçümden gelir. Hedefte `+53%` yazan yerde biz
`+53%` yazarız — **ama gerçekten %53 ise**. Değilse gerçek değer yazar.
Boş kategori "veri yok" der. Sonuç hedefe çok benzer, ama savunulabilir.

**Yol 2 — Birebir kopya.** Mockup'taki gibi sabit/örnek sayılar ve
"MEDIUM-HIGH" gibi türetilmemiş etiketler eklenir. Görsel olarak en yakın
sonuç, ama `philosophy.md` ihlali olur ve ADR gerekir.

Plan Yol 1'e göre yazıldı. Yol 2 isteniyorsa söyleyin, ADR'yi ona göre yazarım.

## 5. Uygulama adımları

Her adım ayrı commit; her adımda tarayıcıda doğrulama + `cargo test`.

### Adım 1 — Palet ve tipografi katmanı (yalnız CSS)
- `web/styles.css`: `--obs-*` değişkenleri, `body.obs-active` altında zemin.
- Tipografi: sayılar için tabular mono, büyük ölçek (`--obs-num: 34px`).
- Kabul: kontrast AA; renk körü testi (renk tek başına anlam taşımıyor).

### Adım 2 — Sinyal şeridi (tam genişlik, büyük sayılar)
- `obsCategories()` yeniden yazılır: kart = kategori adı + tip rozeti /
  büyük değer + birim / büyük `change_pct` / tek satır gerekçe / sparkline.
- Şerit tek satır yerine hedefteki gibi kart dizisi; yatay taşma yerine
  grid + `minmax`.
- Kabul: 13 kategori tek ekranda, kaydırma yok; boş kart gerekçesini yazar.

### Adım 3 — Üç sütunlu gövde + orta "flash" kartı
- `obsBody()`: sol küre (büyütülür), orta `obsSelectedPanel(data)`, sağ feed.
- Yeni `obsSelectedPanel`: seçili sinyalin (varsayılan: en yüksek seviyeli)
  tam künyesi + sparkline + "TAM RAPOR" → `#/signal/:id`.
- Şeritten karta tıklama bu paneli besler (rota değişmez).
- Kabul: sinyal yoksa panel "aktif sinyal yok" der; uydurma alan yok.

### Adım 4 — Feed ve ticker görsel dili
- `obsFeedRow`: seviye rozeti + sapma; `obsTickerBar`: tip rozeti + önem.
- Kabul: klavye ile gezilebilir; `aria-label`'lar tam.

### Adım 5 — Hareket ve durum
- Canlı rozeti nabız, veri tazeliği uyarısı, `prefers-reduced-motion`.
- Kabul: `?broadcast=1` filigranı ve tazelik hâlâ görünür.

### Adım 6 — Doğrulama
- `cargo test --workspace`, `cargo clippy`, `fmt`.
- Tarayıcı: 1920×1080 ve 1366×768'de tek ekran, kaydırma yok.
- Türkçe ve İngilizce.
- ADR 0026 güncellenir (yerleşim + palet kararı).

## 6. Değişecek dosyalar

```
web/styles.css     palet, yerleşim, kart, feed, ticker
web/app.js         obsCategories, obsBody, obsSelectedPanel (yeni),
                   obsFeedRow, obsTickerBar, obsGlobe (ölçek)
docs/decisions/0026-observatory-ui.md   güncelleme
```

Backend'de değişiklik **gerekmiyor**: şerit, küre, feed ve seçili sinyal
künyesi mevcut `/observatory` alanlarından besleniyor.

## 7. Açık sorular

1. Yol 1 (dürüst) mi, Yol 2 (birebir kopya) mı?
2. Üst menüdeki "AY ÜSSÜ / MARS / KAYNAKLAR / SİSTEM" gibi hedef-özel
   sekmeler bizde yok; bizim gerçek rotalarımız kalsın mı? (Öneri: kalsın.)
3. Palet: mevcut nötr griden lacivert-siyana geçiş tüm siteyi mi kapsasın,
   yalnız gözlemevi mi? (Öneri: önce yalnız gözlemevi.)
