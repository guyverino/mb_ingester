# mb_ingester

Open-source Rust-демон, подключающийся к Moonbot-серверам через приватный
протокол **MoonProto** ([`Moonbot-Tech/MoonProtoBeta`](https://github.com/Moonbot-Tech/MoonProtoBeta)),
получающий потоки стратегий и ордеров, и сохраняющий их в PostgreSQL.

Ничего больше: ни торговой логики, ни аналитики, ни Telegram, ни AI.
Это чистый storage-слой. Поверх него можно строить что угодно.

## Возможности

* Подключение одним процессом к N серверам параллельно (один OS-thread = один сервер).
* Автоматический handshake, шифрование (AES-GCM), reconnect, NTP-sync (встроены в `moonproto`).
* Запись в типизированную схему PostgreSQL:
  * `strategies` — текущее состояние
  * `strategy_versions` — история (по `LastEditDate`)
  * `orders` — ордера в реальном времени
* **Modular toggles**: per-server включение/выключение модулей через JSONB-колонку `servers.modules`.
* **Runtime-конфигурация в БД**: timeouts/intervals в `app_settings`, можно менять без рестарта (TTL-cache 60 сек).
* Минимальный `config.toml` — только DB URL. Серверы и параметры — в БД.

## Архитектура модулей

Вместо булевых колонок `cloner_enabled / risk_enabled / ...` всё сделано через две таблицы:

* `modules` — каталог: какие модули вообще существуют, что они делают, от чего зависят.
* `servers.modules` (JSONB) — per-server: какие модули включены ДЛЯ этого инстанса.
* `app_settings.module` (FK на modules.name) — настройки этого модуля.

В этом crate'е встроено два модуля:

| name                  | category | описание                                                                  |
|-----------------------|----------|---------------------------------------------------------------------------|
| listener_strategies   | storage  | Принимает snapshots стратегий, пишет в strategies + strategy_versions    |
| listener_orders       | storage  | Принимает события ордеров, пишет в orders                                 |

Добавить новый модуль (пример — `listener_balances`):

```sql
INSERT INTO modules (name, description, category) VALUES
  ('listener_balances', 'Запись балансов', 'storage');

UPDATE servers SET modules = modules || '{"listener_balances": true}'::jsonb
WHERE name = 'BB';
```

И код, который смотрит `server.modules.listener_balances` перед записью.

## Быстрый старт

```bash
# 1. Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env

# 2. MoonProto library (нужен доступ к приватной beta)
git clone https://github.com/Moonbot-Tech/MoonProtoBeta.git ../MoonProtoBeta

# 3. PostgreSQL
sudo -u postgres createdb mb_ingester

# 4. Конфиг
cp config.example.toml config.toml
$EDITOR config.toml      # вписать DSN

# 5. Добавить сервер
psql -d mb_ingester -c "
INSERT INTO servers (name, ip, port, token, modules) VALUES (
  'BB', '1.2.3.4', 3000,
  '<base64 key exported from MoonBot>',
  '{\"listener_strategies\": true, \"listener_orders\": true}'::jsonb
);"

# 6. Запуск
cargo run --release
```

Логи: `RUST_LOG=info ./target/release/mb_ingester` (`debug` для подробностей).

## Структура

```
ingester/
├── Cargo.toml
├── LICENSE                 MIT
├── README.md
├── config.example.toml
├── migrations/
│   └── 001_initial.sql     все 6 таблиц + seed модулей
└── src/
    ├── main.rs             entry point + orchestrator
    ├── config.rs           TOML loader + DbServer struct
    ├── db.rs               postgres connect + load_servers
    ├── settings.rs         app_settings TTL-cache reader
    ├── session.rs          MoonProto event loop, одна сессия = 1 thread
    └── storage/
        ├── mod.rs
        ├── strategies.rs   upsert в strategies + versions
        └── orders.rs       upsert в orders
```

## Параметры в `app_settings`

| Параметр                       | Module                | Default | Что делает                                                |
|--------------------------------|------------------------|---------|-----------------------------------------------------------|
| connect_timeout_secs           | (global)               | 15      | Connect timeout к MoonBot                                |
| event_poll_ms                  | (global)               | 500     | Период опроса событий внутри session-thread              |
| orders_subscribe_snapshot      | listener_orders        | true    | Запросить AllStatuses snapshot при старте                |
| orders_sync_on_snapshot        | listener_orders        | true    | На Order::Snapshot удалять отсутствующие ордера          |
| strategies_subscribe_schema    | listener_strategies    | true    | Запрашивать live StratSchema во время Init               |
| strategies_log_field_diff      | listener_strategies    | false   | Логировать changed fields при новой версии (debug)        |

Изменения применяются через ~60 сек (TTL кэша).

## Ограничения

* **MoonProto в beta.** API библиотеки меняется почти ежедневно. Этот crate пересобирается на новых snapshot-ах и временами требует обновлений.
* Не реализовано (вне scope этого crate):
  * Запись балансов / orderbook / trades stream
  * Любая торговая логика
  * Telegram / Web UI

## Лицензия

MIT (см. [LICENSE](LICENSE)).
