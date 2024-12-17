use futures::{SinkExt, StreamExt, Sink};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, Mutex};
use tokio_tungstenite::{
    accept_async,
    tungstenite::{Message, protocol::frame::Payload},
};
use std::sync::Arc;
use std::fmt::Debug;
use crate::config::PorticusConfig;
use crate::error::PorticusError;

pub async fn run_websocket_server(
    config: &PorticusConfig,
    tx: Arc<broadcast::Sender<Vec<u8>>>,
    serial_writer: Arc<Mutex<dyn tokio_serial::SerialPort>>,
    quiet: bool,
) -> Result<(), PorticusError> {
    let addr = format!("{}:{}", config.websocket_host, config.websocket_port);
    let listener = TcpListener::bind(&addr).await?;

    while let Ok((stream, addr)) = listener.accept().await {
        if !quiet {
            println!("New client connected: {}", addr);
        }
        
        let ws_stream = accept_async(stream).await?;
        let (ws_sender, ws_receiver) = ws_stream.split();
        let rx = tx.subscribe();
        let serial_writer = serial_writer.clone();

        handle_client_connection(
            ws_sender,
            ws_receiver,
            rx,
            serial_writer,
            quiet,
        ).await;
    }
    Ok(())
}

async fn handle_client_connection<S, R>(
    mut ws_sender: S,
    mut ws_receiver: R,
    mut rx: broadcast::Receiver<Vec<u8>>,
    serial_writer: Arc<Mutex<dyn tokio_serial::SerialPort>>,
    quiet: bool,
) where
    S: Sink<Message> + Unpin + Send + 'static,
    S::Error: Debug,
    R: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin + Send + 'static,
{
    let write_handle = tokio::spawn(async move {
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(msg) => {
                    if msg.is_binary() || msg.is_text() {
                        let data = msg.into_data();
                        if let Err(e) = serial_writer.lock().await.write_all(data.as_slice()) {
                            if !quiet {
                                eprintln!("Failed to write to serial port: {:?}", e);
                            }
                            break;
                        }
                    }
                }
                Err(e) => {
                    if !quiet {
                        eprintln!("WebSocket receive error: {:?}", e);
                    }
                    break;
                }
            }
        }
    });

    let read_handle = tokio::spawn(async move {
        while let Ok(data) = rx.recv().await {
            let message = Message::Binary(Payload::Vec(data));
            if let Err(e) = ws_sender.send(message).await {
                if !quiet {
                    eprintln!("Failed to send WebSocket message: {:?}", e);
                }
                break;
            }
        }
    });

    tokio::select! {
        _ = write_handle => (),
        _ = read_handle => (),
    }
}
