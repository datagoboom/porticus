use clap::Parser;
use directories::ProjectDirs;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::process;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};
use tracing::info;
use tracing_subscriber::filter::LevelFilter;

use porticus::{config::PorticusConfig, serial, websocket, BANNER};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Serial port path (e.g. /dev/ttyACM0, COM1)
    #[arg(short, long)]
    port: Option<String>,
    /// Baud rate
    #[arg(short, long)]
    baud: Option<u32>,
    /// WebSocket listen port
    #[arg(short, long)]
    websocket_port: Option<u16>,
    /// WebSocket listen host
    #[arg(long)]
    websocket_host: Option<String>,
    /// Serial read buffer size in bytes
    #[arg(long)]
    buffer_size: Option<usize>,
    /// Broadcast channel capacity (messages buffered per client)
    #[arg(long)]
    broadcast_capacity: Option<usize>,
    /// Kill running instance
    #[arg(long)]
    kill: bool,
    /// Enable debug logging
    #[arg(long)]
    debug: bool,
    /// Silence all output
    #[arg(short = 'q', long)]
    quiet: bool,
}

fn get_pid_file() -> Option<PathBuf> {
    ProjectDirs::from("com", "porticus", "porticus")
        .map(|proj_dirs| proj_dirs.config_dir().join("porticus.pid"))
}

fn write_pid_file() -> Result<(), Box<dyn Error>> {
    if let Some(pid_file) = get_pid_file() {
        if let Some(parent) = pid_file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&pid_file, process::id().to_string())?;
    }
    Ok(())
}

fn remove_pid_file() {
    if let Some(pid_file) = get_pid_file() {
        let _ = fs::remove_file(pid_file);
    }
}

fn kill_running_instance() -> Result<(), Box<dyn Error>> {
    let Some(pid_file) = get_pid_file() else {
        return Ok(());
    };
    if !pid_file.exists() {
        println!("No running instance found");
        return Ok(());
    }
    let pid = fs::read_to_string(&pid_file)?.trim().parse::<u32>()?;
    #[cfg(unix)]
    {
        // Probe first so a stale PID file doesn't SIGTERM an unrelated process
        // that happens to have reused the PID after a reboot.
        if unsafe { libc::kill(pid as i32, 0) } == 0 {
            unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            println!("Killed process with PID: {pid}");
        } else {
            println!("Stale PID file (process {pid} not running), removing it");
        }
    }
    #[cfg(windows)]
    {
        println!("Process termination not implemented for Windows (PID: {pid})");
    }
    fs::remove_file(pid_file)?;
    Ok(())
}

/// Resolves on Ctrl-C, or on SIGTERM on unix (what `--kill` sends), so the
/// PID file gets cleaned up either way.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    if cli.kill {
        return kill_running_instance();
    }

    let level = if cli.quiet {
        LevelFilter::OFF
    } else if cli.debug {
        LevelFilter::DEBUG
    } else {
        LevelFilter::INFO
    };
    tracing_subscriber::fmt().with_max_level(level).init();

    if !cli.quiet {
        println!("{BANNER}");
    }

    let defaults = PorticusConfig::default();
    let config = PorticusConfig {
        serial_port: cli.port.unwrap_or(defaults.serial_port),
        baud_rate: cli.baud.unwrap_or(defaults.baud_rate),
        websocket_port: cli.websocket_port.unwrap_or(defaults.websocket_port),
        websocket_host: cli.websocket_host.unwrap_or(defaults.websocket_host),
        buffer_size: cli.buffer_size.unwrap_or(defaults.buffer_size),
        broadcast_capacity: cli
            .broadcast_capacity
            .unwrap_or(defaults.broadcast_capacity),
    };

    write_pid_file()?;

    let (tx, _) = broadcast::channel(config.broadcast_capacity.max(1));
    let (serial_tx, serial_rx) = mpsc::channel(32);

    let serial_task = tokio::spawn(serial::run(config.clone(), tx.clone(), serial_rx));

    let addr = format!("{}:{}", config.websocket_host, config.websocket_port);
    let listener = match TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(e) => {
            remove_pid_file();
            return Err(format!("failed to bind {addr}: {e}").into());
        }
    };
    info!("websocket server listening on ws://{addr}");

    let result = tokio::select! {
        result = websocket::run(listener, tx, serial_tx) => result.map_err(Into::into),
        _ = shutdown_signal() => {
            info!("shutting down");
            Ok(())
        }
    };

    serial_task.abort();
    remove_pid_file();
    result
}
