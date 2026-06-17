# Changelog

## [0.8.5] - 2026-06-17

### Fixed

- **Server: ghost session on WiFi → cellular reconnect (0 RX for 5–10 s)** — `cleanup_old_sessions_for_vpn_ip` was called with the new session's VPN IP; when the client reconnects from a different source IP (cellular vs WiFi) the old session still owns the same VPN IP but was never removed, leaving the server routing downlink to the dead WiFi address for up to 300 s; new `cleanup_old_sessions_for_client_id` removes stale sessions by PSK identity immediately on successful re-handshake
- **Server: tag_map visibility gap in counter recovery** — `recover_session_by_tag` used `DashMap::retain()` to update the tag map, briefly removing ALL tags for a session before re-inserting new ones; concurrent packets during this window saw no matching tag and triggered unnecessary handshakes or were dropped; fixed to targeted per-tag removal that never leaves a gap
- **Server: redundant tag_map refresh after PFS ratchet and inline rekey** — `complete_session_ratchet()` and `commit_session_rekey()` already update the tag map internally; the extra `refresh_session_tags()` calls after each caused double-writes and extra lock contention; removed
- **Server: double mutex acquisition in KeyRotate handler** — `session_id` and `has_pending` were fetched in two separate `session.lock()` calls; merged into a single critical section
- **Android: zombie coroutine kills new session via `stopSelf()`** — when `AivpnJni.runTunnel()` did not exit within the 3 s `cancelAndJoin` timeout the old `serviceJob` continued running; when it eventually exited its `finally{}` block checked `manualDisconnect` (already reset to `false` by the new `startVpn()`) and called `stopSelf()`, killing the freshly started session; `sessionId` is now captured at launch time and compared in `finally{}` — stale jobs skip `stopSelf()`
- **Android: `serviceJob` not `@Volatile`** — `serviceJob` was written from `restartJob` on `Dispatchers.IO` and read from `stopVpn()` on the main thread without a JVM visibility guarantee; added `@Volatile`
- **macOS: disconnect callback clobbers new session state** — `VPNManager.disconnect()` fires `sendToHelper` asynchronously; if the user pressed Connect before the callback returned, the callback unconditionally reset `isConnecting` and `isConnected` to `false`, leaving the UI showing Disconnected while the tunnel was actively connecting; a `connectGeneration` counter is now captured before the async call and compared inside the callback — stale callbacks skip the state reset
- **Android: `++sessionId` placed after `cancelAndJoin` — guard fires on every reconnect** — in the initial 0.8.5 implementation `val capturedSessionId = ++sessionId` was placed *after* `withTimeoutOrNull(3_000L) { serviceJob?.cancelAndJoin() }`; when the old `serviceJob`'s `finally{}` block fired during cancellation `sessionId` had not yet been incremented, so `mySessionId == sessionId` was always `true` and `stopSelf()` killed the service on every reconnect trigger (network switch, periodic rekey), causing 0 RX on cellular and a broken disconnect button; `++sessionId` is now incremented *before* `cancelAndJoin()`
- **Server: ghost session lingers for 5 minutes when Shutdown is lost** — `IDLE_TIMEOUT` was 300 s; if the client's Shutdown UDP packet was dropped by a mobile network (CGNAT, MTS) the server held the stale session for 5 minutes, blocking reconnect downlink until the ghost expired; reduced to 30 s so self-healing is fast enough to be invisible to the user
- **Android: single Shutdown packet easily lost on CGNAT links** — the Rust core sent `ControlPayload::Shutdown` exactly once before closing; on lossy CGNAT paths (MTS) this single UDP send was frequently dropped, leaving a ghost session on the server; Shutdown is now retransmitted 3× with 50 ms intervals to reduce loss probability
- **Android/iOS: 0 RX on reconnect with port-preserving CGNAT (MTS)** — on carriers that reuse the same external UDP port for reconnects (MTS CGNAT port preservation), the CGNAT's inbound routing table still pointed to the old (closed) internal port, silently dropping all server downlink until the entry expired (5–30 s); the Rust core now records the local port via `getsockname()` after each successful connect and tries to `bind()` to the same port on the next reconnect — when it succeeds the CGNAT mapping needs no update and downlink works immediately; falls back to OS-assigned ephemeral port if the saved port is unavailable
- **Android/iOS: CGNAT warmup fallback — 4 keepalives after handshake** — as a second line of defence (for carriers that delay updating the inbound CGNAT entry even after port reuse), the client now sends 4 additional keepalive packets at 100 ms intervals immediately after the handshake; each outbound packet nudges the CGNAT to refresh the inbound routing entry for the new socket
- **iOS: Shutdown packet not sent on disconnect** — the iOS Rust core closed the UDP socket without sending `ControlPayload::Shutdown`; the server kept the ghost session for up to 30 s, causing 0 RX on reconnect; Shutdown is now sent 3× with 50 ms intervals (matching the Android fix already in 0.8.5)
- **iOS: handshake retry rotates keypair on every attempt** — the iOS retry loop regenerated the X25519 keypair on every 750 ms retry, creating up to 13 server ghost sessions per 10 s timeout; on reconnect this easily hit the per-IP session limit (5) on CGNAT networks; keypair is now rotated only once (at the 2nd retry, ~1.5 s), limiting ghost sessions to 2 maximum — matching the fix already in 0.8.3 for Android
- **CLI/Linux/macOS/Windows: 0 RX on reconnect with port-preserving CGNAT** — the same CGNAT port reuse fix applied to Android/iOS is now applied to the desktop client (`AivpnClient`): the local UDP port is saved after each successful connect and reused on the next bind; 4 post-handshake warmup keepalives (100 ms apart) are sent after `ServerHello` as a fallback for carriers that delay inbound mapping updates

---

## [0.8.5] — 2026-06-17

### Исправлено

- **Сервер: фантомная сессия при переключении WiFi→сотовая сеть (0 RX 5–10 с)** — `cleanup_old_sessions_for_vpn_ip` вызывалась с VPN IP новой сессии; при переподключении клиента с другого IP (сотовая vs WiFi) старая сессия со своим VPN IP не удалялась, и сервер продолжал слать даунлинк на мёртвый WiFi-адрес до 300 с; новая функция `cleanup_old_sessions_for_client_id` удаляет устаревшие сессии по PSK-идентификатору сразу после успешного повторного рукопожатия
- **Сервер: разрыв видимости в tag_map при восстановлении счётчика** — `recover_session_by_tag` использовал `DashMap::retain()` для обновления карты тегов, на мгновение удаляя ВСЕ теги сессии перед вставкой новых; параллельные пакеты в этот момент не находили тег и вызывали лишние рукопожатия или дропались; исправлено точечным удалением конкретных тегов без разрыва видимости
- **Сервер: избыточное обновление tag_map после PFS-рачета и inline rekey** — `complete_session_ratchet()` и `commit_session_rekey()` уже обновляют tag_map внутри себя; лишние вызовы `refresh_session_tags()` после каждого создавали двойные записи и лишние блокировки; удалены
- **Сервер: двойной захват мьютекса в обработчике KeyRotate** — `session_id` и `has_pending` считывались в двух отдельных вызовах `session.lock()`; объединено в одну критическую секцию
- **Android: зомби-корутина убивала новую сессию через `stopSelf()`** — если `AivpnJni.runTunnel()` не завершался в течение 3 с таймаута `cancelAndJoin`, старый `serviceJob` продолжал работу; когда он завершался, его блок `finally{}` проверял `manualDisconnect` (уже сброшен в `false` новым `startVpn()`) и вызывал `stopSelf()`, убивая только что запущенную сессию; `sessionId` теперь фиксируется при запуске и сравнивается в `finally{}` — устаревшие задачи пропускают `stopSelf()`
- **Android: `serviceJob` без аннотации `@Volatile`** — `serviceJob` записывался в `restartJob` на `Dispatchers.IO` и читался в `stopVpn()` из главного потока без гарантии видимости JVM; добавлено `@Volatile`
- **macOS: колбэк disconnect затирал состояние новой сессии** — `VPNManager.disconnect()` вызывает `sendToHelper` асинхронно; если пользователь нажимал Connect до возврата колбэка, тот безусловно сбрасывал `isConnecting` и `isConnected` в `false`, показывая UI «Отключено» пока тоннель уже подключался; счётчик `connectGeneration` теперь фиксируется до асинхронного вызова и сравнивается внутри колбэка — устаревшие колбэки пропускают сброс состояния
- **Android: `++sessionId` стоял после `cancelAndJoin` — guard срабатывал при каждом переподключении** — в исходной реализации 0.8.5 `val capturedSessionId = ++sessionId` располагался *после* `withTimeoutOrNull(3_000L) { serviceJob?.cancelAndJoin() }`; когда блок `finally{}` старого `serviceJob` срабатывал во время отмены, `sessionId` ещё не был увеличен, поэтому `mySessionId == sessionId` всегда был истинным и `stopSelf()` убивал сервис при каждом триггере переподключения (смена сети, периодический rekey), вызывая 0 RX на сотовой сети и зависание кнопки отключения; `++sessionId` теперь вызывается *до* `cancelAndJoin()`
- **Сервер: фантомная сессия живёт 5 минут при потере Shutdown** — `IDLE_TIMEOUT` был равен 300 с; если UDP-пакет Shutdown клиента дропался мобильной сетью (CGNAT, МТС), сервер удерживал устаревшую сессию 5 минут, блокируя даунлинк при переподключении до истечения призрака; уменьшено до 30 с, чтобы самовосстановление было незаметным для пользователя
- **Android: одиночный Shutdown-пакет легко теряется на CGNAT-линках** — ядро на Rust отправляло `ControlPayload::Shutdown` ровно один раз перед закрытием; на ненадёжных CGNAT-путях (МТС) этот единственный UDP-send часто дропался, оставляя фантомную сессию на сервере; Shutdown теперь ретранслируется 3× с интервалом 50 мс для снижения вероятности потери
- **Android/iOS: 0 RX при переподключении с port-preserving CGNAT (МТС)** — у операторов с сохранением внешнего UDP-порта при переподключении (CGNAT МТС) таблица маршрутизации входящего трафика CGNAT продолжала указывать на старый (закрытый) внутренний порт и молча дропала весь даунлинк с сервера до истечения записи (5–30 с); ядро Rust теперь сохраняет локальный порт через `getsockname()` после каждого успешного подключения и пытается сделать `bind()` на тот же порт при следующем переподключении — если это удаётся, CGNAT-маппинг не требует обновления и даунлинк работает сразу; при недоступности сохранённого порта откатывается на назначаемый ОС эфемерный порт
- **Android/iOS: warmup-фоллбэк для CGNAT — 4 keepalive после рукопожатия** — как вторая линия защиты (для операторов, задерживающих обновление входящей записи CGNAT даже после переиспользования порта) клиент теперь отправляет 4 дополнительных keepalive-пакета с интервалом 100 мс сразу после рукопожатия; каждый исходящий пакет побуждает CGNAT обновить маршрутизацию входящего трафика для нового сокета
- **iOS: пакет Shutdown не отправлялся при отключении** — iOS-ядро Rust закрывало UDP-сокет без отправки `ControlPayload::Shutdown`; сервер удерживал фантомную сессию до 30 с, вызывая 0 RX при переподключении; Shutdown теперь отправляется 3× с интервалом 50 мс (аналогично исправлению Android из 0.8.5)
- **iOS: retry рукопожатия ротировал ключи при каждой попытке** — цикл повторных попыток iOS регенерировал X25519-ключи при каждом retry через 750 мс, создавая до 13 фантомных сессий за 10 с таймаута; при переподключении это легко достигало лимита сессий на IP (5) в CGNAT-сетях; ключи теперь ротируются только один раз (при 2-й попытке, ~1,5 с), ограничивая число фантомных сессий двумя — аналогично исправлению Android из 0.8.3
- **CLI/Linux/macOS/Windows: 0 RX при переподключении с port-preserving CGNAT** — тот же фикс переиспользования UDP-порта, что применён к Android/iOS, теперь применён к десктопному клиенту (`AivpnClient`): локальный порт сохраняется после каждого успешного подключения и переиспользуется при следующем bind; 4 warmup keepalive (по 100 мс) отправляются после `ServerHello` как фоллбэк для операторов, задерживающих обновление inbound-маппинга

---

## [0.8.4] - 2026-06-17

### Fixed

- **Android/iOS disconnect leaves ghost session on server** — the Android and iOS native cores closed the UDP socket without sending `ControlPayload::Shutdown` to the server; the server kept the session alive for 30 s (idle timeout), creating a ghost session window during reconnect where incoming packets could match the stale session's tag and fail decryption — causing the VPN to appear hung and the disconnect button to appear broken on the second connection; both cores now send `Shutdown { reason: 0 }` before closing the socket, matching the behaviour already present in the CLI/macOS/Windows client

### Changed

- Version bumped 0.8.3 → 0.8.4 across workspace `Cargo.toml`, all crate `Cargo.toml` files, macOS `Info.plist`, iOS `App/Info.plist` and `Tunnel/Info.plist`, iOS/macOS version strings

---

## [0.8.4] — 2026-06-17

### Исправлено

- **Android/iOS: фантомная сессия на сервере после отключения** — нативные ядра Android и iOS закрывали UDP-сокет без отправки `ControlPayload::Shutdown` серверу; сервер удерживал сессию ещё 30 с (idle timeout), создавая окно, в котором при повторном подключении входящие пакеты могли попасть в устаревшую сессию с ошибкой расшифровки — VPN зависал, а кнопка отключения переставала работать со второго раза; оба ядра теперь отправляют `Shutdown { reason: 0 }` перед закрытием сокета, как это уже делают CLI/macOS/Windows-клиенты

### Изменено

- Версия поднята с 0.8.3 до 0.8.4 во всём workspace: `Cargo.toml`, все crate-файлы, macOS `Info.plist`, iOS `App/Info.plist` и `Tunnel/Info.plist`, строки версий iOS/macOS

---

## [0.8.3] - 2026-06-16

### Fixed

- **Android jitter on initial connect** — `onLost` network callback was triggering a tunnel restart during the handshake phase, causing rapid reconnect loops ("connecting → reconnecting × 3 → connected" within 2 s); fixed by guarding the abort path with `sessionEstablished`
- **Android disconnect button broken after 2nd connection** — race window between `clearPendingStop()` and the new `serviceJob` launch allowed `stopVpn()` to fire into a null reference; a second `manualDisconnect` check inside the lifecycle mutex closes the window
- **Android buffer size too small** — `BUF_SIZE` raised from 1500 to 2048 bytes in the JNI tunnel to prevent silent packet truncation when MDH headers push total frame size past 1500 bytes
- **Android ghost sessions on CGNAT** — handshake retry logic rotated the X25519 keypair on every 750 ms retry, creating up to 13 server-side sessions per timeout and triggering the per-IP session cap (5) on CGNAT networks (MTS, Megafon); keypair is now rotated only once, at the 2nd retry, limiting ghost sessions to 2 maximum
- **Android poisoned mutex silent no-op** — `ACTIVE_SESSION.lock()` used `.ok()` in the stop and cleanup paths; if the mutex was poisoned the stop signal was silently discarded; changed to `unwrap_or_else(|e| e.into_inner())` so the stop always propagates
- **Android JNI exception not cleared after `protect()` failure** — a pending JNI exception from `VpnService.protect()` was left on the thread, potentially causing unpredictable JVM behavior on subsequent JNI calls; `exception_clear()` is now called before returning an error
- **Android network transport change ignored during post-connect cooldown** — the 15 s cooldown that suppresses network-ID reshuffles also blocked detection of real WiFi→cellular switches, leaving the tunnel bound to the dead interface until the 20 s RX watchdog fired; `isTransportChange()` helper now distinguishes ID reshuffle from transport change and triggers immediate reconnect on the latter
- **Android `START_STICKY` null intent creates zombie service** — when the OS restarts the service after a kill with a null intent, the service now calls `stopSelf()` if no active session was in progress, preventing a foreground service with no tunnel
- **Android traffic callbacks fire after disconnect** — `statsJob` was launched on `serviceScope`, surviving a tunnel exit; changed to use `coroutineScope {}` inside `runTunnel()` so the poll loop is cancelled as soon as the tunnel returns
- **Server counter-drift recovery CPU DoS** — `recover_session_by_tag` searched up to 65536 counter values per session per unrecognised packet (196k BLAKE3 ops per session under 3 time windows); reduced to 1024, sufficient for real drift recovery while eliminating the DoS amplification
- **Server pre-ratchet anti-replay bitmap collision** — `mark_pre_ratchet_received` and the replay check used `counter.min(255)` as the bitmap index, collapsing all counters ≥255 into bit 255; fixed to `counter % TAG_WINDOW_SIZE` which gives each counter in a 256-entry window a unique bit, eliminating both false replay drops and replay acceptance for high counters
- **Server iptables FORWARD rule leaked on restart** — the `Drop` impl deleted the `RELATED,ESTABLISHED` FORWARD rule using `-m state --state` while it was added with `-m conntrack --ctstate`; the mismatched specifier meant `iptables -D` never matched the live rule, accumulating duplicate rules across restarts; both paths now use `-m conntrack --ctstate`
- **Server entropy computed for every packet** — `compute_entropy` (O(payload)) and an `Instant::elapsed()` call ran on every inbound packet even though the neural model only samples every 16th packet; both are now inside the `counter & 0x0f == 0` gate, reducing CPU overhead by 15/16

### Removed

- **Android dead code** — `bindSocketToNetwork()` (JNI method never called from Rust after network binding approach was dropped) and `isVpnNetwork()` (local helper with no remaining callers) removed from `AivpnService`

### Changed

- **Android port validation** — `parseServerAddr()` now validates the parsed port is in range 1–65535 before accepting it; out-of-range values fall back to the default port 443
- Version bumped 0.8.2 → 0.8.3 across workspace `Cargo.toml`, all crate `Cargo.toml` files, macOS `Info.plist`, iOS `App/Info.plist` and `Tunnel/Info.plist`, iOS/macOS version strings

---

## [0.8.3] — 2026-06-16

### Исправлено

- **Дёргание соединения Android при первом подключении** — колбэк `onLost` запускал перезапуск тоннеля в фазе рукопожатия, вызывая быстрые циклы переподключения («подключение → переподключение × 3 → есть связь» за 2 секунды); исправлено добавлением проверки `sessionEstablished` в ветку прерывания
- **Кнопка отключения Android не работала после 2-го подключения** — гонка между `clearPendingStop()` и запуском нового `serviceJob` позволяла `stopVpn()` отработать по нулевой ссылке; вторая проверка `manualDisconnect` внутри мьютекса жизненного цикла закрывает это окно
- **Маленький буфер Android** — `BUF_SIZE` увеличен с 1500 до 2048 байт в JNI-тоннеле во избежание тихого обрезания пакетов, когда MDH-заголовки увеличивают кадр свыше 1500 байт
- **Фантомные сессии Android на CGNAT** — логика повтора рукопожатия ротировала X25519-ключи при каждой попытке через 750 мс, создавая до 13 серверных сессий за таймаут и срабатывая по лимиту сессий на IP (5) в сетях CGNAT (МТС, Мегафон); ключи теперь ротируются один раз — при 2-й попытке, что ограничивает число фантомных сессий двумя
- **Тихое игнорирование заблокированного мьютекса Android** — `ACTIVE_SESSION.lock()` использовал `.ok()` в путях остановки и очистки; при захваченном мьютексе сигнал остановки молча терялся; изменено на `unwrap_or_else(|e| e.into_inner())`, чтобы остановка всегда проходила
- **Необработанное JNI-исключение после ошибки `protect()`** — необработанное JNI-исключение от `VpnService.protect()` оставалось в потоке, вызывая непредсказуемое поведение JVM при последующих JNI-вызовах; теперь перед возвратом ошибки вызывается `exception_clear()`
- **Игнорирование смены типа транспорта Android в период cooldown** — 15-секундный cooldown, подавляющий переназначение сетевых ID, блокировал и обнаружение реальных переключений WiFi→LTE, оставляя тоннель привязанным к мёртвому интерфейсу до срабатывания 20-секундного сторожа RX; хелпер `isTransportChange()` теперь отличает смену ID от смены транспорта и инициирует немедленное переподключение при второй
- **Зомби-сервис Android при `START_STICKY` + нулевой интент** — когда ОС перезапускает сервис после принудительного завершения с нулевым интентом, сервис теперь вызывает `stopSelf()`, если активной сессии не было, предотвращая форегрунд-сервис без тоннеля
- **Колбэки трафика Android срабатывали после отключения** — `statsJob` запускался на `serviceScope` и переживал выход тоннеля; заменено на `coroutineScope {}` внутри `runTunnel()`, чтобы цикл опроса отменялся вместе с тоннелем
- **DoS через восстановление счётчика на сервере** — `recover_session_by_tag` перебирал до 65536 значений счётчика на сессию для каждого нераспознанного пакета (196k операций BLAKE3 на сессию в трёх временных окнах); сокращено до 1024, достаточного для реального дрейфа без DoS-усиления
- **Коллизия в bitmap анти-реплея pre-ratchet на сервере** — `mark_pre_ratchet_received` и проверка реплея использовали `counter.min(255)` как индекс бита, сваливая все счётчики ≥255 в бит 255; исправлено на `counter % TAG_WINDOW_SIZE`, дающее уникальный бит для каждого счётчика в окне из 256 значений — устранены и ложные блокировки реплея, и пропуск реальных реплеев для больших счётчиков
- **Утечка iptables-правила FORWARD на сервере** — реализация `Drop` удаляла правило FORWARD `RELATED,ESTABLISHED` с флагом `-m state --state`, тогда как оно добавлялось с `-m conntrack --ctstate`; несоответствие спецификаторов означало, что `iptables -D` никогда не находило правило, и при каждом перезапуске накапливались дубли; оба пути теперь используют `-m conntrack --ctstate`
- **Энтропия пакетов вычислялась для каждого пакета на сервере** — `compute_entropy` (O(payload)) и вызов `Instant::elapsed()` выполнялись для каждого входящего пакета, хотя нейронная модель сэмплирует только каждый 16-й; оба перенесены внутрь ворот `counter & 0x0f == 0`, что снижает нагрузку CPU на hot-path в 16 раз

### Удалено

- **Мёртвый код Android** — `bindSocketToNetwork()` (JNI-метод, не вызываемый из Rust после смены подхода к привязке сокетов) и `isVpnNetwork()` (локальный хелпер без оставшихся вызывателей) удалены из `AivpnService`

### Изменено

- **Валидация порта Android** — `parseServerAddr()` теперь проверяет, что распарсенный порт находится в диапазоне 1–65535; значения вне диапазона откатываются к дефолтному порту 443
- Версия поднята с 0.8.2 до 0.8.3 во всём workspace: `Cargo.toml`, все crate-файлы, macOS `Info.plist`, iOS `App/Info.plist` и `Tunnel/Info.plist`, строки версий iOS/macOS

---

## [0.8.2] - 2026-06-16

### Fixed

- **Adaptive mode was a UI-only no-op on all platforms** — the adaptive toggle saved a preference but nothing read it; adaptive mode now fully changes connection behaviour end-to-end
- **Android adaptive mode**: TUN MTU is lowered to 1200 (from 1346) when adaptive is enabled, reducing fragmentation on restrictive mobile networks (MTS, Megafon); keepalive interval is shortened to 4 s (from 8 s) to prevent silent NAT timeouts on CGNAT cellular with short UDP state windows
- **iOS adaptive mode**: `PacketTunnelProvider` now reads `adaptiveMode` from `providerConfiguration` and sets `NEPacketTunnelNetworkSettings.mtu = 1200` when enabled (was hardcoded 1400 regardless)
- **macOS compile error**: `VPNManager.connect()` was missing the `adaptiveMode: Bool` parameter that `ContentView` already passed, causing a build failure; parameter added
- **macOS helper adaptive passthrough**: `aivpn-helper` now appends `--adaptive` to the `aivpn-client` subprocess arguments when `adaptiveMode` is true; `HelperRequest` struct updated in both the app and the helper daemon
- **CLI adaptive MTU**: `aivpn-client --adaptive` now caps the initial `ClientNetworkConfig.mtu` at 1200, overriding higher values from the connection key; `AdaptiveMonitor` is active and continues step-down under packet loss

### Changed

- **Android adaptive UI**: the adaptive toggle in the options popup is now a checkable menu item with a system checkmark indicator instead of text that switched between "Adaptive: ON" and "Adaptive: OFF"
- Version bumped 0.8.1 → 0.8.2 across workspace `Cargo.toml`, all crate `Cargo.toml` files, macOS `Info.plist`, iOS `App/Info.plist` and `Tunnel/Info.plist`, macOS/iOS version strings, Android `version_footer`

---

## [0.8.2] — 2026-06-16

### Исправлено

- **Адаптивный режим был заглушкой UI на всех платформах** — переключатель сохранял настройку, но нигде она не использовалась; теперь адаптив реально меняет поведение соединения на всех уровнях
- **Android адаптивный режим**: MTU TUN-интерфейса снижается до 1200 (с 1346) при включённом адаптиве — уменьшает фрагментацию в ограничивающих сетях (МТС, Мегафон); keepalive сокращается до 4 с (с 8 с) для предотвращения незаметных тайм-аутов NAT в сотовых CGNAT-сетях с коротким окном UDP-состояния
- **iOS адаптивный режим**: `PacketTunnelProvider` теперь читает `adaptiveMode` из `providerConfiguration` и устанавливает `NEPacketTunnelNetworkSettings.mtu = 1200` при включённом адаптиве (ранее всегда 1400 независимо от настройки)
- **Ошибка компиляции macOS**: `VPNManager.connect()` не принимал параметр `adaptiveMode: Bool`, который `ContentView` уже передавал — добавлен недостающий параметр
- **Передача адаптива в macOS helper**: `aivpn-helper` теперь добавляет `--adaptive` в аргументы subprocess `aivpn-client` при `adaptiveMode = true`; структура `HelperRequest` обновлена в обоих компонентах
- **CLI MTU в адаптивном режиме**: `aivpn-client --adaptive` теперь ограничивает начальный `ClientNetworkConfig.mtu` значением 1200, переопределяя бо́льшие значения из ключа подключения; `AdaptiveMonitor` активен и продолжает снижать MTU при потере пакетов

### Изменено

- **Android UI адаптива**: переключатель адаптивного режима в меню опций теперь является чекбоксом с системной галочкой вместо текста «Adaptive: ON» / «Adaptive: OFF»
- Версия поднята с 0.8.1 до 0.8.2 во всём workspace: `Cargo.toml`, все crate-файлы, macOS `Info.plist`, iOS `App/Info.plist` и `Tunnel/Info.plist`, строки версий Swift, Android `version_footer`

---

## [0.8.1] - 2026-06-16

### Added

- **Subnet split-tunnel on all GUI clients** — users can now specify per-CIDR route exclusions that bypass the VPN tunnel; exclusions are persisted and forwarded to the underlying `aivpn-client` subprocess as `--exclude-route` args (iOS: `SplitTunnelView` + `NEIPv4Settings.excludedRoutes`; macOS: `ContentView` CIDR field + `VPNManager` subprocess passthrough; Windows: egui multiline input + `vpn_manager.rs` subprocess passthrough; Android: DNS-resolved per-domain exclusions via `Builder.excludeRoute(IpPrefix)` on API 33+, graceful skip + warning on older devices)
- **Domain-based split-tunnel on Android** — `AivpnService.applyDomainExclusions()` resolves saved excluded domains at connect time via `InetAddress.getAllByName()` and adds per-IP exclusion routes; includes API level check with user-visible warning on API < 33
- **`--exclude-route` flag in `aivpn-client`** — new `Append` CLI argument for repeatable CIDR subnet exclusions passed through from all GUI clients
- **Kill-switch toggle in Windows GUI** — checkbox wired to `--kill-switch` subprocess argument in `vpn_manager.rs`
- **UAC elevation manifest** — Windows build now embeds `requireAdministrator` execution level in the application manifest via `build.rs`, eliminating silent access-denied failures on first run
- **Adaptive mode forwarded to iOS tunnel extension** — `adaptiveMode` flag is now included in `providerConfiguration` by `VPNManager.connect()` and read inside `PacketTunnelProvider`
- **Recording IPC response in iOS tunnel extension** — `handleAppMessage` now returns `{"canRecord": false}` for `record_start` / `record_stop` / `record_status` requests, preventing the UI from stalling in `.starting` state
- **Audit log wired into gateway** — `AuditLogger` is now passed into `GatewayServer` and records events for: ClientCert accepted/rejected, RecordingStart, RecordingStop, PoolSync rejected

### Security

- **ServerHello signature verification** (`C-CL-1`, CRITICAL) — `aivpn-client` now verifies the ed25519 signature in `ServerHello` against `server_signing_key` before completing the PFS ratchet; a bad signature disconnects immediately, preventing MitM key substitution
- **MaskUpdate signature verification** (`C-CL-2`, CRITICAL) — mask profiles received via `ControlPayload::MaskUpdate` are now verified against the server's signing key before being applied; unsigned or tampered masks are silently ignored
- **BootstrapDescriptorUpdate signature enforcement** (`C-CL-3`, CRITICAL) — `store_verified_descriptor()` is now called with the server's static key as `trusted_key` instead of `None`; descriptors without a valid signature are rejected
- **Bootstrap SSRF guard** (`C-CL-4`) — `bootstrap_loader.rs` validates all URLs fetched from the `bu` field before making HTTP requests; non-HTTPS schemes and private/loopback hosts (127.x, 10.x, 192.168.x, 172.16–31.x, 169.254.x, ::1) are rejected with an error log
- **iOS connection keys moved to Keychain** (`C-I-1`, CRITICAL) — `KeychainStorage` now uses `Security.framework` (`SecItemAdd` / `SecItemCopyMatching` / `SecItemUpdate` / `SecItemDelete`) with `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`; no connection keys or mTLS certs are written to `UserDefaults`
- **macOS helper `binaryPath` restricted to allowlist** (`C-M-1`, CRITICAL) — `aivpn-helper` now rejects any `binaryPath` not in a hardcoded set of canonical paths before calling `posix_spawn`; eliminates local privilege escalation via socket message injection
- **macOS PSK plaintext write removed** (`C-M-2`, CRITICAL) — `VPNManager.saveKey()` no longer writes the connection key to `UserDefaults`; key storage is Keychain-only
- **ChainForward source IP validated** (`C-S-4`) — `gateway.rs` now parses the IPv4 source address from the inner payload and confirms it matches the forwarding session's assigned VPN IP before writing to the TUN channel; IPv6 inner payloads are blocked unconditionally; mismatches are logged and dropped
- **Pre-ratchet anti-replay bitmap** (`C-S-2`) — `Session` gains a `pre_ratchet_bitmap` field that marks consumed pre-ratchet tag counters, preventing replay of packets captured before a key rotation; bitmap is cleared on `complete_ratchet()`
- **PoolSync guard against non-pool sessions** (`C-S-1`) — `is_pool_peer` flag validated before accepting any `PoolSync` message, preventing arbitrary clients from injecting client-DB records

### Fixed

- **`tun_name` shell injection** (`H-S-3`) — `nat.rs` validates the TUN interface name against `^[a-z][a-z0-9_-]{0,14}$` before it is used in any nftables / iptables command; invalid names are rejected with an error before any firewall rule is applied
- **PoolSync VPN IP collision** (`H-S-2`) — `client_db.merge_from_json()` now checks for duplicate VPN IPs before inserting a synced client record; conflicts are logged and the incoming record is skipped
- **`passive_distribution` panics removed** (`H-S-6`) — `encode_for_image()` and `encode_for_blockchain()` no longer call `unimplemented!()`; they emit a `warn!` and return `Err`, allowing the server to continue running
- **ClientCert sent after PFS ratchet** (`H-CL-1`) — `aivpn-client` now queues `ClientCert` inside the `ServerHello` handler after `complete_ratchet()`, ensuring the cert is encrypted with ratcheted session keys
- **MessagePack size limit for bootstrap descriptors** (`H-CL-6`) — `BootstrapDescriptorUpdate` handler rejects payloads larger than 512 KiB before `rmp_serde::from_slice`, preventing OOM from oversized control messages
- **iOS 104-byte mTLS cert check removed** — `PacketTunnelProvider` no longer rejects certs that are not exactly 104 bytes; any non-empty base64-decoded value is accepted
- **iOS `LocalizationManager` crash on iOS 15** — `Locale.current.language.languageCode` gated behind `#available(iOS 16, *)`; falls back to `Locale.current.languageCode`
- **Android `onRevoke()` infinite reconnect** — `AivpnService.onRevoke()` now sets `manualDisconnect = true` before `super.onRevoke()`, preventing the reconnect loop triggered by OS-initiated VPN revocation
- **Android `@Volatile` callback race** — `statusCallback`, `trafficCallback`, and `tileCallback` invocations now capture the reference in a local `val` before the null-check and invoke
- **Android callbacks leaked in `onDestroy`** — `AivpnService.onDestroy()` now nullifies all three callbacks before `super.onDestroy()`
- **Android bench `DatagramSocket` not protected** — the UDP RTT probe socket in `MainActivity` now calls `VpnService.protect()` before sending, preventing a routing loop when VPN is active
- **iOS `syncStatus()` called off main thread** — `VPNManager` wraps `syncStatus()` in `DispatchQueue.main.async` inside the `loadAllFromPreferences` completion handler
- **`current_timestamp_ms()` panic** — `.unwrap()` replaced with `.unwrap_or_default()` in `aivpn-common/src/crypto.rs`
- **`handshake_locks` unbounded growth** — periodic gateway cleanup now prunes entries with `Arc::strong_count == 1`
- **MikroTik container non-functional as gateway** — `entrypoint.sh` rewritten: enables `net.ipv4.ip_forward`, installs idempotent MASQUERADE + FORWARD rules, quotes `AIVPN_KEY`, defaults `AIVPN_FULL_TUNNEL=false`, adds 5-second restart loop; `README.md` / `README_RU.md` / `README_CN.md` updated with `cap=net-admin` in all `/container/add` examples
- **Windows GUI abrupt exit** — `main.rs` no longer calls `std::process::exit(0)`; the tray thread is signalled and joined before the process exits naturally
- **macOS helper `mtlsCertPath` path traversal** — helper now applies an allowlist prefix and extension check before accepting a cert path argument

### Changed

- Version bumped 0.8.0 → 0.8.1 across workspace `Cargo.toml`, all crate `Cargo.toml` files, macOS `Info.plist` (CFBundleVersion 5 → 6), iOS `App/Info.plist` and `Tunnel/Info.plist` (CFBundleVersion 3 → 4), macOS/iOS version strings, Android `version_footer`
- macOS helper now warns when mTLS cert path is configured but proxy mode is active
- Android `SplitTunnelActivity` shows API-level note explaining domain exclusions require Android 10+

---

## [0.8.1] — 2026-06-16

### Добавлено

- **Раздельное туннелирование по подсетям во всех GUI-клиентах** — пользователи могут указывать исключения маршрутов по CIDR, которые обходят VPN-туннель; исключения сохраняются и передаются в subprocess `aivpn-client` через аргументы `--exclude-route` (iOS: `SplitTunnelView` + `NEIPv4Settings.excludedRoutes`; macOS: поле CIDR в `ContentView` + передача через `VPNManager`; Windows: multiline-ввод в egui + `vpn_manager.rs`; Android: DNS-разрешённые исключения через `Builder.excludeRoute(IpPrefix)` на API 33+, graceful fallback с предупреждением на старых версиях)
- **Доменное split-tunnel на Android** — `AivpnService.applyDomainExclusions()` разрешает сохранённые исключённые домены через `InetAddress.getAllByName()` при подключении и добавляет маршруты-исключения для каждого IP; включает проверку версии API с видимым предупреждением при API < 33
- **Флаг `--exclude-route` в `aivpn-client`** — новый аргумент типа `Append` для многократного указания CIDR-подсетей, передаваемых из GUI-клиентов
- **Kill-switch в Windows GUI** — чекбокс подключён к аргументу `--kill-switch` в `vpn_manager.rs`
- **Манифест UAC-повышения привилегий** — сборка Windows теперь встраивает уровень выполнения `requireAdministrator` в манифест приложения через `build.rs`
- **Адаптивный режим передаётся в iOS tunnel extension** — флаг `adaptiveMode` теперь включается в `providerConfiguration` в `VPNManager.connect()` и читается в `PacketTunnelProvider`
- **Recording IPC ответ в iOS tunnel extension** — `handleAppMessage` возвращает `{"canRecord": false}` на запросы `record_start` / `record_stop` / `record_status`, предотвращая зависание UI
- **Аудит-лог подключён к шлюзу** — `AuditLogger` передаётся в `GatewayServer` и фиксирует события: принятие/отклонение ClientCert, RecordingStart, RecordingStop, отклонённый PoolSync

### Безопасность

- **Верификация подписи ServerHello** (`C-CL-1`, КРИТИЧНО) — `aivpn-client` проверяет ed25519-подпись в `ServerHello` по `server_signing_key` перед завершением PFS-рэтчета; неверная подпись разрывает соединение
- **Верификация подписи MaskUpdate** (`C-CL-2`, КРИТИЧНО) — профили масок из `ControlPayload::MaskUpdate` проверяются по ключу подписи сервера; неподписанные маски игнорируются
- **Верификация подписи BootstrapDescriptorUpdate** (`C-CL-3`, КРИТИЧНО) — `store_verified_descriptor()` вызывается со статическим ключом сервера как `trusted_key`; дескрипторы без корректной подписи отклоняются
- **SSRF-защита в bootstrap_loader** (`C-CL-4`) — проверка всех URL из поля `bu`: только HTTPS, блокировка приватных и loopback-адресов
- **Ключи подключения iOS перемещены в Keychain** (`C-I-1`, КРИТИЧНО) — `KeychainStorage` использует `Security.framework` с `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`; ничего не пишется в `UserDefaults`
- **Ограничение `binaryPath` в macOS helper** (`C-M-1`, КРИТИЧНО) — `aivpn-helper` принимает только пути из жёстко заданного allowlist перед `posix_spawn`; устраняет локальное повышение привилегий
- **Удалена запись PSK в открытом виде в macOS** (`C-M-2`, КРИТИЧНО) — `VPNManager.saveKey()` больше не пишет ключ в `UserDefaults`
- **Валидация src IP в ChainForward** (`C-S-4`) — `gateway.rs` проверяет IPv4-адрес источника внутренней нагрузки против VPN IP сессии; несоответствия и IPv6 отбрасываются
- **Bitmap анти-replay для pre-ratchet тегов** (`C-S-2`) — поле `pre_ratchet_bitmap` в `Session` отмечает использованные счётчики тегов; сбрасывается при `complete_ratchet()`
- **Защита PoolSync от не-pool сессий** (`C-S-1`) — флаг `is_pool_peer` проверяется перед принятием любого `PoolSync`

### Исправлено

- **Инъекция через `tun_name`** (`H-S-3`) — валидация по шаблону `^[a-z][a-z0-9_-]{0,14}$` в `nat.rs`
- **Коллизия VPN IP при PoolSync** (`H-S-2`) — `merge_from_json()` проверяет дублирование IP; конфликты пропускаются с предупреждением
- **Паники в `passive_distribution`** (`H-S-6`) — `unimplemented!()` заменены на `Err` + `warn!`
- **ClientCert отправляется после PFS рэтчета** (`H-CL-1`) — сертификат ставится в очередь внутри обработчика `ServerHello` после `complete_ratchet()`
- **Лимит размера MessagePack** (`H-CL-6`) — `BootstrapDescriptorUpdate` отклоняет нагрузки > 512 КиБ
- **Проверка 104 байт mTLS в iOS убрана** — принимается любое непустое base64-значение
- **Краш `LocalizationManager` на iOS 15** — `#available(iOS 16, *)` guard для `Locale.current.language.languageCode`
- **Бесконечный reconnect при `onRevoke()` на Android** — `manualDisconnect = true` + `super.onRevoke()`
- **Гонка `@Volatile` callback на Android** — захват ссылки в локальный `val` перед null-проверкой
- **Утечка callbacks в `onDestroy` на Android** — обнуление всех callback перед `super.onDestroy()`
- **Незащищённый `DatagramSocket` бенчмарка на Android** — вызов `VpnService.protect()` перед отправкой
- **`syncStatus()` вне главного потока на iOS** — оборачивается в `DispatchQueue.main.async`
- **Паника `current_timestamp_ms()`** — `.unwrap()` → `.unwrap_or_default()` в `crypto.rs`
- **Неограниченный рост `handshake_locks`** — периодическая очистка по `Arc::strong_count == 1`
- **Нефункциональный контейнер MikroTik** — `entrypoint.sh` переписан; `cap=net-admin` добавлен в README (EN/RU/CN)
- **Резкое завершение Windows GUI** — graceful shutdown с join tray thread вместо `process::exit(0)`
- **Path traversal `mtlsCertPath` в macOS helper** — allowlist-проверка префикса и расширения

### Изменено

- Версия поднята с 0.8.0 до 0.8.1 во всём workspace: `Cargo.toml`, crate-файлы, macOS `Info.plist` (CFBundleVersion 5 → 6), iOS `Info.plist` (CFBundleVersion 3 → 4), строки версий, Android `version_footer`
- macOS helper предупреждает при активном proxy-режиме и настроенном mTLS-сертификате
- `SplitTunnelActivity` на Android отображает примечание об уровне API для доменных исключений

---

## [0.8.0] - 2026-06-13

### Added

- **Multi-server pool / failover** — `pool` block in `server.json`; nodes share the same X25519 keypair; in-protocol UDP sync over the VPN port (`ControlPayload::PoolSync` 0x12) — sync traffic is indistinguishable from client traffic, no extra port or firewall rule required; all nodes derive identical `SessionKeys` from a shared `sync_key` via blake3 KDF; `aivpn-server enroll <peer>` command for one-shot peer enrollment (`aivpn-server/src/pool_sync.rs`)
- **Client server pool** — failover, round-robin, weighted, and latency-based selection; optional `pool` JSON array in `aivpn://` connection key (backward-compatible — old clients ignore unknown fields) (`aivpn-client/src/server_pool.rs`)
- **OpenWRT native package + LuCI plugin** — `aivpn-openwrt/package/aivpn/` with procd init script, UCI config template, WAN hotplug restart; `luci-app-aivpn` web UI with Status and Configuration tabs; OpenWRT setup guide at `aivpn-openwrt/docs/openwrt-setup.md`
- **Per-client QoS / bandwidth limiting** — eBPF TC egress hook (`ebpf/tc_qos_prog.c`) with LRU_HASH `qos_rules` map, token-bucket rate limiting and DSCP marking per client VPN IP; transparent userspace fallback when BPF absent; `--set-client-qos` CLI flag (`aivpn-server/src/qos.rs`)
- **Backup / migration tools** — `--export <path.tar.gz>` and `--import <path.tar.gz>` with `manifest.json`; covers clients DB, mask files, and server config (`aivpn-server/src/backup.rs`)
- **eBPF observability stub** — XDP/TC ring-buffer stats observer; attaches when `/sys/fs/bpf/aivpn_events` is present; graceful no-op otherwise (`aivpn-server/src/ebpf_observer.rs`)
- **Structured event logging** — `AivpnEvent` enum covering connect/disconnect, key rotation, XDP drops, peer sync, kill-switch; `EventBus` with JSONL stdout sink and optional webhook (`aivpn-common/src/event_log.rs`)
- **Adaptive mode** — 20-entry sliding window tracks per-connection packet loss; auto-adjusts `mtu_delta` (−50 per step, floor 576) and keepalive multiplier; `--adaptive` CLI flag; toggle in all UI clients: Windows egui panel, macOS menu popover, iOS settings row, Android options menu (`aivpn-client/src/adaptive.rs`)
- **Admin audit log** — append-only JSONL at `/var/log/aivpn/audit.log` (configurable via `--audit-log`); records actor, action, target, result, and ISO-8601 timestamp for every management operation (`aivpn-server/src/audit_log.rs`)
- **Benchmarking / Diagnostics** — UDP RTT probes, P50/P95/P99 latency percentiles, throughput up/down, packet loss %, 0–100 quality score; `aivpn-client bench` subcommand; Diagnostics panel in Windows GUI, macOS popover, iOS sheet, Android options-menu dialog (`aivpn-client/src/bench.rs`)
- **eBPF XDP drop statistics** — `xdp_prog.c` now maintains a `BPF_MAP_TYPE_ARRAY` map (`drop_stats`, 4 slots: `TOO_SHORT`, `TAG_EXPIRED`, `RESERVED`, `TOTAL`) and a 256 KB `BPF_MAP_TYPE_RINGBUF` (`events`). All XDP_DROP paths call an inline `record_drop(reason)` helper that atomically increments the counter and emits a ring-buffer event. `ebpf_observer.rs` opens the pinned map via raw `BPF_OBJ_GET` / `BPF_MAP_LOOKUP_ELEM` syscalls (no new crate dependency) and emits delta `XdpDrop` events on the `EventBus` (`aivpn-server/src/ebpf_observer.rs`, `aivpn-linux-kernel/src/xdp_prog.c`)
- **DNS-over-HTTPS proxy** — optional in-server DoH forwarder (`feature = "dns"`); binds UDP :53 on the VPN interface and tunnels queries via RFC 8484 POST to a configurable upstream (default Cloudflare); optional secondary fallback resolver; `block_plain_dns` mode adds an nftables rule dropping UDP/53 to non-VPN interfaces so clients cannot bypass the proxy; config block `"dns"` in `server.json` (`aivpn-server/src/dns_proxy.rs`)
- **Site-to-site VPN** — two or more AIVPN server nodes can connect their local subnets without any VPN client software; peers advertise routes via `ControlPayload::RouteSync` (0x13) using the same blake3 KDF as pool sync; outbound advertisements are sent every 30 s; incoming `RouteSync` is authenticated against the configured peer list (exact `IP:port` match), each received subnet is validated against the peer's `remote_subnets` allowlist, dangerous prefixes (default route, loopback, link-local) are rejected, payload is bounded at 4 KiB / 64 subnets; config block `"site_to_site"` in `server.json` (`aivpn-server/src/site_sync.rs`)
- **Multi-hop chain forwarding** — transparent double-hop routing; the entry node decrypts client IP payloads and re-wraps them in `ControlPayload::ChainForward` (0x14) encrypted with the pool shared key, relaying them to a configured exit node; the exit node injects the inner payload directly into its TUN device and routes to the internet; the client is never aware of the hop; config: `pool.exit_node` on the entry node, `pool.exit_node_enabled: true` on the exit node (defaults to `false` to prevent open relay); `pool.sync_key` must be a valid non-zero 32-byte key or the chain forwarder refuses to start (`aivpn-server/src/chain_forwarder.rs`)
- **mTLS-lite client certificate layer** — optional ed25519-signed client certificates layered over the existing X25519 + PSK handshake; certificate is a compact 104-byte token (`client_pub_key[32] || expiry_ts_le[8] || ca_signature[64]`) sent via `ControlPayload::ClientCert` (0x15); `required: false` (default) accepts PSK-only clients and verifies the cert when present; `required: true` blocks all Data packets from a session until a valid cert is received; no new crate dependency (uses existing `ed25519-dalek`); config block `"mtls"` in `server.json` (`aivpn-server/src/mtls.rs`)
- **Protocol: three new control subtypes** — `RouteSync = 0x13`, `ChainForward = 0x14`, `ClientCert = 0x15` added to `ControlSubtype` enum and fully encoded/decoded in `ControlPayload` with 4-byte LE length-prefix framing (`aivpn-common/src/protocol.rs`)

### Security

- **mTLS enforcement** — `Session.mtls_ok` field tracks per-session cert state; set to `false` at session creation when `mtls.required = true`; flipped to `true` only on a valid `ClientCert` message; Data packets are silently dropped until the gate opens
- **Exit-node relay gating** — `ChainForward` messages are rejected unless `GatewayConfig.exit_node_enabled` is `true` (`pool.exit_node_enabled` in config); prevents any server from inadvertently acting as an open relay
- **RouteSync peer authentication** — inbound `RouteSync` is matched against configured peer endpoints (`IP:port`); packets from unknown senders are dropped; each advertised subnet is checked against the peer's declared `remote_subnets` allowlist before any `ip route add` subprocess is spawned; default routes, loopback, and link-local prefixes are unconditionally rejected; deserialization is bounded (4 KiB JSON / 64 subnets)
- **Zero sync_key guard** — chain forwarder startup aborts with an error log if `pool.sync_key` is absent, malformed, or decodes to all-zero bytes

### Changed

- Version bumped 0.7.0 → 0.8.0 across workspace `Cargo.toml`, all crate `Cargo.toml` files, macOS `Info.plist`, iOS `App/Info.plist` and `Tunnel/Info.plist`, macOS/iOS version strings, Android `version_footer`
- `GatewayConfig` gains `event_bus: EventBus` and `qos_enforcer: Arc<QosEnforcer>` (backward-compatible `Default` impl); also gains `chain_forwarder: Option<Arc<ChainForwarder>>`, `mtls: Option<MtlsConfig>`, `exit_node_enabled: bool`
- `ClientConfig` gains `qos: Option<ClientQos>` with `#[serde(default)]` — existing `clients.json` files are unaffected
- `PoolSyncConfig` gains `exit_node: Option<String>` and `exit_node_enabled: Option<bool>`
- `ServerFileConfig` gains `site_to_site: Option<SiteToSiteConfig>`, `mtls: Option<MtlsConfig>`, `dns: Option<DnsProxyConfig>` (all `#[serde(default)]`)
- Server `--audit-log` defaults to `/var/log/aivpn/audit.log`
- `aivpn-server/Cargo.toml` adds `flate2 = "1"` and `tar = "0.4"` for backup functionality; adds `dns = ["reqwest"]` feature

---

## [0.8.0] — 2026-06-13

### Добавлено

- **Пул серверов / отказоустойчивость** — блок `pool` в `server.json`; узлы используют общую пару ключей X25519; синхронизация встроена в основной VPN-протокол (`ControlPayload::PoolSync` 0x12) через UDP-порт VPN — трафик синхронизации неотличим от клиентского, не требует отдельного порта или правила firewall; все узлы выводят одинаковые `SessionKeys` из общего `sync_key` через blake3 KDF; команда `aivpn-server enroll <peer>` для регистрации пира в один шаг (`aivpn-server/src/pool_sync.rs`)
- **Пул серверов на клиенте** — режимы failover, round-robin, weighted и latency-based; опциональный массив `pool` в JSON-ключе `aivpn://` (обратная совместимость — старые клиенты игнорируют неизвестные поля) (`aivpn-client/src/server_pool.rs`)
- **Нативный пакет OpenWRT + плагин LuCI** — `aivpn-openwrt/package/aivpn/` с init-скриптом procd, шаблоном UCI-конфига, hotplug-перезапуском при поднятии WAN; веб-интерфейс `luci-app-aivpn` с вкладками Status и Configuration; руководство по установке `aivpn-openwrt/docs/openwrt-setup.md`
- **QoS / ограничение полосы пропускания на клиента** — eBPF TC egress-хук (`ebpf/tc_qos_prog.c`) с картой `LRU_HASH qos_rules`, token-bucket ограничением скорости и DSCP-маркировкой по VPN IP клиента; прозрачный userspace-fallback при отсутствии BPF; флаг CLI `--set-client-qos` (`aivpn-server/src/qos.rs`)
- **Инструменты резервного копирования и миграции** — `--export <path.tar.gz>` и `--import <path.tar.gz>` с `manifest.json`; охватывают БД клиентов, файлы масок и конфигурацию сервера (`aivpn-server/src/backup.rs`)
- **Заглушка наблюдаемости eBPF** — наблюдатель статистики через кольцевой буфер XDP/TC; подключается при наличии `/sys/fs/bpf/aivpn_events`; graceful no-op при отсутствии (`aivpn-server/src/ebpf_observer.rs`)
- **Структурированное логирование событий** — перечисление `AivpnEvent`: подключение/отключение, ротация ключей, XDP-дропы, синхронизация пиров, kill-switch; `EventBus` с JSONL-выводом в stdout и опциональным вебхуком (`aivpn-common/src/event_log.rs`)
- **Адаптивный режим** — скользящее окно из 20 пакетов отслеживает потери; автоматически корректирует `mtu_delta` (−50 за шаг, минимум 576) и множитель keepalive; флаг CLI `--adaptive`; переключатель во всех UI-клиентах: Windows (egui панель), macOS (popover меню), iOS (строка настроек), Android (меню опций) (`aivpn-client/src/adaptive.rs`)
- **Аудит-лог администратора** — append-only JSONL по пути `/var/log/aivpn/audit.log` (настраивается через `--audit-log`); фиксирует субъект, действие, цель, результат и метку времени ISO-8601 для каждой операции управления (`aivpn-server/src/audit_log.rs`)
- **Бенчмарк / Диагностика** — UDP RTT-зондирование, перцентили задержки P50/P95/P99, пропускная способность вверх/вниз, процент потерь, оценка качества 0–100; подкоманда `aivpn-client bench`; панель Диагностика в Windows GUI, macOS popover, iOS sheet, Android диалог из меню опций (`aivpn-client/src/bench.rs`)
- **Статистика дропов eBPF XDP** — `xdp_prog.c` теперь ведёт карту `BPF_MAP_TYPE_ARRAY` (`drop_stats`, 4 слота: `TOO_SHORT`, `TAG_EXPIRED`, `RESERVED`, `TOTAL`) и кольцевой буфер `BPF_MAP_TYPE_RINGBUF` объёмом 256 КБ (`events`). Все пути XDP_DROP вызывают инлайн-хелпер `record_drop(reason)`, атомарно увеличивающий счётчик и отправляющий событие в кольцевой буфер. `ebpf_observer.rs` открывает закреплённую карту через сырые syscall `BPF_OBJ_GET` / `BPF_MAP_LOOKUP_ELEM` (без новых зависимостей) и публикует дельта-события `XdpDrop` в `EventBus` (`aivpn-server/src/ebpf_observer.rs`, `aivpn-linux-kernel/src/xdp_prog.c`)
- **DNS-over-HTTPS прокси** — опциональный встроенный DoH-форвардер (`feature = "dns"`); слушает UDP :53 на VPN-интерфейсе и пробрасывает запросы через RFC 8484 POST к настраиваемому апстриму (по умолчанию Cloudflare); поддерживается опциональный запасной резолвер; режим `block_plain_dns` добавляет правило nftables, блокирующее UDP/53 на не-VPN интерфейсах, чтобы клиенты не могли обойти прокси; блок конфигурации `"dns"` в `server.json` (`aivpn-server/src/dns_proxy.rs`)
- **Сеть сайт-сайт (site-to-site VPN)** — два или более узла AIVPN могут соединить свои локальные подсети без клиентского ПО; пиры обмениваются маршрутами через `ControlPayload::RouteSync` (0x13), используя тот же blake3 KDF, что и пул-синхронизация; исходящие объявления отправляются каждые 30 с; входящий `RouteSync` аутентифицируется по списку настроенных пиров (точное совпадение `IP:port`), каждая полученная подсеть проверяется по allowlist `remote_subnets` пира, опасные префиксы (маршрут по умолчанию, loopback, link-local) отклоняются, полезная нагрузка ограничена 4 КиБ / 64 подсети; блок конфигурации `"site_to_site"` в `server.json` (`aivpn-server/src/site_sync.rs`)
- **Многоскачковая цепочка (multi-hop)** — прозрачная маршрутизация через двойной скачок; входной узел расшифровывает IP-нагрузку клиента и переупаковывает её в `ControlPayload::ChainForward` (0x14), зашифрованный общим ключом пула, и пересылает на выходной узел; выходной узел вводит внутреннюю нагрузку прямо в TUN-устройство и маршрутизирует в интернет; клиент не знает о промежуточном скачке; конфигурация: `pool.exit_node` на входном узле, `pool.exit_node_enabled: true` на выходном (по умолчанию `false`, чтобы не превратиться в открытый прокси); `pool.sync_key` должен быть корректным ненулевым 32-байтным ключом, иначе chain forwarder не запустится (`aivpn-server/src/chain_forwarder.rs`)
- **Лёгкий mTLS (mTLS-lite)** — опциональные ed25519-подписанные клиентские сертификаты поверх существующего X25519 + PSK-рукопожатия; сертификат — компактный токен в 104 байта (`client_pub_key[32] || expiry_ts_le[8] || ca_signature[64]`), передаётся через `ControlPayload::ClientCert` (0x15); `required: false` (по умолчанию) принимает клиентов без сертификата и проверяет его при наличии; `required: true` блокирует все Data-пакеты сессии до получения корректного сертификата; без новых зависимостей (используется существующий `ed25519-dalek`); блок конфигурации `"mtls"` в `server.json` (`aivpn-server/src/mtls.rs`)
- **Протокол: три новых управляющих подтипа** — `RouteSync = 0x13`, `ChainForward = 0x14`, `ClientCert = 0x15` добавлены в перечисление `ControlSubtype` и полностью реализованы в `ControlPayload` с 4-байтовым LE-префиксом длины (`aivpn-common/src/protocol.rs`)

### Безопасность

- **Принудительный mTLS** — поле `Session.mtls_ok` отслеживает состояние сертификата в рамках сессии; устанавливается в `false` при создании сессии, если `mtls.required = true`; переключается в `true` только при получении корректного сообщения `ClientCert`; Data-пакеты сбрасываются до открытия ворот
- **Ограничение ретрансляции exit-узла** — сообщения `ChainForward` отклоняются, если `GatewayConfig.exit_node_enabled` не равно `true` (`pool.exit_node_enabled` в конфиге); исключает случайное превращение сервера в открытый прокси
- **Аутентификация пиров RouteSync** — входящий `RouteSync` сопоставляется с адресами настроенных пиров (`IP:port`); пакеты от неизвестных отправителей сбрасываются; каждая рекламируемая подсеть проверяется по allowlist `remote_subnets` пира перед любым вызовом `ip route add`; маршруты по умолчанию, loopback и link-local префиксы безусловно отклоняются; десериализация ограничена (4 КиБ JSON / 64 подсети)
- **Защита от нулевого sync_key** — запуск chain forwarder прерывается с записью в лог об ошибке, если `pool.sync_key` отсутствует, некорректен или декодируется в последовательность нулевых байт

### Изменено

- Версия поднята с 0.7.0 до 0.8.0 во всём workspace `Cargo.toml`, всех `Cargo.toml` крейтов, macOS `Info.plist`, iOS `App/Info.plist` и `Tunnel/Info.plist`, строках версии macOS/iOS, Android `version_footer`
- `GatewayConfig` получает поля `event_bus: EventBus` и `qos_enforcer: Arc<QosEnforcer>` (обратносовместимая реализация `Default`); также получает `chain_forwarder: Option<Arc<ChainForwarder>>`, `mtls: Option<MtlsConfig>`, `exit_node_enabled: bool`
- `ClientConfig` получает `qos: Option<ClientQos>` с `#[serde(default)]` — существующие `clients.json` не затронуты
- `PoolSyncConfig` получает `exit_node: Option<String>` и `exit_node_enabled: Option<bool>`
- `ServerFileConfig` получает `site_to_site: Option<SiteToSiteConfig>`, `mtls: Option<MtlsConfig>`, `dns: Option<DnsProxyConfig>` (все с `#[serde(default)]`)
- `--audit-log` по умолчанию равен `/var/log/aivpn/audit.log`
- `aivpn-server/Cargo.toml` добавляет `flate2 = "1"` и `tar = "0.4"` для функционала резервного копирования; добавляет фичу `dns = ["reqwest"]`

---


## [0.7.0] - 2026-06-13

### Added
- **Advanced Split-Tunneling**: `--include-routes` and `--exclude-routes` CLI flags for fine-grained per-CIDR routing control on Linux, macOS, and Windows. Routes are automatically cleaned up on disconnect.
- **Kill-Switch + Leak Protection**: `--kill-switch` flag installs firewall rules (nftables on Linux, pfctl on macOS, Windows Firewall on Windows) that block all non-VPN traffic. Rules survive unexpected process termination and persist until explicitly cleared with `kill-switch clear`.
- **IPv6 Dual-Stack**: Full NAT66/NPTv6 support on the server (`aivpn-server`). New `ipv6_enabled` and `ipv6_prefix` fields in `VpnNetworkConfig`; clients receive an IPv6 address in `ServerHello`.
- **MTU Auto-Detection**: `mtu: "auto"` in server config triggers PMTUD-based MTU discovery, replacing hardcoded 1400-byte defaults.
- **Mask Validator**: `--validate-mask <path>` server subcommand validates a mask JSON file — checks structure, confidence score, FSM reachability, and IAT distribution consistency.
- **Six New DPI-Evasion Masks**: `avito`, `sber`, `vk`, `sberjazz`, `whatsapp`, and `yandex` traffic profiles added to `mask-assets/`. Each has confidence score ≥ 0.90.
- **Neural Anti-Probing Improvements**: Neural Resonance Module now tracks 64 traffic features including burst pattern, packet direction ratio, IAT periodicity, and entropy variance. Rotation cooldown of 60 s prevents oscillation under sustained active probing.
- **Linux Desktop GUI**: Native Linux application (`aivpn-linux`) built with Iced framework, distributed as AppImage with system tray integration.
- **eBPF/XDP Early Packet Filter**: `aivpn-linux-kernel` module now compiles an XDP BPF program (`xdp_prog.o`). When present, it attaches to the default-route NIC at connect time and drops malformed or replayed UDP packets at NIC level before socket buffer allocation. Configuration is pinned at `/sys/fs/bpf/aivpn/xdp_config`.
- **Threat Model Document**: Added `THREAT_MODEL.md` covering adversary model, cryptographic design, traffic-analysis resistance, kill-switch guarantees, XDP properties, and known limitations.

### Changed
- **`record_traffic` API**: Added `is_rx: bool` parameter for directional traffic analysis (upload vs. download distinction in neural feature extraction).
- **Rust Workspace version**: Bumped to 0.7.0.
- **macOS build**: CFBundleVersion bumped to 5.
- **iOS build**: CFBundleVersion bumped to 3.

### Fixed
- **`resolve_config_path` crash**: Server no longer calls `process::exit(1)` when `/etc/aivpn/server.json` exists but is not readable (e.g. root-owned). Auto-discovery now uses `File::open().is_ok()` instead of `path.exists()`.
- **Test fixture API alignment**: Updated `VpnNetworkConfig`, `ClientNetworkConfig`, and `ServerArgs` test literals in `client_db.rs`, `management_api_tests.rs`, and `main.rs` to match 0.7.0 struct fields.

## [0.7.0] — 2026-06-13

### Добавлено
- **Раздельное туннелирование**: Флаги `--include-routes` и `--exclude-routes` для точного управления маршрутизацией по CIDR на Linux, macOS и Windows. Маршруты автоматически удаляются при отключении.
- **Kill-Switch + защита от утечек**: Флаг `--kill-switch` устанавливает правила брандмауэра (nftables на Linux, pfctl на macOS, Windows Firewall на Windows), блокирующие весь не-VPN трафик. Правила сохраняются при неожиданном завершении процесса и удаляются командой `kill-switch clear`.
- **IPv6 Dual-Stack**: Полная поддержка NAT66/NPTv6 на сервере (`aivpn-server`). Новые поля `ipv6_enabled` и `ipv6_prefix` в `VpnNetworkConfig`; клиенты получают IPv6-адрес в `ServerHello`.
- **Авто-определение MTU**: `mtu: "auto"` в конфигурации сервера запускает PMTUD-определение MTU вместо фиксированных значений.
- **Валидатор масок**: Подкоманда `--validate-mask <path>` проверяет JSON-файл маски — структуру, оценку уверенности, достижимость состояний FSM и согласованность распределения IAT.
- **Шесть новых масок для обхода DPI**: Профили `avito`, `sber`, `vk`, `sberjazz`, `whatsapp` и `yandex` добавлены в `mask-assets/`. Оценка уверенности ≥ 0.90 у каждой.
- **Улучшения нейронного анти-зондирования**: Модуль Neural Resonance теперь отслеживает 64 признака трафика: паттерны burst, соотношение направлений пакетов, периодичность IAT и дисперсию энтропии. Кулдаун ротации 60 с предотвращает осцилляцию при продолжительном зондировании.
- **Linux Desktop GUI**: Нативное приложение (`aivpn-linux`) на фреймворке Iced, распространяется как AppImage с интеграцией системного трея.
- **eBPF/XDP фильтр раннего отклонения пакетов**: Модуль `aivpn-linux-kernel` теперь компилирует XDP BPF программу (`xdp_prog.o`). При наличии подключается к NIC на уровне RX и отбрасывает некорректные или повторяющиеся UDP-пакеты до выделения буфера сокета. Конфигурация пинится по пути `/sys/fs/bpf/aivpn/xdp_config`.
- **Документ модели угроз**: Добавлен `THREAT_MODEL.md` — модель злоумышленника, криптографический дизайн, устойчивость к анализу трафика, гарантии kill-switch, свойства XDP и известные ограничения.

### Изменено
- **API `record_traffic`**: Добавлен параметр `is_rx: bool` для направленного анализа трафика.
- **Версия Rust Workspace**: Обновлена до 0.7.0.
- **macOS-сборка**: CFBundleVersion обновлён до 5.
- **iOS-сборка**: CFBundleVersion обновлён до 3.

### Исправлено
- **Падение `resolve_config_path`**: Сервер больше не вызывает `process::exit(1)`, если `/etc/aivpn/server.json` существует, но недоступен для чтения. Авто-обнаружение теперь использует `File::open().is_ok()` вместо `path.exists()`.
- **Согласование тестовых данных**: Обновлены тестовые литералы `VpnNetworkConfig`, `ClientNetworkConfig` и `ServerArgs` в `client_db.rs`, `management_api_tests.rs` и `main.rs` под API 0.7.0.


## [0.6.0] - 2026-06-12

### Added
- **MikroTik RouterOS 7 support**: Docker container (`aivpn-mikrotik`) for running the AIVPN server inside a RouterOS 7 container slot. veth+TUN topology, minimal scratch-based image, `AIVPN_KEY` env var for one-line provisioning. Full RouterOS setup guide included.
- **Configurable listen address**: `AIVPN_LISTEN` environment variable allows overriding the server bind address and port at runtime without touching config files.
- **SOCKS5 proxy mode (client)**: New `--proxy` / `-P` flag routes VPN traffic through a userspace TCP stack (smoltcp). For environments where raw UDP is blocked or unreliable.
- **SOCKS5 proxy toggle (Windows GUI)**: Windows GUI exposes the proxy mode as a settings toggle.
- **Linux kernel module (`aivpn-linux-kernel`)**: Optional `aivpn.ko` module offloads session tag lookup and packet crypto to kernel space. Dual-table RCU design, atomic nonce counters, WireGuard-style replay window, `/dev/aivpn` character device (ioctl API v2).
- **KernelAccel integration**: Server and client auto-detect and load `aivpn.ko` on Linux. Session lifecycle and tag-window updates pushed via ioctl. Transparent fallback to userspace when module is absent.
- **Cross-platform stop signals**: Client handles `SIGTERM`/`SIGINT` on Unix and `Ctrl+C` on Windows uniformly, with clean TUN teardown.
- **Configurable keepalive**: Keepalive interval stored per-client in `ClientDatabase` and exposed via management API.

### Fixed
- **macOS full-tunnel routing**: Rewrote route setup — full route wipe on disconnect, correct subnet route syntax (`-net` flag).
- **Kernel security audit (aivpn.ko)**:
  - *Critical* — nonce no longer extracted from wire bytes; derived solely from internal atomic counter.
  - *High* — use-after-free: session pointer no longer dereferenced after `rcu_read_unlock()` in `udp_hook`.
  - *Medium* — AEAD authentication: AAD scatter-gather list now correctly linked into AEAD request (resonance tag was previously unauthenticated).
  - `CAP_NET_ADMIN` capability check added to `/dev/aivpn` open path.
- **Server security audit**:
  - `forward_packet()` write path was broken (referenced `self.writer` always `None`); fixed to use `writer_taken`.
  - `DashMap` unbounded growth: `rate_limits` and `handshake_cooldowns` maps pruned every 5 seconds.
  - `Session::is_expired()` removed — always returned `true` due to `HARD_TIMEOUT = Duration::ZERO`; no callers.
  - iptables: replaced legacy `-m state --state` with `-m conntrack --ctstate` (modern kernels).
- **Android build**: Force-delete stale APK before signing to prevent shipping previous build.
- **macOS build**: Create `releases/` directory before writing installer package.
- **iOS build**: Updated bridging header to include `aivpn_core.h` via header search paths; `aivpn-ios-core` included in musl cross-build Docker context.
- **Test stability**: Fixed time-based flakiness in `battle_session_multiple_clients` by checking adjacent tag windows.

### Build / CI
- Windows cross-compilation and iOS unsigned IPA jobs added to release asset workflow.
- `aivpn-ios-core` workspace member added to musl Dockerfile `COPY` context.
- `releases/` directory removed from git tracking; added to `.gitignore`.

## [0.6.0] — 2026-06-12

### Добавлено
- **Поддержка MikroTik RouterOS 7**: Docker-контейнер (`aivpn-mikrotik`) для запуска сервера AIVPN в слоте контейнера RouterOS 7. Топология veth+TUN, минимальный образ на базе scratch, переменная `AIVPN_KEY` для одностроковой инициализации. Включена полная документация по настройке RouterOS.
- **Настраиваемый адрес прослушивания**: Переменная окружения `AIVPN_LISTEN` позволяет задавать адрес и порт сервера во время выполнения без изменения конфигурационных файлов.
- **Режим SOCKS5-прокси (клиент)**: Новый флаг `--proxy` / `-P` маршрутизирует VPN-трафик через пользовательский TCP-стек (smoltcp). Предназначен для сред, где UDP заблокирован или ненадёжен.
- **Переключатель SOCKS5-прокси (Windows GUI)**: В настройках Windows-клиента добавлен переключатель режима прокси.
- **Модуль ядра Linux (`aivpn-linux-kernel`)**: Опциональный модуль `aivpn.ko` переносит поиск сессионных тегов и криптографию пакетов в пространство ядра. Двутабличная RCU-архитектура, атомарные счётчики nonce, окно воспроизведения в стиле WireGuard, символьное устройство `/dev/aivpn` (ioctl API v2).
- **Интеграция KernelAccel**: Сервер и клиент автоматически обнаруживают и загружают `aivpn.ko` под Linux. Жизненный цикл сессий и обновления окна тегов передаются через ioctl. Прозрачный откат на пользовательское пространство при отсутствии модуля.
- **Кроссплатформенные сигналы завершения**: Клиент единообразно обрабатывает `SIGTERM`/`SIGINT` на Unix и `Ctrl+C` в Windows с корректным удалением TUN-интерфейса.
- **Настраиваемый keepalive**: Интервал keepalive хранится отдельно для каждого клиента в `ClientDatabase` и доступен через management API.

### Исправлено
- **Полная маршрутизация macOS**: Переписана настройка маршрутов — полное удаление маршрутов при отключении, корректный синтаксис подсетевых маршрутов (`-net`).
- **Аудит безопасности ядра (aivpn.ko)**:
  - *Критично* — nonce больше не извлекается из входящих байтов; выводится исключительно из внутреннего атомарного счётчика.
  - *Высокий* — use-after-free: указатель сессии больше не разыменовывается после `rcu_read_unlock()` в `udp_hook`.
  - *Средний* — аутентификация AEAD: scatter-gather список AAD теперь корректно включён в AEAD-запрос (ранее resonance-тег не аутентифицировался).
  - Добавлена проверка `CAP_NET_ADMIN` при открытии `/dev/aivpn`.
- **Аудит безопасности сервера**:
  - Путь записи-fallback `forward_packet()` был сломан (ссылался на `self.writer`, всегда равный `None`); исправлено на `writer_taken`.
  - Неограниченный рост `DashMap`: карты `rate_limits` и `handshake_cooldowns` теперь очищаются каждые 5 секунд.
  - Удалён `Session::is_expired()` — всегда возвращал `true` из-за `HARD_TIMEOUT = Duration::ZERO`; вызовов нет.
  - iptables: устаревший `-m state --state` заменён на `-m conntrack --ctstate` (современные ядра).
- **Android-сборка**: Принудительное удаление устаревшего APK перед подписью предотвращает публикацию предыдущей сборки.
- **macOS-сборка**: Создание директории `releases/` до записи пакета установщика.
- **iOS-сборка**: Обновлён bridging header для включения `aivpn_core.h` через пути поиска заголовков; `aivpn-ios-core` добавлен в Docker-контекст musl-сборок.
- **Стабильность тестов**: Устранено нестабильное поведение `battle_session_multiple_clients`, зависевшее от времени выполнения.

### Сборка / CI
- В workflow GitHub Actions добавлены задания кросс-компиляции для Windows и сборки неподписанного IPA для iOS.
- Член воркспейса `aivpn-ios-core` добавлен в `COPY`-контекст musl-Dockerfile.
- Директория `releases/` исключена из git-трекинга и добавлена в `.gitignore`.


## [0.5.0] - 2026-06-11

### Added
- **iOS Client application**: Native Swift application with a Network Extension (`PacketTunnelProvider`) and integrated Rust core (`aivpn-ios-core`).
- **Android Quick Settings tile**: One-tap quick settings tile for toggling the VPN connection easily.
- **ED25519 descriptor verification**: Verification of `BootstrapDescriptor` signatures using ed25519 trusted keys.
- **Neural core auto-calibration**: Added auto-calibration for MSE and O(1) time complexity optimization using sliding window in `VecDeque`.
- **CI/CD build automation**: Added automated release builds for Windows client binaries, NSIS installers, and iOS unsigned IPAs directly in GitHub Actions.

### Changed
- **Apksigner integration**: Switch from deprecated `jarsigner` to `apksigner` for Android APK v2/v3 signing.
- **Improved Windows installer**: Enhanced NSIS-based cross-compilation packaging.
- **Rust workspace version**: Bumped to 0.5.0.

### Fixed
- **Helper daemon security**: Fixed world-writable socket permissions in macOS client helper.
- **Key rotation logic**: Fixed key rotation ratchet no-op bug.
- **Deadlock resolved**: Fixed server handshake retry deadlock on Android.
- **Layout & Docs**: Stability fixes for macOS layout, secure fields, and post-connect sync.

## [0.4.0] - 2026-04-18

### Added
- **PSK-based bootstrap mask selection**: Deterministic initial mask selection based on PSK hash (blake3)
- **Multi-channel bootstrap loader**: Load descriptors from CDN, Telegram, GitHub, IPFS
- **Background descriptor refresh**: Automatic bootstrap descriptor updates
- **Neural resonance check**: Resonance verification system for detecting compromised masks
- **Mask recording mode**: Traffic recording mode for generating new masks from captured traffic
- **PFS ratchet**: Perfect Forward Secrecy with automatic key rotation
- **Linux arm64 support**: Full aarch64 support for server and client (Keenetic KN1012, OpenWrt, NanoPi R3S)
- **New mask presets**: Added QUIC over HTTPS v2 mask for improved traffic mimicry

### Changed
- **Optimized binary sizes**: Reduced binary sizes by 3-5x (release build)
- **Universal macOS binaries**: All macOS components built as universal (x86_64 + arm64)
- **Improved session management**: Better handling of sessions and reconnections
- **Removed 24h hard session timeout**: `HARD_TIMEOUT` now defaults to `Duration::ZERO` (unlimited). PFS ratchet handles key rotation, forced expiration caused reconnect failures (Issue #33)
- **Enhanced error handling**: More detailed connection error diagnostics

### Fixed
- **macOS helper daemon**: Fixed privileged helper daemon issues
- **Android JNI stability**: Improved JNI call stability
- **Bootstrap mask rotation**: Correct mask rotation on compromise
- **Session tag window**: Fixed edge cases in tag handling
- **Bootstrap mask loading** (Issue #38): Fixed parsing of bootstrap mask files - now supports both single MaskProfile objects and arrays of MaskProfile objects, as well as empty files
- **Bootstrap file reference removed from example config**: The `bootstrap_mask_files` entry has been removed from `config/server.json.example` since the bootstrap mask file is no longer created automatically. Users who need custom bootstrap masks can add the `bootstrap_mask_files` entry manually.

### Platform Updates
- **macOS**: v0.4.0 (build 4)
  - Installer: aivpn-macos.pkg (15 MB)
  - DMG: aivpn-macos.dmg (15 MB)
  - CLI: aivpn-client-macos-universal (17 MB)
- **Android**: API level 26+, universal APK 7 MB
- **Windows**: Rebuild required
- **Linux Server**:
  - x86_64 (4.7 MB)
  - arm64/aarch64 (5.0 MB) - **NEW** for Keenetic KN1012, OpenWrt, NanoPi R3S
  - armv7 (3.5 MB)
  - mipsel (4.5 MB)
- **Linux Client**:
  - x86_64 (3.8 MB)
  - arm64/aarch64 (9.6 MB) - **NEW** for Keenetic, OpenWrt, NanoPi
  - armv7 (3.5 MB)
  - mipsel (4.5 MB)

### Technical Details
- Rust workspace version: 0.4.0
- Protocol version: compatible with 0.3.x
- Minimum macOS: 13.0
- Minimum Android: 8.0 (API 26)
