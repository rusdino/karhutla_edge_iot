use clap::Parser;
use karhutla_protocol::{topics, Scenario, SimControlRequest};
use karhutla_server::{build_router_with_web_dir, EngineConfig, KarhutlaEngine};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "karhutla-server", about = "Karhutla Edge-IoT Rust Telemetry & Control Server")]
struct Args {
    #[arg(long, default_value = "0.0.0.0")]
    host: String,
    #[arg(long, default_value_t = 8080)]
    port: u16,
    #[arg(long, default_value = topics::BROKER_HOST_DEFAULT)]
    broker_host: String,
    #[arg(long, default_value_t = topics::BROKER_PORT_DEFAULT)]
    broker_port: u16,
    #[arg(long, default_value = "logs")]
    out: PathBuf,
    #[arg(long)]
    no_mqtt: bool,
    #[arg(long)]
    auto_sim: Option<String>,
    #[arg(long, default_value_t = 1.0)]
    speed: f64,
    #[arg(long)]
    ai_preempt: bool,
    #[arg(long)]
    web_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "karhutla_server=info".into()),
        )
        .init();

    let args = Args::parse();
    let engine = KarhutlaEngine::new(EngineConfig {
        broker_host: args.broker_host.clone(),
        broker_port: args.broker_port,
        out_dir: args.out.clone(),
        enable_mqtt: !args.no_mqtt,
        esp_stale_s: 5.0,
    });

    if let Some(sc_str) = args.auto_sim.as_deref() {
        if Scenario::parse(sc_str).is_some() {
            let _ = engine
                .control_sim(SimControlRequest {
                    action: Some("start".to_string()),
                    scenario: Some(sc_str.to_string()),
                    speed: Some(args.speed),
                    ai_preempt: Some(args.ai_preempt),
                    ..Default::default()
                })
                .await;
        }
    }

    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .expect("invalid bind address");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind HTTP listener");
    let local_addr = listener.local_addr().unwrap();
    println!(
        "Karhutla Rust Server aktif di http://localhost:{}/ (LAN: http://{}:{}/)",
        local_addr.port(),
        engine.lan_ip(),
        local_addr.port()
    );
    println!(
        "MQTT: {}:{} (aktif: {})",
        args.broker_host,
        args.broker_port,
        !args.no_mqtt
    );

    if let Some(ref dir) = args.web_dir {
        if dir.join("index.html").is_file() {
            println!("Web UI statis disajikan dari: {}", dir.display());
        } else {
            println!(
                "Peringatan: --web-dir '{}' tidak memiliki index.html, rute statis dinonaktifkan.",
                dir.display()
            );
        }
    }

    let router = build_router_with_web_dir(engine.clone(), args.web_dir.clone());
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            println!("\nMenutup Karhutla Rust Server...");
            engine.shutdown().await;
        })
        .await
        .expect("HTTP server error");
}
