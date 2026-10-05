use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};
use tracing::{debug, info, warn};

use crate::error::PorticusError;

/// Accepts WebSocket clients forever. Each client runs in its own task, so a
/// slow client or a failed handshake never affects the others.
pub async fn run(
    listener: TcpListener,
    tx: broadcast::Sender<Vec<u8>>,
    serial_tx: mpsc::Sender<Vec<u8>>,
) -> Result<(), PorticusError> {
    loop {
        let (stream, addr) = listener.accept().await?;
        let rx = tx.subscribe();
        let serial_tx = serial_tx.clone();
        tokio::spawn(async move {
            match accept_async(stream).await {
                Ok(ws) => {
                    info!(%addr, "client connected");
                    handle_client(ws, rx, serial_tx).await;
                    info!(%addr, "client disconnected");
                }
                Err(e) => warn!(%addr, "websocket handshake failed: {e}"),
            }
        });
    }
}

async fn handle_client<S>(
    ws: WebSocketStream<S>,
    mut rx: broadcast::Receiver<Vec<u8>>,
    serial_tx: mpsc::Sender<Vec<u8>>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut sender, mut receiver) = ws.split();
    loop {
        tokio::select! {
            msg = receiver.next() => {
                match msg {
                    Some(Ok(msg)) if msg.is_binary() || msg.is_text() => {
                        if serial_tx.send(msg.into_data().to_vec()).await.is_err() {
                            break; // serial task gone
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // ping/pong handled by tungstenite
                    Some(Err(e)) => {
                        debug!("websocket receive error: {e}");
                        break;
                    }
                }
            }
            data = rx.recv() => {
                match data {
                    Ok(data) => {
                        if sender.send(Message::Binary(data.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("client too slow, dropped {n} serial messages");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}
