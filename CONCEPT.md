# samoswallow — концепт

Self-hosted мини-PaaS: «самосвал» заглатывает гит-репозитории и выкатывает из них
контейнеризированные сервисы с доступом по поддомену третьего уровня, мониторингом
и управлением через веб-интерфейс.

## Стек (зафиксировано)

| Слой              | Решение                                              |
|-------------------|-----------------------------------------------------|
| Бэкенд/демон      | **Rust** — axum + tokio, `sqlx` (SQLite)            |
| Контейнеры        | **Docker** через `bollard`                          |
| Reverse proxy     | **Caddy** — wildcard TLS + Admin API (конфиг на лету)|
| State             | **SQLite**, один файл                               |
| Сборка приложений | **Только Dockerfile** (автодетект — фаза 3)         |
| Доступ            | **Один пользователь** (схема БД заложена под мульти) |
| Цель              | light linux, статический бинарь, install/uninstall  |

## Сущности (модель данных)

```
User (1 на старте, но таблица есть)
 └─ App           # репа + манифест swallow.yaml
     ├─ Build     # образ из коммита (sha), иммутабельный
     ├─ Instance  # запущенный контейнер из Build
     ├─ Route     # поддомен → инстанс(ы)
     └─ Secret    # env-переменные, шифрованные
```

`App → Build → Instance` даёт: откаты, несколько инстансов, blue-green деплой.

## Манифест `swallow.yaml` (в репе пользователя)

```yaml
name: my-app
dockerfile: ./Dockerfile
domain: my-app                # → my-app.<base-domain>
ports:
  - container: 3000           # порт внутри; наружу назначает Caddy
env:
  NODE_ENV: production
resources:
  cpu: "0.5"
  memory: "256m"
healthcheck:
  path: /health
scale:
  default_instances: 1
```

## Поток деплоя

```
push в master ─▶ webhook (HMAC) ─▶ очередь сборки ─▶ git clone @sha
   ─▶ docker build (тег=sha) ─▶ поднять Instance ─▶ healthcheck OK
   ─▶ Caddy переключает трафик ─▶ старый instance гасится
```

## Структура проекта

```
samoswallow/
├─ crates/
│  └─ swallowd/        # демон: axum API, control plane, builder, docker, caddy
├─ web/                # фронт (собирается в статику, вшивается в бинарь)
├─ scripts/
│  ├─ install.sh
│  └─ uninstall.sh
├─ swallow.example.yaml
└─ CONCEPT.md
```

Фронт собирается в статику и вшивается в бинарь (`rust-embed`) — один файл для раздачи.

## Дорожная карта

**Фаза 1 — MVP (ядро): ✅ готово**
1. ✅ Скелет: `swallowd` (axum), SQLite-схема + миграции, конфиг, systemd-юнит.
2. ✅ Docker-слой через `bollard`: build / run / stop / restart / logs / stats.
3. ✅ CRUD App + парсер `swallow.yaml`.
4. ✅ Деплой вручную из UI: clone → build → run Instance + ротация старых.
5. ✅ Caddy-интеграция: роут поддомена на инстанс.
6. ✅ Auth (пароль + сессия), веб-UI: список App, deploy/restart/stop, логи, нагрузка.
7. ✅ `install.sh` / `uninstall.sh` (+ авто-установка Caddy, dev-скрипты).

**Фаза 2 — в работе:**
- ✅ webhooks-автодеплой (push → пересборка, HMAC-подпись).
- ✅ откаты на прошлый билд (без пересборки, из иммутабельного образа).
- ✅ метрики во времени (фоновый сэмплер + спарклайны CPU/RAM, retention).
- ✅ просмотр логов инстансов в UI (с ограничением размера логов контейнера).
- ⏳ N инстансов + балансировка.

**Фаза 3:** автодетект стека (nixpacks), мульти-юзер + роли, бэкапы, secrets-менеджмент.
