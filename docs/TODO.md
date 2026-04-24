# AIVPN — TODO / Roadmap

**Первичный аудит:** 2026-03-25  
**Обновлено:** 2026-04-24  
**Версия:** 0.4.0  
**Состояние проекта:** Production (server + clients работают end-to-end)

---

## Решённые проблемы (Resolved)

### Бывшие Critical (все исправлены ✅)

| ID | Проблема | Статус | Коммит/Версия |
|----|----------|--------|---------------|
| C1 | `receive_and_write_packet()` — заглушка | ✅ Реализован полный receive pipeline | 0.3.x |
| C2 | `main.rs` — зависание на старте | ✅ Ctrl+C handler в spawn | 0.3.x |
| C3 | MutexGuard held across `.await` (deadlock) | ✅ Извлечение перед `.await` | 0.3.x |
| C4 | `tag_map` никогда не заполняется | ✅ Реализована вставка тегов в `tag_map` | 0.3.x |
| C5 | Nonce collision (u16 vs u64) | ✅ Унифицированный counter (u64) | 0.3.x |

### Бывшие High (исправлены ✅)

| ID | Проблема | Статус |
|----|----------|--------|
| H1 | TUN создаётся в конструкторе Gateway | ✅ Отложено до `run()` |
| H2 | LogNormal distribution — мат. баг | ✅ Правильный Box-Muller + empirical CDF |
| H3 | `ControlPayload::decode()` — неполный | ✅ Все subtypes реализованы |
| H4 | Конфигурация hardcoded | ✅ `config/server.json` + CLI args + key-file |

### Бывшие Phase 2–4 (реализованы ✅)

| Задача | Статус |
|--------|--------|
| Mask profile parser (MessagePack) | ✅ `rmp_serde` |
| Mask-Dependent Header builder | ✅ `HeaderSpec` + dynamic generation |
| Bidirectional mimicry (server→client) | ✅ Per-session mask MDH |
| Mask signature verification | ✅ Ed25519 signing + verify |
| Graceful mask transition | ✅ Deferred mask switch (500ms grace) + dual MDH decode |
| 5 initial mask profiles | ✅ zoom, sberjazz, telemost, vk_teams, quic_https |
| In-band CONTROL messages | ✅ ControlPayload full protocol |
| Docker packaging | ✅ Dockerfile + Dockerfile.prebuilt + docker-compose |
| Auto Mask Recording | ✅ Full pipeline: record → analyze → KS-test → store → broadcast |
| Neural Resonance checks | ✅ Baked Mask Encoder, periodic MSE checks |
| PFS key ratchet | ✅ DH2 ratchet with transition window |

---

## Активные проблемы

### 🔴 CRIT-1. `broadcast_mask_update()` — TODO-заглушка

- **Файл:** [`aivpn-server/src/mask_store.rs:158-167`](../aivpn-server/src/mask_store.rs)
- **Проблема:** Функция `broadcast_mask_update()` не отправляет новые записанные маски уже подключённым клиентам. Содержит только `// TODO: broadcast to all active sessions via ControlPayload::MaskUpdate`.
- **Последствие:** После записи новой маски admin-клиентом, другие уже подключённые клиенты продолжают использовать старую маску до переподключения. Новые маски доступны им только через:
  1. Переподключение (получают MaskUpdate при handshake) ✅
  2. Neural Resonance авторотация при DPI-компрометации ✅
- **Приоритет:** HIGH — единственный оставшийся разрыв в mask delivery pipeline
- **Решение:**

```rust
// mask_store.rs — добавить ссылку на SessionManager + UdpSocket
pub async fn broadcast_mask_update(
    &self,
    mask_id: &str,
    sessions: &SessionManager,
    socket: &UdpSocket,
) -> Result<()> {
    if let Some(entry) = self.masks.get(mask_id) {
        let mut sent = 0;
        for session_entry in sessions.iter_sessions() {
            let session = session_entry.value().clone();
            let client_addr = session.lock().client_addr;
            match sessions.build_mask_update_packet(&session, &entry.profile) {
                Ok(packet) => {
                    if socket.send_to(&packet, client_addr).await.is_ok() {
                        sessions.update_session_mask(
                            &session.lock().session_id,
                            entry.profile.clone(),
                        );
                        sent += 1;
                    }
                }
                Err(e) => warn!("Failed to build MaskUpdate for {}: {}", client_addr, e),
            }
        }
        info!("Broadcast mask '{}' to {} clients", mask_id, sent);
    }
    Ok(())
}
```

### 🟡 MED-1. Downlink traffic (bytes_out) не учитывается полностью

- **Файл:** `aivpn-server/src/gateway.rs` → TUN read loop
- **Проблема:** `bytes_out` аккумулируется через `pending_bytes_out` с flush threshold 64KB. Мелкие сессии могут не записать финальные данные при disconnect.
- **Решение:** Flush pending_bytes_out в cleanup_expired() при удалении сессии.

### 🟡 MED-2. Fragmentation не реализован

- **Файл:** `aivpn-server/src/gateway.rs`
- **Проблема:** `InnerType::Fragment` → debug log only. Пакеты >MTU не фрагментируются.

### 🟡 MED-3. Zeroization ключей неполная

- **Проблема:** `SessionKeys` не реализует `Zeroize`. При drop ключи могут остаться в памяти.
- **Решение:** `#[derive(Zeroize, ZeroizeOnDrop)]` на `SessionKeys`.

---

## Roadmap дальнейшего развития

### Фаза A: Mask System Enhancement (ближайшая)

#### A1. Реализовать broadcast_mask_update → CRIT-1
- Добавить ссылки `Arc<SessionManager>` + `Arc<UdpSocket>` в `MaskStore`
- Вызывать broadcast из `mask_gen::generate_and_store_mask()` после записи
- Проверка: подключить 2 клиента → записать маску → оба получают MaskUpdate

#### A2. Централизованный выбор primary mask
- Сейчас primary mask = первая загруженная с диска (недетерминированный порядок)
- Добавить `primary_mask_id` в `server.json` для явного управления
- Альтернатива: выбирать маску с наибольшим `success_rate` из `MaskStats`

#### A3. Crowdsourced mask submission (Phase 5)
- Обычные клиенты записывают частичные traffic fingerprints
- Сервер агрегирует от N клиентов → генерирует маску
- См. `docs/CROWDSOURCED_MASK_LEARNING.md`

#### A4. Mask expiration / TTL
- Добавить `expires_at` в `MaskStats`
- Автоматически деактивировать маски старше N дней
- Принуждает к периодической перезаписи масок для актуальности

### Фаза B: Клиентский UX

#### B1. Windows GUI — интеграция recording UI
- Показывать прогресс записи маски в системном трее
- Статус recording из `RecordingLocalStatus` → UI виджет
- Файл: `aivpn-windows/src/tray.rs`, `vpn_manager.rs`

#### B2. macOS GUI — mask management panel
- Список доступных масок (через ControlPayload extension)
- Ручной выбор предпочтительной маски
- Отображение confidence и success rate

#### B3. Статистика трафика в клиенте
- Текущая скорость up/down (данные есть в `traffic.stats`)
- График использования за сессию
- Время подключения, текущая маска, server ping

### Фаза C: Серверная инфраструктура

#### C1. REST/gRPC Admin API
- `GET /api/masks` — список масок + статистика
- `POST /api/masks/{id}/activate` / `deactivate`
- `GET /api/sessions` — список активных сессий
- `POST /api/clients` — добавление клиента (сейчас через clients.json)
- `GET /api/metrics` — Prometheus metrics exporter

#### C2. Web Panel
- React/Vue dashboard для управления сервером
- Realtime сессии, трафик, маски
- Управление клиентами (выпуск ключей, отключение)

#### C3. Multi-server deployment
- Несколько серверов за одним доменом (DNS round-robin)
- Синхронизация clients.json между серверами
- Shared mask store (S3/MinIO)

#### C4. Log rotation и мониторинг
- ~~Реализована ротация через docker-entrypoint.sh~~ ✅
- Добавить logrotate для native deployment
- Интеграция с Grafana/Loki

### Фаза D: Безопасность и устойчивость к DPI

#### D1. Constant-rate padding (§25)
- Отправка пакетов с фиксированной частотой даже без данных
- Полностью скрывает burst patterns от DPI
- Конфигурируемый rate (trade-off: bandwidth vs stealth)

#### D2. CDN relay transport (§23)
- Маршрутизация через CDN (Cloudflare, CloudFront)
- Трафик выглядит как обычный HTTPS к CDN
- Domain fronting / SNI hiding

#### D3. Bridge distribution system (§24)
- Распределённые bridge-ноды
- Обмен адресами через Tor-подобные механизмы
- Устойчивость к блокировке основного сервера

#### D4. Protocol responder stubs (§28)
- При прямом подключении к порту 443 — отвечает как настоящий HTTPS/QUIC сервер
- Возвращает валидный TLS handshake для не-AIVPN клиентов
- Active probing resistance

#### D5. Passive mask distribution channels
- DNS TXT → стеганографическое распространение масок
- Image LSB → маски в изображениях
- Blockchain OP_RETURN → неуничтожимое хранение
- Реализация: `passive_distribution.rs` (framework готов, загрузчики — заглушки)

### Фаза E: Тестирование

#### E1. Integration tests (e2e client↔server)
- Тест: client connect → handshake → data transfer → disconnect
- Тест: recording → mask generation → broadcast → client apply
- CI pipeline с Docker-based testing

#### E2. Property-based и fuzz-тесты
- Fuzz-тест `AivpnPacket::from_bytes()` и `ControlPayload::decode()`
- Property-test: encrypt → decrypt = identity
- Property-test: mask build → self-test passes

#### E3. DPI simulation
- Имитация DPI-блокировки (drop пакетов по entropy / size pattern)
- Проверка что Neural Resonance → авторотация → клиент не теряет связь
- Benchmark: время переключения маски < 100ms

---

## Техдолг (Low Priority)

| Задача | Файл | Описание |
|--------|------|----------|
| Clippy cleanup | all | ~15 unused imports, naming conventions |
| `AivpnPacket::from_bytes()` | protocol.rs | Бесполезна без MDH context, пометить deprecated |
| TUN read atomicity | tunnel.rs | 4-byte header + payload read не атомичны |
| `M5. verify_signature()` | mask.rs | Реализована Ed25519, но нет проверки chain of trust |
| LFS tracking | releases/ | Бинарники в Git LFS, нужен .gitattributes audit |

---

## Приоритеты на ближайшие спринты

### Sprint 1 (текущий)
- [x] Обновить сервер до 0.4.0 (builds: x86_64 + musl targets)
- [x] Бэкап и верификация server.key + clients.json
- [x] Deploy на 217.26.25.6
- [ ] **Реализовать broadcast_mask_update (CRIT-1)**

### Sprint 2
- [ ] REST Admin API (C1) — минимум: GET /masks, GET /sessions
- [ ] Flush pending_bytes_out при session cleanup (MED-1)
- [ ] Windows GUI recording UI (B1)

### Sprint 3
- [ ] Constant-rate padding (D1) — опциональный режим
- [ ] Integration tests (E1)
- [ ] Web panel MVP (C2)
