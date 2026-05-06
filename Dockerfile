FROM debian:bookworm-slim

# Обновление списка пакетов и установка корневых сертификатов
# Необходимо для проверки TLS-сертификатов при подключении к upstream по HTTPS.
# Если ваш upstream использует только HTTP, эту строку можно удалить.
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# Копирование бинарника с явным указанием прав на исполнение
COPY --link --chmod=+x target/release/ecies_proxy /usr/local/bin/ecies_proxy

# Устанавливаем рабочую директорию (опционально, но добавляет определённость)
WORKDIR /usr/local/bin

# Метки для связи с репозиторием
LABEL org.opencontainers.image.source="https://github.com/grevinden/ecies-reverse-proxy"
LABEL org.opencontainers.image.description="ECIES decrypting reverse proxy"

# Переменные окружения (настраиваются при запуске)
ENV LISTEN_ADDR=0.0.0.0:8080

# Порт приложения
EXPOSE 8080

# Запуск прокси с явным полным путём к исполняемому файлу
CMD ["/usr/local/bin/ecies_proxy"]