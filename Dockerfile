FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY target/release/ecies_proxy /usr/local/bin/ecies_proxy

LABEL org.opencontainers.image.source="https://github.com/grevinden/ecies-reverse-proxy"
LABEL org.opencontainers.image.description="ECIES decrypting reverse proxy"

ENV LISTEN_ADDR=0.0.0.0:8080
EXPOSE 8080
CMD ["ecies_proxy"]