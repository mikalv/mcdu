# TODOs — kjente bugs og mangler

Funnet ved kodegjennomgang av alle fire crates (v0.5.0). Sortert etter alvorlighet.
Referanser er `fil:linje` i nåværende `main`.

## 🔴 Kritisk — feil sletting / datatap

- [x] **Sletting av enkeltfil i browseren fungerer ikke, men UI sier suksess.**
  `delete::delete_directory()` antar katalog: WalkDir gir bare fila selv (hoppes over som "root"),
  og fase 4 kjører `fs::remove_dir()` på en fil → feiler. Feilen legges i `errors`, men
  `app.rs` sender `Complete` uansett og `remove_entry_from_tree()` fjerner fila fra treet —
  fila ligger igjen på disk mens UI viser "✓ Deleted 0 files".
  (`crates/mcdu-tui/src/delete.rs:133`, `crates/mcdu-tui/src/app.rs:944-968`)

- [x] **Tom seleksjon i cleanup = "slett alt".** `cleanup_selection()` returnerer ALLE kandidater
  når `cleanup_selected` er tom, så guarden "No cleanup items selected" i `start_cleanup_delete()`
  trigges aldri så lenge det finnes kandidater. Å trykke `d` uten å ha valgt noe sletter alt som ble funnet.
  (`crates/mcdu-tui/src/app.rs:924-934`)

- [x] **Symlinker knekker sletting.** WalkDir med `follow_links(false)` gir symlink-metadata:
  `is_file() == false` for symlinker → `fs::remove_dir()` på symlink feiler → forelder-katalogen
  kan heller ikke fjernes → kaskade av feil og halvslettede trær (typisk i `node_modules/.bin`).
  Samme klasse feil i `executor::execute()`: `path.is_file()` *følger* symlinken, og en dangling
  symlink havner i `remove_dir_all()`-grenen som feiler. Bruk `symlink_metadata()` + `remove_file()` for symlinker.
  (`crates/mcdu-tui/src/delete.rs:100`, `crates/mcdu-core/src/executor.rs:50-54`)

- [x] **Quarantine-systemet er aldri koblet til.** `mcdu_core::quarantine` (531 linjer, med TTL,
  restore, purge) brukes ikke noe sted — all sletting er permanent via `executor`/`delete`.
  Samtidig har cleanup-UI en Quarantine-fane som hardkoder "No quarantined items" og lover
  "can be restored within the retention period" med taster (r/p) som ikke finnes i input-handleren.
  Enten koble på quarantine i slette-flyten, eller fjern fanen og løftet.
  (`crates/mcdu-core/src/quarantine.rs`, `crates/mcdu-tui/src/ui.rs:791-816`, `crates/mcdu/src/main.rs:194-236`)

- [x] **`git gc --prune=now` kjøres alltid, uten samtykke.** `handle_cleanup_final_confirm()` kaller
  `execute_async(pending, /* run_git = */ true, git_roots, …)`. `--prune=now` fjerner unreferenced
  objects umiddelbart (reflog-redning umulig), git_roots er foreldrekatalogene til alle kandidater,
  og `find_git_repos` traverserer dem rekursivt uten pruning (kan finne hundrevis av repos).
  Exit-status fra git ignoreres også (`.map(|_| ())`). Bør være opt-in, uten `--prune=now`, og feil bør rapporteres.
  (`crates/mcdu-tui/src/app.rs:888`, `crates/mcdu-core/src/git.rs:40-49`)

- [x] **`risky = true` i defaults.toml har ingen effekt.** Feltet parses (`rules.rs:33`), men
  propageres aldri til `Candidate` og sjekkes aldri i scanner/UI/executor. Regler markert risky
  (Xcode DerivedData, iOS-simulatorer, Trash …) behandles identisk med trygge regler og
  auto-selekteres (`default_selected: true`). `cleanup_command` er nå wired: kjører shell-kommando
  (med `{path}`/`{dir}`) i stedet for quarantine/sletting når feltet er satt.
  (`crates/mcdu-core/src/rules.rs:33,55`, `crates/mcdu-core/src/defaults.toml:1034+`)

- [x] **Quarantine: partial failure gir usporbart datatap.** I `quarantine()` flyttes filer én og én
  inn i batch-katalogen; manifest.json skrives først til slutt. Feiler flytt nr. N (f.eks. cross-device
  copy av dangling symlink → `?` returnerer), ligger item 0..N-1 i quarantine uten manifest — usynlig
  for `list()`/`restore()`/`current_size()`. Tilsvarende i `restore()`: feiler et element midtveis
  (`PathExists`), er tidligere elementer allerede flyttet ut mens manifestet står urørt → retry feiler
  alltid på item 0 og batchen kan aldri gjenopprettes. Skriv manifest først / oppdater inkrementelt.
  (`crates/mcdu-core/src/quarantine.rs:208-233, 271-297`)

- [x] **`copy_dir_all` (cross-device-fallback) mister symlinker og metadata.** `fs::copy` på symlink
  kopierer target-innholdet (eller feiler på dangling), hardlinks dupliseres, katalog-permissions
  bevares ikke. En "restore" gir altså ikke tilbake det som ble slettet.
  (`crates/mcdu-core/src/quarantine.rs:388-403`)

## 🟠 Alvorlig — krasj og feil tall

- [x] **Panic på ikke-ASCII filnavn (UTF-8 byte-slicing).** `&entry.name[..entry.name.len().min(25)]`
  slicer på byte-grense — filnavn med æ/ø/å/emoji innenfor de første 25 tegnene gir panic midt i render.
  Samme mønster i `draw_loading()` på path (`&path[path.len().saturating_sub(max_width - 3)..]`),
  som i tillegg underflower når terminalen er < 13 kolonner (`max_width - 3` på usize).
  Bruk `chars()`/`unicode-width`-basert trunkering.
  (`crates/mcdu-tui/src/ui.rs:218`, `crates/mcdu-tui/src/ui.rs:957-960`)

- [x] **Ingen panic-hook → ødelagt terminal ved krasj.** Raw mode disables bare på normal exit.
  Med panic-bugene over etterlates brukerens terminal i raw mode uten cursor. Appen bruker heller
  ikke alternate screen (`EnterAlternateScreen`) — `terminal.clear()` sletter scrollback i stedet.
  (`crates/mcdu/src/main.rs:59-88`)

- [x] **`replace_subtree()` oppdaterer størrelser feil oppover i treet.** Løkka legger diffen på
  noden *før* den går ned: root får diffen to ganger (linje 321 + første iterasjon), og den nærmeste
  forelderen (noden som faktisk inneholder barnet) får den aldri. Etter `r` (rescan subtree) viser
  hele stien oppover feil størrelser. Sammenlign med `remove_entry_from_tree()` som gjør det riktig
  (gå ned først, så oppdater).
  (`crates/mcdu-tui/src/app.rs:318-328` vs `:356-366`)

- [x] **`nav_stack` er indeksbasert + re-sortering = feil katalog / mulig panic.**
  `replace_subtree()` re-sorterer `children` etter innsetting; `nav_stack`-indeksene peker da på
  andre barn enn før. Navigering etter subtree-rescan kan hoppe til feil katalog, og
  `&mut node.children[idx]`-indeksering i både `replace_subtree` og `remove_entry_from_tree` kan
  panikke hvis treet har endret seg (barn slettet) siden indeksene ble lagret. Vurder path-basert navigasjon.
  (`crates/mcdu-tui/src/app.rs:292-329, 332-368`)

- [x] **`refresh()`/`start_scan()` blokkerer UI-tråden.** Kansellering finnes ikke: `thread.join()`
  på forrige scan-tråd venter til den er *helt ferdig* (kan være minutter på store trær) mens UI fryser.
  Scan-tråder bør ha et cancel-flagg (AtomicBool) de sjekker per entry.
  (`crates/mcdu-tui/src/app.rs:371-376`)

- [x] **Key events filtreres ikke på `KeyEventKind::Press`.** På Windows og terminaler med kitty
  keyboard protocol sendes både Press og Release → hver tastetrykk håndteres dobbelt
  (dobbel navigasjon, modaler som bekreftes umiddelbart).
  (`crates/mcdu/src/main.rs:121`)

- [x] **Konstant ~60 fps redraw ved idle.** Event-loopen tegner hver iterasjon og poller 16 ms —
  mcdu bruker merkbar CPU selv når ingenting skjer. Tegn kun ved events/state-endringer, eller øk poll-intervallet når idle.
  (`crates/mcdu/src/main.rs:115-127`)

## 🟠 Cleanup-UI — funksjonelle hull

- [x] **Files-fanen er død.** `cleanup_files_selected` og `cleanup_files_scroll` settes aldri av noen
  taster (j/k flytter `cleanup_selected_index` som hører til Categories-radene), `s`-tasten for
  sortering som annonseres i tittelen er ikke implementert i `handle_cleanup_input()`, og Space i
  Files-fanen kaller `toggle_cleanup_selection()` som slår opp i `cleanup_rows()`
  (Categories-strukturen) → toggler et *annet, usynlig* element enn det brukeren tror.
  (`crates/mcdu/src/main.rs:194-236`, `crates/mcdu-tui/src/ui.rs:697-789`, `crates/mcdu-tui/src/app.rs:719-750`)

- [x] **Categories-fanen: viewport-matte antar 1 linje per rad, men kandidater tegner 2 linjer**
  (path + regel-linje). Med mange kandidater havner markøren under synlig område og scrolling blir feil.
  (`crates/mcdu-tui/src/ui.rs:606-694`)

- [x] **Etter sletting med implisitt "alle" vises slettede elementer fortsatt.**
  `update_cleanup_delete()` fjerner kandidater med `retain(!cleanup_selected.contains(...))` —
  men når slettingen skjedde via tom-seleksjon-fallbacken er `cleanup_selected` tom, så ingenting
  fjernes fra lista. Feilede slettinger fjernes omvendt *også* fra lista selv om de fortsatt finnes på disk.
  Bygg heller lista på nytt fra hva som faktisk ble slettet (executor bør returnere per-path-resultat).
  (`crates/mcdu-tui/src/app.rs:820-824`)

- [x] **Feildetaljer forsvinner.** Både cleanup ("Cleanup completed with N errors") og browser-delete
  viser bare antall; hvilke paths som feilet og hvorfor finnes kun i loggfila. Vis dem i UI (modal/liste).
  (`crates/mcdu-tui/src/app.rs:812-816`)

- [x] **`update_cleanup_delete`: join-feil (panic i tråden) håndteres ikke** — `cleanup_delete_rx`
  nullstilles ikke og ingen notification settes; progress-overlayet blir hengende.
  (`crates/mcdu-tui/src/app.rs:801-834`)

- [x] **`mcdu cleanup` starter også full tre-scan av cwd.** `App::new()` kaller alltid `start_scan()`,
  og splash-logikken i `ui::draw()` returnerer tidlig så lenge `is_scanning` — så cleanup-visningen
  skjules bak splash til en irrelevant scan av cwd er ferdig. Skill cleanup-modus fra browser-init.
  (`crates/mcdu-tui/src/app.rs:162`, `crates/mcdu-tui/src/ui.rs:15-33`, `crates/mcdu/src/main.rs:67-81`)

## 🟡 Scanner — korrekthet

- [x] **Filer dedupliseres ikke på tvers av regler.** `matched_dirs` gjelder bare kataloger; en fil som
  matcher to regler (fullt mulig med flere `pattern = "**/*"`-regler mot overlappende paths) blir to
  kandidater → dobbelttelling i totals og dobbel sletting/feil. Testen `avoids_duplicate_matches`
  tester kun kataloger.
  (`crates/mcdu-core/src/scanner.rs:409-447`)

- [x] **`scan_paths`-filteret slipper gjennom hele base_path.** Betingelsen
  `p.starts_with(&base_path)` gjør at en regel med `path = "${HOME}"` skanner hele hjemmekatalogen
  selv om scan_path er `~/Repos` — kandidater havner utenfor stien brukeren ba om
  (rammer særlig `mcdu cleanup <path>` som overstyrer scan_paths).
  Walken må begrenses til snittet av base_path og scan_paths.
  (`crates/mcdu-core/src/scanner.rs:194-200`)

- [x] **`Rule::matches()` bruker template-path, ikke command-resolved path.** Scanner kaller
  `resolve_base_path()` (som kan komme fra `command = "..."`), men `matches()` regner relativ path og
  signature-sjekk mot `base_path()` (templaten). For command-baserte regler feiler `strip_prefix`
  og glob matches mot absolutt path i stedet.
  (`crates/mcdu-core/src/rules.rs:190-208` vs `:122-140`)

- [x] **`min_size_bytes` er meningsløs for kataloger.** `matches()` sammenligner `metadata.len()`
  (inode-størrelse, typisk 64B–4KB) — en katalog-regel med `min_size_bytes = 100MB` matcher aldri
  (eller alt, avhengig av fs). Den rekursive størrelsen beregnes først *etter* matching, og
  re-sjekkes bare i project_marker-stien.
  (`crates/mcdu-core/src/rules.rs:218-222`, `crates/mcdu-core/src/scanner.rs:296-301`)

- [x] **Inkonsistent størrelsesberegning mellom moduler.** Fire kopier av `disk_usage`/`dir_size`:
  scanner-`dir_size` mangler `same_file_system(true)` (teller mounts, i motsetning til tree/delete),
  orphans-`dir_size` bruker `metadata.len()` for filer i én gren, hardlinks dobbelttelles overalt
  (ncdu dedupliserer på inode/nlink), symlinker telles som 0. Browser og cleanup kan vise ulik
  størrelse for samme katalog. Samle i én funksjon i mcdu-core + inode-dedup.
  (`scanner.rs:64-84`, `tree.rs:28-37`, `delete.rs:10-19`, `orphans.rs:142-160`)

- [x] **`is_active` (48t-heuristikken) ser bare på toppkatalogens mtime** — mtime endres kun når
  direkte barn endres, så en `target/` med fersk build dypere ned regnes som inaktiv. Feltet vises
  dessuten ikke i UI-et i dag.
  (`crates/mcdu-core/src/scanner.rs:303-309`)

- [x] **Ytelse: `matched_dirs`-sjekken er O(n·m).** `matched_dirs.iter().any(|d| path.starts_with(d))`
  per entry blir kvadratisk med mange treff (typisk mange `node_modules`). Prefiks-trie eller
  sortert Vec + binærsøk, eller prune walken med `filter_entry`. Også: glob-`Pattern::new(marker)`
  rekompileres per katalog i `find_project_roots`, og `Pattern::new` per exclude per fil i `is_excluded`.
  (`crates/mcdu-core/src/scanner.rs:366-369, 121-135`, `crates/mcdu-core/src/rules.rs:143-160`)

- [x] **`resolve_base_path` kjører vilkårlige shell-kommandoer fra config.** Bevisst feature for
  `command`-regler, men verdt en sikkerhetsnote: en delt/kopiert `cleanup.toml` er dermed kjørbar kode.
  Bør dokumenteres, og evt. begrenses.
  (`crates/mcdu-core/src/rules.rs:122-140`)

## 🟡 Config og persistens

- [x] **Brukerregler kan ikke overstyre eller deaktivere default-regler.** `load_config` gjør
  `extend()` — en bruker som definerer `name = "node-modules"` med `enabled = false` får i stedet
  to regler der defaulten fortsatt er aktiv. Merge på navn (bruker vinner) + en måte å skru av defaults.
  (`crates/mcdu-core/src/config.rs:72-74`)

- [x] **`save_state` er ikke atomisk** (`File::create` + write → krasj gir trunkert/korrupt state).
  Skriv til tempfil + rename. Samme for quarantine-manifest og fingerprint-save.
  (`crates/mcdu-core/src/config.rs:95-107`)

- [x] **Inkonsistente datakataloger.** Config/state bruker `dirs::config_dir()` (`~/Library/Application
  Support` på macOS), mens logger og fingerprints hardkoder `$HOME/.mcdu/...`, og quarantine får
  `base_dir` utenfra. Velg én konvensjon.
  (`crates/mcdu-tui/src/logger.rs:39-42`, `crates/mcdu-tui/src/changes.rs:162-173`)

- [x] **`cleanup-state.toml` lagrer absolutte paths som "selected"** og re-selekterer dem ved neste
  scan — men etter at kandidatene er slettet er lista meningsløs, og `dismissed` skrives alltid tom
  (dismiss-funksjonalitet finnes ikke). Vurder om state-fila i det hele tatt gir verdi i nåværende form.
  (`crates/mcdu-tui/src/app.rs:629-662`)

## 🟡 macOS orphans

- [x] **Spotlight av / mdfind feiler → alt ser ut som orphans.** `get_installed_bundle_ids()`
  returnerer stille tom set ved feil; da er nesten alle apper "ikke installert". Reddes delvis av
  `default_selected(false)`, men lista blir skremmende og ubrukelig. Avbryt orphan-scan (med feilmelding)
  hvis mdfind gir 0 treff.
  (`crates/mcdu-macos/src/installed.rs:5-31`)

- [x] **Én `defaults read`-prosess per installert app** — med mange apper blir orphan-scan treg
  (hundrevis av prosess-spawns). Les Info.plist direkte med en plist-crate, eller bruk
  `mdls -name kMDItemCFBundleIdentifier` i batch.
  (`crates/mcdu-macos/src/installed.rs:34-54`)

- [x] **Orphan-heuristikken er skjør.** `strip_service_suffix` tar bare ett suffiks-nivå
  (`com.foo.app.helper.agent` → matcher ikke), CLI-verktøy uten .app-bundle (homebrew-tjenester m.m.)
  har legitim Library-data men flagges som orphans. Vurder whitelist/kjente prefikser.
  (`crates/mcdu-macos/src/installed.rs:88-96`, `crates/mcdu-macos/src/orphans.rs`)

## 🔵 Dead code / arkitektur-opprydding

- [x] **Tre hele moduler i mcdu-tui er ubrukt:** `cache.rs` (SizeCache), `scan.rs` (scan_directory)
  og `changes.rs` (DirectoryFingerprint) refereres ikke fra app/ui/main. Fjern eller ta i bruk.
  (Merk: `changes.rs`-formatet knekker uansett på filnavn med `:`.)

- [x] **Duplisert cleanup-state:** `mcdu_core::cleanup_ui::CleanupViewState` og
  `mcdu_tui::cleanup_ui::CleanupViewState` er nesten identiske, og *ingen av dem* brukes av appen —
  `App` har sin egen tredje variant med løse `cleanup_*`-felter. Konsolider til én.
  (`crates/mcdu-core/src/cleanup_ui.rs`, `crates/mcdu-tui/src/cleanup_ui.rs`, `crates/mcdu-tui/src/app.rs:57-74`)

- [x] **`parallel.rs` (552 linjer) er ubrukt** — eksportert fra lib.rs men ingen kall; drar inn
  `rayon`, `num_cpus` og `jwalk` som avhengigheter for død kode. Ta i bruk (scanneren er i dag
  single-threaded og treg) eller fjern.
  (`crates/mcdu-core/src/parallel.rs`, `crates/mcdu-core/Cargo.toml`)

- [x] **`executor.rs` og `delete.rs` er to parallelle slette-implementasjoner** med ulik semantikk
  (executor: `remove_dir_all` uten same_file_system-vern og uten per-fil-progress; delete: manuell
  walk med vern). Cleanup-sletting via executor kan dermed krysse inn i mounts som browser-sletting
  beskytter mot. Samle til én slette-motor i core.

- [x] **To `format_size` + hardkodede enheter** (`ui.rs:1019`, `modal.rs:157`, `cleanup_ui.rs:377`),
  og `create_bar` bruker hardkodet 100 GB som maks i stedet for størst-i-katalogen slik ncdu gjør
  (alle barer er nesten tomme i vanlige kataloger). (`crates/mcdu-tui/src/ui.rs:1041-1046, 187-191`)

## 🔵 UX-småplukk

- [x] Esc quitter hele appen fra browseren — lett å trykke av gammel vane; bør kreve `q` (eller bekreftelse).
  (`crates/mcdu/src/main.rs:169-175`)
- [x] Hjelpeskjermen nevner ikke cleanup-tastene (Tab/1-4/Space/a/n/d/D) i det hele tatt.
  (`crates/mcdu-tui/src/ui.rs:1067-1145`)
- [x] `%`-kolonnen i browseren regnes mot synlig katalogsum, "..%"-raden inkluderes ikke — greit,
  men prosent + bar bruker to ulike skalaer (total vs. 100 GB), forvirrende.
- [x] Notification-overlay dekker midten av skjermen i 3 s og blokkerer visuelt; en statuslinje er mindre invasiv.
- [x] Permission-feil under scan ignoreres stille (`filter_map(|e| e.ok())` overalt) — ncdu viser en
  markør når en katalog ikke kunne leses; her ser tallene bare "riktige" ut.
- [x] `disk_space`-prosent deler på `total_bytes` uten null-sjekk (NaN → 0 ved cast, ufarlig men stygt).
  (`crates/mcdu-tui/src/ui.rs:114`)

## Testgap (verdt å dekke når bugs fikses)

- [x] Slette enkeltfil via browser-flyten (avdekker kritisk bug #1).
- [x] Sletting av tre som inneholder symlinker (dangling + til katalog).
- [x] Cleanup-delete med tom seleksjon (avdekker "slett alt"-fallbacken).
- [x] `replace_subtree` størrelses-propagering (avdekker dobbel-diff-buggen).
- [x] Render med ikke-ASCII filnavn > 25 tegn (avdekker panic).
- [x] Quarantine med cross-device + symlink + partial failure.
