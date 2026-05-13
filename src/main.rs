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

    let interval = cli.interval;
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(interval))
                .await;

            if let Err(err) = capture(&cli.storage_dir, &cli.bucket).await {
                eprintln!(
                    "capture error: {err}"
                );
            }
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