# 🔐 ECIES Reverse Proxy

[![Rust](https://img.shields.io/badge/rust-%23000000.svg?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Docker](https://img.shields.io/badge/docker-%230db7ed.svg?style=for-the-badge&logo=docker&logoColor=white)](https://www.docker.com/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

Высокопроизводительный асинхронный прокси‑сервер для прозрачной расшифровки ECIES‑пакетов в HTTP‑трафике.  
Позволяет развернуть end‑to‑end шифрование, не меняя исходный код ваших сервисов.

---

## 📖 Содержание

- [Принцип работы](#-принцип-работы)
- [Алгоритмы шифрования](#-алгоритмы-шифрования)
- [Ключевые преимущества](#-ключевые-преимущества)
- [Отказоустойчивость](#-отказоустойчивость)
- [Быстрый старт](#-быстрый-старт)
- [Конфигурация](#-конфигурация)
- [Пример использования](#-пример-использования)
- [Лицензия](#-лицензия)

---

## 🧠 Принцип работы

Прокси перехватывает HTTP‑запросы, находит в теле зашифрованные блоки вида `{{Base64URL_Safe_String}}`, расшифровывает их с помощью вашего приватного ключа X25519 и передаёт уже открытые данные на конечный сервер (upstream).  
Ваше приложение получает чистые данные, а клиент может не беспокоиться о том, что его трафик будет раскрыт — вся магия происходит в контейнере.

```mermaid
sequenceDiagram
    participant C as Клиент
    participant P as ECIES Proxy
    participant U as Ваш сервер (Upstream)

    C->>P: POST /data<br/>Body: {"msg": "{{зашифровано}}"}
    P->>P: Поиск по regex `{{ ... }}`
    P->>P: Извлечение Base64-строки
    P->>P: Расшифровка ECIES (X25519 + ChaCha20)
    P->>U: POST /data<br/>Body: {"msg": "расшифровано"}
    U-->>P: HTTP 200 OK
    P-->>C: HTTP 200 OK
```

---

## 🔐 Алгоритмы шифрования

Прокси реализует схему **ECIES** (Elliptic Curve Integrated Encryption Scheme) — одну из самых современных и быстрых схем асимметричного шифрования.

| Компонент | Алгоритм | Детали |
|-----------|----------|--------|
| **Соглашение о ключе** | X25519 (Curve25519) | Обмен 32‑байтными ключами. ~128 бит стойкости. |
| **Производный ключ** | HKDF‑SHA256 | info = `"ecies-chacha20-poly1305"` |
| **Симметричное шифрование** | ChaCha20‑Poly1305 | Аутентифицированное шифрование с 256‑битным ключом и 96‑битным nonce |

```mermaid
graph TD
    A[Зашифрованный пакет] -->|Base64 URL-safe| B(Декодирование)
    B --> C[32 байта: Ephemeral Public Key]
    B --> D[12 байт: Nonce]
    B --> E[Шифротекст + тег Poly1305]
    C --> F{X25519 ECDH}
    G[Ваш приватный ключ] --> F
    F --> H[Общий секрет]
    H --> I(HKDF-SHA256)
    I --> J[Симметричный ключ]
    J --> K(ChaCha20-Poly1305 Decrypt)
    D --> K
    E --> K
    K --> L[Открытый текст]
```

---

## 🚀 Ключевые преимущества

- 🔒 **End‑to‑End шифрование** – данные остаются зашифрованными до последнего звена перед вашим сервером.
- ⚡ **Высокая производительность** – асинхронный ввод‑вывод на `Tokio` и легковесный HTTP‑сервер `Hyper` позволяют обрабатывать тысячи соединений.
- 🧩 **Прозрачная интеграция** – никаких изменений в бизнес‑логике. Прокси полностью скрыт от остальной системы.
- 🎯 **Точечная замена** – расшифровываются только данные в шаблоне `{{...}}`, остальной контент остаётся нетронутым.
- 🛟 **Отказоустойчивость** – корректное завершение без потери запросов (Graceful Shutdown).
- 📦 **Простое развёртывание** – готовый Docker‑образ, конфигурация через переменные окружения.

---

## 🛡️ Отказоустойчивость

При получении сигнала `SIGTERM` или `SIGINT` прокси **не обрывает** соединения, а плавно завершает работу:

1. Перестаёт принимать новые TCP‑соединения.
2. Ожидает завершения обработки всех активных запросов (до 30 секунд).
3. Только после этого процесс корректно завершается.

```mermaid
stateDiagram-v2
    [*] --> Active
    Active --> ShutdownSignal : SIGTERM / SIGINT
    ShutdownSignal --> StopAccepting : Закрытие слушающего сокета
    StopAccepting --> WaitActiveConnections : Ожидание активных запросов
    WaitActiveConnections --> Exit : Все соединения завершены или тайм-аут 30с
    Exit --> [*]
```

Структурированное логирование позволяет наблюдать за состоянием через `docker compose logs`.

---

## 🏁 Быстрый старт

### 1. Подготовьте приватный ключ

Сгенерируйте ключи, например, с помощью Python или вашей 1С‑компоненты. Вам нужен **приватный** ключ в Base64 URL‑safe без паддинга, длиной 32 байта.

```bash
# Пример ключа (НЕ ИСПОЛЬЗОВАТЬ В БОЮ)
export ECIES_PRIVATE_KEY="Pz8_Pz8_Pz8_Pz8_Pz8_Pz8_Pz8_Pz8"
```

### 2. Запуск через Docker Compose

Создайте файл `docker-compose.yml`:

```yaml
services:
  proxy:
    image: ghcr.io/your-username/ecies-proxy:latest
    ports:
      - "8080:8080"
    environment:
      - ECIES_PRIVATE_KEY=${ECIES_PRIVATE_KEY}
      - UPSTREAM_URL=http://your-app:80
      - LISTEN_ADDR=0.0.0.0:8080
    restart: unless-stopped
```

Запустите:

```bash
docker compose up -d
```

---

## ⚙️ Конфигурация

| Переменная окружения | По умолчанию | Описание |
|----------------------|--------------|----------|
| `ECIES_PRIVATE_KEY` | *обязательно* | Приватный ключ ECIES в кодировке URL‑safe Base64 (без `=`). |
| `UPSTREAM_URL` | `http://localhost:8000` | Адрес вашего сервера, куда будут перенаправлены запросы. |
| `LISTEN_ADDR` | `0.0.0.0:8080` | Порт, на котором прокси принимает соединения. |

---

## 📮 Пример использования

```bash
# Зашифрованные данные (сгенерированы клиентом)
curl -X POST http://localhost:8080/ \
  -H "Content-Type: application/json" \
  -d '{"message": "{{зашифрованный_base64_здесь}}"}'
```

Прокси найдёт `{{зашифрованный_base64_здесь}}`, расшифрует его и отправит на `UPSTREAM_URL`:

```json
{
  "message": "секретные данные"
}
```

---

## 📄 Лицензия

Распространяется под лицензией **MIT**.


---

## Презентационные материалы

Ниже представлены слайды с диаграммами Mermaid и описанием. Код Mermaid можно вставить в [Mermaid Live Editor](https://mermaid.live/) или в любое markdown‑совместимое средство просмотра.

### Слайд 1: Проблема и решение

**Как передать секретные данные через публичный канал?**

```mermaid
graph LR
    A[Клиент] -->|Публичная сеть| B{??}
    B --> C[Ваш сервер]
    
    D[Клиент] -->|"TLS + ECIES в {{...}}"| E[ECIES Proxy]
    E -->|Локальная сеть / Доверенный канал| F[Ваш сервер]
    
    style A fill:#f9f,stroke:#333
    style D fill:#9f9,stroke:#333
    style E fill:#ff9,stroke:#333
    style F fill:#9f9,stroke:#333
```

### Слайд 2: Архитектура ECIES Proxy

```mermaid
sequenceDiagram
    autonumber
    participant C as Клиент
    box rgba(100,100,200,0.2) Докер контейнер
    participant P as Прокси (Hyper + Tokio)
    end
    participant U as Upstream сервер

    C->>P: HTTP запрос с {{зашифровано}}
    activate P
    Note over P: 1. Поиск всех {{...}}
    Note over P: 2. Декодирование Base64
    Note over P: 3. ECIES Расшифровка
    Note over P: 4. Замена в теле запроса
    P->>U: Проксированный запрос (открытые данные)
    activate U
    U-->>P: Ответ
    deactivate U
    P-->>C: Ответ клиенту
    deactivate P
    Note over P: Заголовок X-Decrypted-Count: 2
```

### Слайд 3: Детали алгоритма ECIES

```mermaid
graph TD
    subgraph "Входные данные"
    PKG[Зашифрованный пакет {{...}}]
    end

    subgraph "ECIES Decryption"
    PKG --> Decode[Base64 URL-safe Decode]
    Decode --> EphPub[Эфемерный публичный ключ<br/>32 байта]
    Decode --> Nonce[Nonce<br/>12 байт]
    Decode --> Cipher[Шифротекст + тег Poly1305]
    
    Priv[Ваш приватный ключ] --> DH{X25519 ECDH}
    EphPub --> DH
    DH --> Shared[Общий секрет]
    
    Shared --> HKDF[<b>HKDF-SHA256</b><br/>info=ecies-chacha20]
    HKDF --> SymKey[Симметричный ключ 32 байта]
    
    SymKey --> Decrypt{ChaCha20-Poly1305}
    Nonce --> Decrypt
    Cipher --> Decrypt
    Decrypt -->|✅ Успех| Plain[Открытый текст]
    Decrypt -->|❌ Ошибка| Err[Пакет не тронут]
    end
```

### Слайд 4: Производительность и Многопоточность

```mermaid
graph TD
    A[Слушающий сокет] -->|Accept| B(Tokio Task 1)
    A -->|Accept| C(Tokio Task 2)
    A -->|Accept| D(Tokio Task ...)
    
    subgraph "Tokio Thread Pool"
    B --> E[Hyper Connection]
    C --> F[Hyper Connection]
    D --> G[Hyper Connection]
    end
    
    E --> H[Расшифровка ECIES]
    F --> I[Расшифровка ECIES]
    G --> J[Расшифровка ECIES]
    
    H --> K[Запрос к Upstream]
    I --> L[Запрос к Upstream]
    J --> M[Запрос к Upstream]
```

### Слайд 5: Graceful Shutdown

```mermaid
stateDiagram-v2
    [*] --> Active : Запуск
    Active : Принимает соединения
    Active : Обрабатывает запросы
    
    Active --> ShutdownSignal : SIGTERM / SIGINT
    
    state ShutdownSignal {
        StopAccepting : Закрывает TCP listener
        StopAccepting : Перестаёт принимать новые соединения
        
        StopAccepting --> WaitActive
        WaitActive : Ожидает завершения активных запросов
        WaitActive : Логирует количество активных соединений
        
        WaitActive --> ForceExit : Тайм-аут 30 сек
        WaitActive --> CleanExit : Все соединения завершены
    }
    
    ForceExit --> [*]
    CleanExit --> [*]
```

### Слайд 6: Демонстрация

```bash
# 1. Запуск
docker compose up -d

# 2. Отправка зашифрованного запроса
curl -X POST http://localhost:8080/data \
  -H "Content-Type: application/json" \
  -d '{"payload": "{{encrypted_string_here}}"}'

# 3. Просмотр логов
docker compose logs -f

# 4. Остановка с Graceful Shutdown
docker compose stop
```

### Слайд 7: Заключение

- **Безопасность**: Современные алгоритмы X25519 + ChaCha20.
- **Простота**: Не требует изменений в вашем коде.
- **Надёжность**: Асинхронный Rust, Graceful Shutdown, логирование.
- **Открытый исходный код**: Лицензия MIT.

⭐ **Star us on GitHub!** ⭐