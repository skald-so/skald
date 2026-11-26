//! Fault injection on one WebSocket in the disposable loopback fixture.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use skald_proto::{Cbor, Frame, PROTOCOL_VERSION, Push, subprotocol};
use tokio::net::TcpListener;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::{Message as BackendMessage, client::IntoClientRequest};

#[derive(Clone, Copy)]
pub enum Fault {
    DelaySecondPush,
    DropAck,
}

#[derive(Clone)]
pub struct Relay {
    fault: Fault,
    armed: Arc<AtomicBool>,
    connections: Arc<AtomicUsize>,
}

impl Relay {
    pub fn new(fault: Fault) -> Self {
        Self {
            fault,
            armed: Arc::default(),
            connections: Arc::default(),
        }
    }

    pub fn arm(&self) {
        self.armed.store(true, Ordering::Relaxed);
    }

    pub async fn serve(self, front: TcpListener, app: Router) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let backend = listener.local_addr().unwrap();
        let rest = app.clone();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let frontend = Router::new()
            .route(
                "/ws",
                get(move |upgrade: WebSocketUpgrade| {
                    let relay = self.clone();
                    async move {
                        // The third socket is a receive-only member in these three-device tests.
                        let selected = relay.connections.fetch_add(1, Ordering::Relaxed) == 2;
                        upgrade
                            .protocols([subprotocol(PROTOCOL_VERSION)])
                            .on_upgrade(move |ws| bridge(ws, backend, selected.then_some(relay)))
                    }
                }),
            )
            .fallback_service(rest);
        axum::serve(front, frontend).await.unwrap();
    }
}

async fn bridge(mut front: WebSocket, backend: SocketAddr, relay: Option<Relay>) {
    let mut request = format!("ws://{backend}/ws").into_client_request().unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        subprotocol(PROTOCOL_VERSION).parse().unwrap(),
    );
    let (mut back, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let mut measured_pushes = 0;
    let mut drop_ack = None;
    loop {
        tokio::select! {
            from_client = front.next() => {
                let Some(Ok(Message::Binary(bytes))) = from_client else { break };
                if let Ok(Frame::Ack { seq }) = Frame::from_cbor(&bytes)
                    && drop_ack == Some(seq)
                {
                    continue;
                }
                if back.send(BackendMessage::Binary(bytes)).await.is_err() {
                    break;
                }
            }
            from_server = back.next() => {
                let Some(Ok(BackendMessage::Binary(bytes))) = from_server else { break };
                if let Some(relay) = &relay
                    && relay.armed.load(Ordering::Relaxed)
                    && let Ok(Frame::Push(Push::Message { seq, .. })) = Frame::from_cbor(&bytes)
                {
                    measured_pushes += 1;
                    match relay.fault {
                        Fault::DelaySecondPush if measured_pushes == 2 => sleep(Duration::from_millis(1_550)).await,
                        Fault::DropAck => drop_ack = Some(seq),
                        Fault::DelaySecondPush => {}
                    }
                }
                if front.send(Message::Binary(bytes)).await.is_err() {
                    break;
                }
            }
        }
    }
}
