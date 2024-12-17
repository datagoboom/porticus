use clap::Parser;
use directories::ProjectDirs;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::process;
use tokio::sync::broadcast;
use std::sync::Arc;
use porticus::{config::PorticusConfig, serial, websocket, BANNER};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    port: Option<String>,
    #[arg(short, long)]
    baud: Option<u32>,
    #[arg(short, long)]
    websocket_port: Option<u16>,
    #[arg(long)]
    websocket_host: Option<String>,
    #[arg(long)]
    buffer_size: Option<usize>,
    #[arg(long)]
    broadcast_capacity: Option<usize>,
    #[arg(long)]
    kill: bool,
    #[arg(long)]
    debug: bool,
    #[arg(short = 'q', long, help = "Silence all output")]
    quiet: bool,
}

fn get_pid_file() -> Option<PathBuf> {
    ProjectDirs::from("com", "porticus", "porticus").map(|proj_dirs| {
        proj_dirs.config_dir().join("porticus.pid")
    })
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

fn kill_running_instance() -> Result<(), Box<dyn Error>> {
    if let Some(pid_file) = get_pid_file() {
        if pid_file.exists() {
            let pid = fs::read_to_string(&pid_file)?.parse::<u32>()?;
            #[cfg(unix)]
            {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
            #[cfg(windows)]
            {
                println!("Process termination not implemented for Windows");
            }
            fs::remove_file(pid_file)?;
            println!("Killed process with PID: {}", pid);
        }
    }
    Ok(())
}

macro_rules! print_if_not_quiet {
    ($quiet:expr, $($arg:tt)*) => {
        if !$quiet {
            println!($($arg)*);
        }
    };
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    if cli.kill {
        return kill_running_instance();
    }

    print_if_not_quiet!(cli.quiet, "{}", BANNER);
    
    write_pid_file()?;

    let config = PorticusConfig {
        serial_port: cli.port.unwrap_or_else(|| {
            if cfg!(windows) { "COM1".to_string() } else { "/dev/ttyUSB0".to_string() }
        }),
        baud_rate: cli.baud.unwrap_or(9600),
        websocket_port: cli.websocket_port.unwrap_or(8080),
        websocket_host: cli.websocket_host.unwrap_or_else(|| "127.0.0.1".to_string()),
        buffer_size: cli.buffer_size.unwrap_or(1024),
        broadcast_capacity: cli.broadcast_capacity.unwrap_or(16),
    };

    print_if_not_quiet!(cli.quiet, "Starting Porticus...");
    print_if_not_quiet!(cli.quiet, "Opening serial port: {} at {} baud", config.serial_port, config.baud_rate);

    let serial_port = serial::create_serial_port(&config).await?;
    let serial_reader = serial_port.clone();
    let serial_writer = serial_port.clone();

    let (tx, _) = broadcast::channel(config.broadcast_capacity);
    let tx = Arc::new(tx);
    let tx_serial = tx.clone();

    tokio::spawn(async move {
        serial::handle_serial_reading(serial_reader, tx_serial, config.buffer_size, cli.quiet).await;
    });

    websocket::run_websocket_server(&config, tx, serial_writer, cli.quiet).await?;

    Ok(())
}
