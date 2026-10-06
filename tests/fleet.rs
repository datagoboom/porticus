#![cfg(unix)]

//! Fleet hub routing: a simulated agent (raw ws + HELLO) and a client, exercising
//! the hub's registry and bidirectional device routing without real serial ports.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use porticus::fleet::hub::Hub;
use porticus::fleet::protocol::{decode_data, encode_data, DeviceInfo, Hello};

/// Starts a hub on ephemeral ports; returns (agent_url, client_base_url).
async fn start_hub() -> (String, String) {
    let hub = Hub::new();
    let agents = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let agent_addr = agents.local_addr().unwrap();
    let clients = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client_addr = clients.local_addr().unwrap();
    tokio::spawn(hub.clone().run_agent_listener(agents, None));
    tokio::spawn(hub.run_client_listener(clients));
    (format!("ws://{agent_addr}"), format!("ws://{client_addr}"))
}

#[tokio::test]
async fn hub_routes_between_agent_and_client() {
    let (agent_url, client_base) = start_hub().await;

    // Simulated agent: connect and announce one device "dev".
    let (mut agent, _) = connect_async(&agent_url).await.unwrap();
    let hello = Hello {
        node: "n1".into(),
        devices: vec![DeviceInfo {
            alias: "dev".into(),
            baud: 9600,
            framing: "raw".into(),
        }],
    };
    agent.send(Message::text(hello.to_json())).await.unwrap();

    // Let registration land, then connect a client to /n1/dev.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (mut client, _) = connect_async(format!("{client_base}/n1/dev"))
        .await
        .expect("client connect");
    tokio::time::sleep(Duration::from_millis(100)).await;

    // device -> client: the agent emits DATA(0, "ping"). Retry to absorb the
    // subscription race (messages before the client subscribed are dropped).
    let mut got = None;
    for _ in 0..25 {
        agent
            .send(Message::binary(encode_data(0, b"ping")))
            .await
            .unwrap();
        if let Ok(Some(Ok(msg))) = timeout(Duration::from_millis(200), client.next()).await {
            got = Some(msg.into_data());
            break;
        }
    }
    assert_eq!(
        got.as_deref(),
        Some(&b"ping"[..]),
        "client never got device data"
    );

    // client -> device: the client's write is routed back to the agent as a
    // DATA frame tagged with device index 0.
    client
        .send(Message::binary(b"pong".as_ref()))
        .await
        .unwrap();
    let mut routed = None;
    for _ in 0..25 {
        match timeout(Duration::from_millis(300), agent.next()).await {
            Ok(Some(Ok(Message::Binary(frame)))) => {
                if let Some((idx, payload)) = decode_data(frame.as_ref()) {
                    routed = Some((idx, payload.to_vec()));
                    break;
                }
            }
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }
    assert_eq!(
        routed,
        Some((0u16, b"pong".to_vec())),
        "client write not routed to agent"
    );
}

#[tokio::test]
async fn fleet_registry_lists_devices() {
    let (agent_url, client_base) = start_hub().await;

    let (mut agent, _) = connect_async(&agent_url).await.unwrap();
    let hello = Hello {
        node: "bench".into(),
        devices: vec![
            DeviceInfo {
                alias: "uno".into(),
                baud: 115200,
                framing: "line".into(),
            },
            DeviceInfo {
                alias: "gps".into(),
                baud: 9600,
                framing: "raw".into(),
            },
        ],
    };
    agent.send(Message::text(hello.to_json())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Raw HTTP GET /fleet against the client listener.
    let host = client_base.strip_prefix("ws://").unwrap();
    let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream
        .write_all(b"GET /fleet HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    timeout(Duration::from_secs(2), stream.read_to_end(&mut buf))
        .await
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(&buf);
    assert!(
        text.contains("\"node\":\"bench\""),
        "fleet missing node: {text}"
    );
    assert!(
        text.contains("\"alias\":\"uno\""),
        "fleet missing uno: {text}"
    );
    assert!(
        text.contains("\"alias\":\"gps\""),
        "fleet missing gps: {text}"
    );
}
