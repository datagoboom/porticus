use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::process;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};
use tracing_subscriber::filter::LevelFilter;

use porticus::fleet;
use porticus::{
    capture, config::PorticusConfig, framing::Framing, http, script, serial, websocket, BANNER,
};

#[derive(Subcommand)]
enum Command {
    /// Run as a fleet agent: bridge allowlisted serial ports to a hub
    Agent {
        /// Path to the agent TOML config
        #[arg(long)]
        config: String,
    },
    /// Fleet hub: run it, or manage its CA (init/enroll)
    Hub {
        #[command(subcommand)]
        action: HubCmd,
    },
}

#[derive(Subcommand)]
enum HubCmd {
    /// Run the hub (agent listener + client-facing server)
    Run {
        /// Address that agents dial into
        #[arg(long, default_value = "0.0.0.0:9000")]
        listen: String,
        /// Host for the client-facing server (/fleet, per-device ws)
        #[arg(long, default_value = "0.0.0.0")]
        http_host: String,
        /// Port for the client-facing server
        #[arg(long, default_value_t = 8080)]
        http_port: u16,
        /// Require mutual TLS from agents (run `hub init` first)
        #[arg(long)]
        tls: bool,
    },
    /// Generate the hub CA and server certificate
    Init {
        /// Address(es) agents will dial the hub at, added as certificate SANs
        #[arg(long = "san")]
        san: Vec<String>,
    },
    /// Issue a client-certificate bundle for an agent node
    Enroll {
        /// Node name (becomes the certificate's common name)
        node: String,
        /// Directory to write <node>.crt, <node>.key, ca.crt into
        #[arg(long, default_value = ".")]
        out: String,
    },
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Fleet subcommand (omit for standalone single-port bridge)
    #[command(subcommand)]
    command: Option<Command>,
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
    /// Write channel capacity (client-to-device messages buffered before send back-pressures)
    #[arg(long)]
    write_capacity: Option<usize>,
    /// Message framing: how the serial stream is split into messages
    #[arg(long, value_enum)]
    framing: Option<Framing>,
    /// Browser console HTTP port (0 disables the console)
    #[arg(long)]
    http_port: Option<u16>,
    /// Record the session (both directions) to a capture file
    #[arg(long, value_name = "FILE")]
    record: Option<String>,
    /// Replay a capture file as a virtual device (no serial port is opened)
    #[arg(long, value_name = "FILE")]
    replay: Option<String>,
    /// Run a send/expect script against the device, then exit with its result
    #[arg(long, value_name = "FILE")]
    script: Option<String>,
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

/// If another porticus is already running, returns its PID. Reads the PID file
/// and probes liveness with signal 0 (unix); a stale file (owning process gone)
/// is removed and treated as "no instance" so startup can take over. Other
/// platforms can't cheaply probe, so they always return `None` (no guard).
fn running_instance() -> Option<u32> {
    let pid_file = get_pid_file()?;
    let pid = fs::read_to_string(&pid_file)
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()?;
    #[cfg(unix)]
    {
        if unsafe { libc::kill(pid as i32, 0) } == 0 {
            return Some(pid);
        }
        let _ = fs::remove_file(&pid_file); // stale: owner is gone
        None
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
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

/// Dispatch a `hub` subcommand: run the hub, or manage its CA.
async fn run_hub_cmd(action: HubCmd) -> Result<(), Box<dyn Error>> {
    match action {
        HubCmd::Init { san } => {
            let dir = fleet::tls::init(san)?;
            println!("hub CA and server certificate written to {}", dir.display());
            Ok(())
        }
        HubCmd::Enroll { node, out } => {
            let fp = fleet::tls::enroll(&node, std::path::Path::new(&out))?;
            println!(
                "enrolled '{node}' (fingerprint {fp})\n\
                 bundle in {out}/: {node}.crt, {node}.key, ca.crt — copy these to the agent"
            );
            Ok(())
        }
        HubCmd::Run {
            listen,
            http_host,
            http_port,
            tls,
        } => run_hub(listen, http_host, http_port, tls).await,
    }
}

/// Runs the fleet hub: an agent listener and a client-facing listener, until a
/// shutdown signal.
async fn run_hub(
    listen: String,
    http_host: String,
    http_port: u16,
    tls: bool,
) -> Result<(), Box<dyn Error>> {
    let hub = fleet::hub::Hub::new();

    let tls_config = if tls {
        Some(fleet::tls::server_config()?)
    } else {
        warn!("hub running WITHOUT mTLS — agents connect over plain ws (use --tls for production)");
        None
    };

    let agent_listener = TcpListener::bind(&listen)
        .await
        .map_err(|e| format!("failed to bind agent listener {listen}: {e}"))?;
    let scheme = if tls { "wss" } else { "ws" };
    info!("hub: agents dial {scheme}://{listen}");

    let client_addr = format!("{http_host}:{http_port}");
    let client_listener = TcpListener::bind(&client_addr)
        .await
        .map_err(|e| format!("failed to bind client listener {client_addr}: {e}"))?;
    info!("hub: clients at http://{client_addr}  (GET /fleet, ws /<node>/<device>)");

    let agents = tokio::spawn(hub.clone().run_agent_listener(agent_listener, tls_config));
    let clients = tokio::spawn(hub.clone().run_client_listener(client_listener));

    tokio::select! {
        _ = shutdown_signal() => info!("shutting down"),
        _ = agents => {}
        _ = clients => {}
    }
    Ok(())
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

    // Fleet subcommands run their own event loop and never return to the
    // standalone bridge path below.
    match cli.command {
        Some(Command::Agent { config }) => {
            let cfg = fleet::config::load(&config)?;
            info!(node = %cfg.node, hub = %cfg.hub, ports = cfg.ports.len(), "starting fleet agent");
            tokio::select! {
                _ = fleet::agent::run(cfg) => {}
                _ = shutdown_signal() => info!("shutting down"),
            }
            return Ok(());
        }
        Some(Command::Hub { action }) => return run_hub_cmd(action).await,
        None => {}
    }

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
        write_capacity: cli.write_capacity.unwrap_or(defaults.write_capacity),
        framing: cli.framing.unwrap_or(defaults.framing),
        http_port: cli.http_port.unwrap_or(defaults.http_port),
    };

    // Refuse to start on top of a live instance; otherwise the second process
    // would overwrite the PID file and make --kill target the wrong one.
    if let Some(pid) = running_instance() {
        return Err(
            format!("porticus is already running (PID {pid}); use --kill to stop it").into(),
        );
    }

    write_pid_file()?;

    let (tx, _) = broadcast::channel(config.broadcast_capacity.max(1));
    let (serial_tx, serial_rx) = mpsc::channel(config.write_capacity.max(1));

    // Script mode is a headless, one-shot run: bring up the device, execute the
    // send/expect script, and exit with its pass/fail. No ws server or console.
    if let Some(path) = cli.script {
        let serial_task = tokio::spawn(serial::run(config.clone(), tx.clone(), serial_rx, None));
        let outcome = script::run(serial_tx, tx.subscribe(), path).await;
        serial_task.abort();
        remove_pid_file();
        return match outcome {
            Ok(()) => {
                info!("script passed");
                Ok(())
            }
            Err(e) => Err(format!("script failed: {e}").into()),
        };
    }

    if cli.replay.is_some() && cli.record.is_some() {
        warn!("--record is ignored in replay mode");
    }

    // The data source feeding clients is either a replayed capture (virtual
    // device) or the real serial port (optionally recorded).
    let source_task = if let Some(path) = cli.replay {
        tokio::spawn(capture::replay(tx.clone(), serial_rx, path))
    } else {
        let rec = cli.record.map(|path| {
            let (rec_tx, rec_rx) = mpsc::channel::<(capture::Dir, Vec<u8>)>(1024);
            tokio::spawn(capture::record(rec_rx, path));
            rec_tx
        });
        tokio::spawn(serial::run(config.clone(), tx.clone(), serial_rx, rec))
    };

    let addr = format!("{}:{}", config.websocket_host, config.websocket_port);
    let listener = match TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(e) => {
            remove_pid_file();
            return Err(format!("failed to bind {addr}: {e}").into());
        }
    };
    info!("websocket server listening on ws://{addr}");

    // Optional browser console on its own HTTP port. A bind failure disables the
    // console but never takes down the bridge.
    let console_task = if config.http_port != 0 {
        let http_addr = format!("{}:{}", config.websocket_host, config.http_port);
        match TcpListener::bind(&http_addr).await {
            Ok(http_listener) => {
                info!("console available at http://{http_addr}");
                Some(tokio::spawn(http::run(http_listener, config.clone())))
            }
            Err(e) => {
                warn!("console disabled: failed to bind {http_addr}: {e}");
                None
            }
        }
    } else {
        None
    };

    let result = tokio::select! {
        result = websocket::run(listener, tx, serial_tx) => result.map_err(Into::into),
        _ = shutdown_signal() => {
            info!("shutting down");
            Ok(())
        }
    };

    source_task.abort();
    if let Some(task) = console_task {
        task.abort();
    }
    remove_pid_file();
    result
}
