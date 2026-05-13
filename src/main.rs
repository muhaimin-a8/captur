use anyhow::{anyhow, Result};
use chrono::Utc;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use screenshots::Screen;
use s3s::auth::SimpleAuth;
use s3s::service::S3ServiceBuilder;
use s3s_fs::FileSystem;
use std::sync::Arc;
use tokio::{
    net::TcpListener,
    time::{sleep, Duration},
};

const STORAGE_DIR: &str = "./data";
const BUCKET: &str = "captur";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    tokio::fs::create_dir_all(format!(
        "{}/{}",
        STORAGE_DIR,
        BUCKET
    ))
    .await?;

    let fs = FileSystem::new(STORAGE_DIR)
        .map_err(|e| anyhow!("{e:?}"))?;

    let auth = SimpleAuth::from_single(
        "captur",
        "captur123",
    );

    let mut builder =
        S3ServiceBuilder::new(fs);

    builder.set_auth(auth);

    let service =
        Arc::new(builder.build().into_shared());

    let listener =
        TcpListener::bind("0.0.0.0:8014")
            .await?;

    println!(
        "S3 server running at http://localhost:8014"
    );

    tokio::spawn(async {
        loop {
            sleep(Duration::from_secs(3))
                .await;

            if let Err(err) = capture().await {
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

async fn capture() -> Result<()> {
    let screen = Screen::all()?
        .into_iter()
        .next()
        .unwrap();

    let image = screen.capture()?;

    let filename = format!(
        "{}/{}/{}.png",
        STORAGE_DIR,
        BUCKET,
        Utc::now().timestamp()
    );

    image.save(&filename)?;

    println!("saved {}", filename);

    Ok(())
}