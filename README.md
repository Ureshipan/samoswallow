# samoswallow

Self-hosted мини-PaaS: «самосвал» заглатывает гит-репозитории и выкатывает из них
контейнеризированные сервисы с доступом по поддомену, мониторингом и управлением
через веб-интерфейс.

Полное описание архитектуры и дорожная карта — в [`CONCEPT.md`](./CONCEPT.md).

## Стек

Rust (axum + tokio + sqlx/SQLite) · Docker (через `bollard`) · Caddy (wildcard TLS).

## Разработка

```sh
cargo build
SWALLOW_LISTEN=127.0.0.1:8080 cargo run -p swallowd
curl http://127.0.0.1:8080/healthz
```

Конфигурация через переменные окружения (`SWALLOW_*`), см. `crates/swallowd/src/config.rs`.

## Установка на сервер

```sh
cargo build --release
sudo ./scripts/install.sh            # ставит бинарь, systemd-юнит, конфиг
sudo ./scripts/uninstall.sh --purge  # полное удаление вместе с данными
```

После запуска открой `http://127.0.0.1:8080/` — там веб-интерфейс: добавление
приложений, деплой, мониторинг инстансов (CPU/RAM), restart/stop, логи сборок.

## Статус

Фаза 1 (MVP), почти готова. Работает:

- HTTP JSON API + серверный веб-интерфейс (maud SSR).
- Деплой из git: clone → сборка образа (Docker/bollard) → запуск инстанса →
  роут поддомена через Caddy → ротация старых инстансов.
- Управление инстансами: restart / stop, снапшоты нагрузки (CPU/RAM), логи.
- SQLite-состояние с миграциями, упаковка (systemd + install/uninstall).

Осталось по фазе 1: аутентификация, авто-установка Caddy в `install.sh`.
