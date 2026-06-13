# Changelog

## [0.9.0] - 2026-06-13

### Added

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

- `GatewayConfig` gains `chain_forwarder: Option<Arc<ChainForwarder>>`, `mtls: Option<MtlsConfig>`, `exit_node_enabled: bool` (all backward-compatible via `Default`)
- `PoolSyncConfig` gains `exit_node: Option<String>` and `exit_node_enabled: Option<bool>`
- `ServerFileConfig` gains `site_to_site: Option<SiteToSiteConfig>`, `mtls: Option<MtlsConfig>`, `dns: Option<DnsProxyConfig>` (all `#[serde(default)]`)
- `aivpn-server/Cargo.toml` adds `dns = ["reqwest"]` feature

---

## [0.9.0] — 2026-06-13

### Добавлено

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

- `GatewayConfig` получает поля `chain_forwarder: Option<Arc<ChainForwarder>>`, `mtls: Option<MtlsConfig>`, `exit_node_enabled: bool` (обратносовместимо через `Default`)
- `PoolSyncConfig` получает `exit_node: Option<String>` и `exit_node_enabled: Option<bool>`
- `ServerFileConfig` получает `site_to_site: Option<SiteToSiteConfig>`, `mtls: Option<MtlsConfig>`, `dns: Option<DnsProxyConfig>` (все с `#[serde(default)]`)
- `aivpn-server/Cargo.toml` добавляет фичу `dns = ["reqwest"]`

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

### Changed

- Version bumped 0.7.0 → 0.8.0 across workspace `Cargo.toml`, all crate `Cargo.toml` files, macOS `Info.plist`, iOS `App/Info.plist` and `Tunnel/Info.plist`, macOS/iOS version strings, Android `version_footer`
- `GatewayConfig` gains `event_bus: EventBus` and `qos_enforcer: Arc<QosEnforcer>` (backward-compatible `Default` impl)
- `ClientConfig` gains `qos: Option<ClientQos>` with `#[serde(default)]` — existing `clients.json` files are unaffected
- Server `--audit-log` defaults to `/var/log/aivpn/audit.log`
- `aivpn-server/Cargo.toml` adds `flate2 = "1"` and `tar = "0.4"` for backup functionality

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

### Изменено

- Версия поднята с 0.7.0 до 0.8.0 во всём workspace `Cargo.toml`, всех `Cargo.toml` крейтов, macOS `Info.plist`, iOS `App/Info.plist` и `Tunnel/Info.plist`, строках версии macOS/iOS, Android `version_footer`
- `GatewayConfig` получает поля `event_bus: EventBus` и `qos_enforcer: Arc<QosEnforcer>` (обратносовместимая реализация `Default`)
- `ClientConfig` получает `qos: Option<ClientQos>` с `#[serde(default)]` — существующие `clients.json` не затронуты
- `--audit-log` по умолчанию равен `/var/log/aivpn/audit.log`
- `aivpn-server/Cargo.toml` добавляет `flate2 = "1"` и `tar = "0.4"` для функционала резервного копирования

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
