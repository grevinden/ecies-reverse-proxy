mod ecies;

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use regex::Regex;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::time::timeout;

struct AppState {
    upstream: String,
    secret_key: [u8; 32],
    base64_re: Regex,
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    state: Arc<AppState>,
) -> Result<Response<Full<Bytes>>, Box<dyn std::error::Error>> {
    if req.method() != Method::POST && req.method() != Method::PUT {
        let resp = Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Full::from(Bytes::from("Only POST/PUT allowed")))?;
        return Ok(resp);
    }

    // Собираем тело
    let body_bytes = req.collect().await?.to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes).to_string();

    let mut decrypted_count = 0u32;
    let mut modified = body_str.clone();

    // Поиск всех потенциальных ECIES-пакетов
    for cap in state.base64_re.find_iter(&body_str) {
        let candidate = cap.as_str();
        if let Ok(plain) = ecies::decrypt(candidate, &state.secret_key) {
            modified = modified.replace(candidate, &plain);
            decrypted_count += 1;
        }
    }

    // Проксирование запроса к upstream
    let upstream_url = format!(
        "{}{}",
        state.upstream,
        req.uri()
            .path_and_query()
            .map(|p| p.as_str())
            .unwrap_or("/")
    );

    let client_req: Request<Full<D>> = Request::builder()
        .method(req.method())
        .uri(&upstream_url)
        .header("X-Decrypted-Count", decrypted_count.to_string())
        .body(Full::from(Bytes::from(modified)))
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

    let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
        .build_http();

    match client.request(client_req).await {
        Ok(resp) => {
            // Проксируем ответ обратно клиенту
            let (parts, body) = resp.into_parts();
            let body_bytes = body.collect().await?.to_bytes();
            let response = Response::from_parts(parts, Full::from(body_bytes));
            Ok(response)
        }
        Err(e) => {
            let msg = format!("Upstream error: {}", e);
            let resp = Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Full::from(Bytes::from(msg)))?;
            Ok(resp)
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Чтение конфигурации из переменных окружения
    let private_key_b64 =
        env::var("ECIES_PRIVATE_KEY").expect("ECIES_PRIVATE_KEY environment variable is required");
    let secret_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&private_key_b64)
        .expect("Failed to decode private key base64");
    let secret_key: [u8; 32] = secret_bytes
        .as_slice()
        .try_into()
        .expect("Private key must be 32 bytes");

    let upstream =
        env::var("UPSTREAM_URL").unwrap_or_else(|_| "http://localhost:8000".to_string());
    let listen_addr =
        env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let base64_re = Regex::new(r"[A-Za-z0-9_-]{44,}")?;

    let state = Arc::new(AppState {
        upstream,
        secret_key,
        base64_re,
    });

    let addr: SocketAddr = listen_addr.parse()?;
    let listener = TcpListener::bind(&addr).await?;
    println!("ECIES decrypting proxy listening on {}", listen_addr);
    println!("Upstream: {}", state.upstream);

    // Механизм оповещения о завершении
    let shutdown = Arc::new(Notify::new());
    let shutdown_clone = shutdown.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        println!("Shutdown signal received, initiating graceful shutdown...");
        shutdown_clone.notify_waiters();
    });

    // Основной цикл приёма соединений
    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _)) => {
                        let io = TokioIo::new(stream);
                        let state = state.clone();
                        let shutdown = shutdown.clone();

                        tokio::spawn(async move {
                            let svc = service_fn(move |req| {
                                let state = state.clone();
                                async move { handle_request(req, state).await.map_err(|e| {
                                    eprintln!("Request handling error: {}", e);
                                    hyper::Error::new(std::io::ErrorKind::Other, e)
                                }) }
                            });

                            let conn = http1::Builder::new().serve_connection(io, svc);
                            tokio::select! {
                                _ = conn => {},
                                _ = shutdown.notified() => {
                                    // Даём завершиться текущему запросу
                                    let _ = timeout(Duration::from_secs(10), conn).await;
                                }
                            }
                        });
                    },
                    Err(e) => {
                        eprintln!("Accept error: {}", e);
                    }
                }
            }
            _ = shutdown.notified() => {
                println!("Stop accepting new connections. Waiting for active connections...");
                drop(listener);
                // Ждём завершения всех активных задач (упрощённо — пауза, можно более точно через счетчик)
                tokio::time::sleep(Duration::from_secs(5)).await;
                break;
            }
        }
    }

    println!("Server gracefully shut down");
    Ok(())
}