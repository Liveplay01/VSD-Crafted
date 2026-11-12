# n1deck – schlanker Ersatz für VSD Craft (VSD / Basicolor N1)

Rust-Dienst mit Tray-Icon (kein Chromium, kein Node) plus kleine Config-GUI (`n1deck-config`).
Protokoll: Bibliothek `mirajazz`.

## Bauen (Windows)
Variante A – GitHub: Ordner als Repo pushen, unter *Actions → build-windows* die `.exe`s als Artefakt laden.
Variante B – lokal: Rust (rustup) + Visual-Studio-Build-Tools, dann `cargo build --release`.
Ergebnis: `target\release\n1deck.exe` und `n1deck-config.exe` (im selben Ordner lassen).

## Einrichten (einmalig, VSD Craft vorher beenden!)
1. `n1deck list`   – N1 in der Liste suchen (VID:PID). Weicht sie von 6603:1000 / 5548:1038 / 5548:1032 ab,
   in `%APPDATA%\n1deck\config.toml` unter `[device]` eintragen (Dezimalwerte).
2. `n1deck probe`  – Tasten drücken, es müssen Codes erscheinen. Wenn nicht: Ausgabe melden.
3. `n1deck learn`  – Tasten zeigen Nummern, der Reihe nach drücken; dann Regler klicken, rechts, links.
4. `n1deck`        – Dienst starten (Tray-Icon). `n1deck autostart on` für Start mit Windows.
5. Rechtsklick Tray → „Konfiguration öffnen“, Tasten belegen, Speichern (wird automatisch übernommen).

## Aktionen
Öffnen (Datei/Ordner/URL), Programm starten, Tastenkombination, Medien/Lautstärke, Seitenwechsel,
OBS (obs-websocket v5; in OBS: Werkzeuge → WebSocket-Servereinstellungen, Passwort in der Config).
Nicht übernehmbar: die originalen VSD-Craft-/Stream-Deck-Plugins (Discord usw.).

## Stand
Am echten N1 geprüft (Windows 11, Rust 1.95): Build, 5 Unit-Tests, `list`/`probe`/`learn`, Tasten und Regler (Drehen) funktionieren.
- Gerät: **5548:1002** („HOTSPOTEKUSB HID DEMO“, usage 65440/1).
- Eingabe: Tastendruck `state=1`, Loslassen `state=0`. Tastencodes 1–15 (Tasten 0–14), 30 und 31 (kleine Tasten 15 und 16); Taste 17 ist noch nicht zugeordnet.
- Regler drehen: Code 51 = rechts, 50 = links, jeweils `state=0`.
- Das N1 trennt die Verbindung nach ~30 s ohne `keep_alive` – `probe` und `learn` senden es deshalb ebenfalls.
- Regler-Klick: Code 35 (`state` 1/0). Drehen: Code 51 (rechts) / 50 (links), `state=0`.
- `N1DECK_DEBUG=1` gibt in `probe` jedes Rohpaket aus.
- Weiter ungeprüft: Tray, `autostart`, Hotkey-/Medien-Aktionen im Detail, OBS.

## Config-GUI

`n1deck-config` hat ein helles Theme, Live-Vorschau der Tastenbilder und Icon-Auswahl (Bild wird nach `%APPDATA%\n1deck\icons` kopiert). Aktionen: Öffnen, Programm, Hotkey (mit Discord-Vorlagen), Medien, Seitenwechsel, OBS, **Mehrere nacheinander** und **Zirkel** (jeder Druck löst den nächsten Schritt aus; Zählerstand gilt pro Seite und Taste bis zum Neustart des Dienstes).

Discord: es gibt keine Discord-RPC-Anbindung. Die Vorlagen senden Hotkeys, die in Discord unter Einstellungen → Tastenkombinationen als globale Shortcuts eingetragen werden müssen.

### Icon-Bibliothek und Bildformate

Im Tasten-Editor: **Bild wählen…** importiert png, jpg, webp, gif, bmp, ico und svg. **Bibliothek…** sucht online in Iconify (api.iconify.design, weiß eingefärbt). Beim Auswählen wird das Icon heruntergeladen. Alles landet als 144-px-PNG in `%APPDATA%\n1deck\icons`. Grenze: Text in SVGs wird nicht gerendert.

### Druck-Effekt und Ladeanimation

Im Tasten-Editor unter **Feedback**: Druck-Effekt (Keiner, Aufhellen, Abdunkeln, Aufblitzen) wird kurz auf dem Tasten-Display gezeigt. Bei Programm-/Öffnen-Aktionen (.exe, .lnk, .bat, .cmd, .url) läuft eine Ladeanimation, bis ein neuer Prozess erscheint (bei bekannter .exe: bis genau diese läuft), maximal 15 s. Läuft die Exe schon, endet sie nach ca. 1,5 s. Das Raster im Editor ist 3 breit, 5 tief.

## Auto-Seite pro Programm

In der GUI oben bei „Auto bei Programm“ den Prozessnamen eintragen (z. B. `ArcRaiders.exe`, `BeamNG.drive.x64.exe`; mehrere mit Komma; Namen im Task-Manager unter „Details“). Läuft das Programm, wechselt der Dienst (Prüfung alle 2 s) auf diese Seite, nach dem Beenden zurück zur Startseite. Manuelles Seitenwechseln bleibt möglich, bis sich der Programmstatus ändert.
