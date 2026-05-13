use anyhow::{anyhow, Result};
use chrono::Utc;
use clap::Parser;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use screenshots::Screen;
use s3s::auth::SimpleAuth;
use s3s::service::S3ServiceBuilder;
use s3s_fs::FileSystem;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::{
    net::TcpListener,
    time::{sleep, Duration},
};

#[derive(Parser, Debug)]
#[command(name = "captur")]
// #[command(about = "S3-based screen capture server")]
struct Cli {
    #[arg(long, default_value = "8014")]
    port: u16,

    #[arg(long, default_value = "8016")]
    config_port: u16,

    #[arg(long, default_value = "3")]
    interval: u64,

    #[arg(long, default_value = "captur")]
    key_id: String,

    #[arg(long, default_value = "captur123")]
    secret_key: String,

    #[arg(long, default_value = "./data")]
    storage_dir: String,

    #[arg(long, default_value = "captur")]
    bucket: String,
}

fn get_local_ips() -> Vec<IpAddr> {
    let mut ips = Vec::new();
    let external = "8.8.8.8:80";
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        let _ = socket.connect(external);
        if let Ok(local) = socket.local_addr() {
            ips.push(local.ip());
        }
    }
    if ips.is_empty() {
        if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
            let _ = socket.connect("1.1.1.1:80");
            if let Ok(local) = socket.local_addr() {
                ips.push(local.ip());
            }
        }
    }
    ips
}

async fn run_config_server(port: u16, interval: Arc<AtomicU64>, running: Arc<AtomicBool>) -> Result<()> {
    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).await?;

    println!("Config server running at http://localhost:{}", port);

    loop {
        let (stream, _) = listener.accept().await?;
        let interval = interval.clone();
        let running = running.clone();

        tokio::spawn(async move {
            let _ = handle_config_request(stream, interval, running).await;
        });
    }
}

async fn handle_config_request(mut stream: tokio::net::TcpStream, interval: Arc<AtomicU64>, running: Arc<AtomicBool>) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut buffer = [0u8; 4096];
    let n = stream.read(&mut buffer).await?;

    let request = String::from_utf8_lossy(&buffer[..n]);

    let (status, body, content_type) = if request.starts_with("GET /") && (request.starts_with("GET / ") || request.starts_with("GET /index.html")) {
        let html = tokio::fs::read_to_string("index.html").await.unwrap_or_else(|_| {
            "<html><body><h1>index.html not found</h1></body></html>".to_string()
        });
        ("200 OK", html, "text/html")
    } else if request.starts_with("GET /interval") {
        let current = interval.load(Ordering::Relaxed);
        let response = format!("{{\"interval\":{}}}\n", current);
        ("200 OK", response, "application/json")
    } else if request.starts_with("POST /interval") {
        let body_start = request.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0);
        let body_str = &request[body_start..];

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(body_str) {
            if let Some(interval_val) = json.get("interval").and_then(|v| v.as_u64()) {
                interval.store(interval_val, Ordering::Relaxed);
                println!("[CONFIG] interval changed to {}s", interval_val);
                let response = format!("{{\"interval\":{}}}\n", interval_val);
                ("200 OK", response, "application/json")
            } else {
                ("400 Bad Request", "{\"error\":\"missing interval field\"}\n".to_string(), "application/json")
            }
        } else {
            ("400 Bad Request", "{\"error\":\"invalid json\"}\n".to_string(), "application/json")
        }
    } else if request.starts_with("GET /capture") {
        let is_running = running.load(Ordering::Relaxed);
        let response = format!("{{\"running\":{}}}\n", is_running);
        ("200 OK", response, "application/json")
    } else if request.starts_with("POST /capture") {
        let body_start = request.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0);
        let body_str = &request[body_start..];

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(body_str) {
            if let Some(running_val) = json.get("running").and_then(|v| v.as_bool()) {
                running.store(running_val, Ordering::Relaxed);
                println!("[CONFIG] capture {}", if running_val { "started" } else { "stopped" });
                let response = format!("{{\"running\":{}}}\n", running_val);
                ("200 OK", response, "application/json")
            } else {
                ("400 Bad Request", "{\"error\":\"missing running field\"}\n".to_string(), "application/json")
            }
        } else {
            ("400 Bad Request", "{\"error\":\"invalid json\"}\n".to_string(), "application/json")
        }
    } else {
        ("404 Not Found", "{\"error\":\"not found\"}\n".to_string(), "application/json")
    };

    let mut stream = stream;
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Length: {}\r\nContent-Type: {}\r\n\r\n{}",
        status,
        body.len(),
        content_type,
        body
    );
    stream.write_all(response.as_bytes()).await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let cli = Cli::parse();

    tokio::fs::create_dir_all(format!(
        "{}/{}",
        cli.storage_dir,
        cli.bucket
    ))
    .await?;

    let fs = FileSystem::new(&cli.storage_dir)
        .map_err(|e| anyhow!("{e:?}"))?;

    let auth = SimpleAuth::from_single(
        cli.key_id,
        s3s::auth::SecretKey::from(cli.secret_key),
    );

    let mut builder =
        S3ServiceBuilder::new(fs);

    builder.set_auth(auth);

    let service =
        Arc::new(builder.build().into_shared());

    let addr = format!("0.0.0.0:{}", cli.port);
    let listener =
        TcpListener::bind(&addr)
            .await?;

    let ips = get_local_ips();
    let ip_str = if ips.is_empty() {
        String::new()
    } else {
        let ip_list: Vec<String> = ips.iter().map(|ip| format!("http://{}:{}", ip, cli.port)).collect();
        format!(" | {}", ip_list.join(" | "))
    };

    println!(
        "S3 server running at http://localhost:{}{}",
        cli.port, ip_str
    );

    let interval = Arc::new(AtomicU64::new(cli.interval));
    let running = Arc::new(AtomicBool::new(true));
    let interval_clone = interval.clone();
    let running_clone = running.clone();
    let storage_dir = cli.storage_dir.clone();
    let bucket = cli.bucket.clone();

    tokio::spawn(async move {
        loop {
            let current_interval = interval_clone.load(Ordering::Relaxed);
            sleep(Duration::from_secs(current_interval))
                .await;

            if running_clone.load(Ordering::Relaxed) {
                if let Err(err) = capture(&storage_dir, &bucket).await {
                    eprintln!(
                        "capture error: {err}"
                    );
                }
            }
        }
    });

    let config_interval = interval.clone();
    let config_running = running.clone();
    tokio::spawn(async move {
        if let Err(e) = run_config_server(cli.config_port, config_interval, config_running).await {
            eprintln!("config server error: {}", e);
        }
    });

    loop {
        let (stream, _) =
            listener.accept().await?;

        let io = TokioIo::new(stream);

        let service = service.clone();

        tokio::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service)
                .await
            {
                eprintln!(
                    "server error: {err}"
                );
            }
        });
    }
}

async fn capture(storage_dir: &str, bucket: &str) -> Result<()> {
    use image::{ImageBuffer, RgbaImage};

    let screens = Screen::all()?;

    let mut captures = vec![];

    let mut total_width = 0;
    let mut max_height = 0;

    for screen in screens {
        let img = screen.capture()?;

        total_width += img.width();

        max_height = max_height.max(img.height());

        captures.push(img);
    }

    let mut canvas: RgbaImage = ImageBuffer::new(total_width, max_height);

    let mut offset_x = 0;

    for img in captures {
        let width = img.width();
        let height = img.height();
        let buffer = img.into_raw();

        let rgba = RgbaImage::from_raw(width, height, buffer).unwrap();

        image::imageops::overlay(&mut canvas, &rgba, offset_x.into(), 0);

        offset_x += rgba.width();
    }

    let filename = format!(
        "{}/{}/{}.png",
        storage_dir,
        bucket,
        Utc::now().timestamp()
    );

    canvas.save(&filename)?;

    println!("saved {}", filename);

    Ok(())
}