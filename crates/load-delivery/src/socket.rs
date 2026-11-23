//! Device WebSockets and their timestamped observations.

use std::net::SocketAddrV4;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use skald_proto::{Cbor, Frame, PROTOCOL_VERSION, Push, subprotocol};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::Result;

const IO_TIMEOUT: Duration = Duration::from_secs(4);

pub struct Socket(WebSocketStream<MaybeTlsStream<TcpStream>>);

impl Socket {
    pub async fn connect(address: SocketAddrV4) -> Result<Self> {
        let mut request = format!("ws://{address}/ws").into_client_request()?;
        let protocol = subprotocol(PROTOCOL_VERSION);
        request
            .headers_mut()
            .insert("sec-websocket-protocol", protocol.parse()?);
        let (stream, answer) =
            timeout(IO_TIMEOUT, tokio_tungstenite::connect_async(request)).await??;
        if answer
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|value| value.to_str().ok())
            != Some(protocol.as_str())
        {
            return Err("the WebSocket did not negotiate the protocol version".into());
        }
        Ok(Self(stream))
    }

    pub async fn send(&mut self, frame: &Frame) -> Result<()> {
        timeout(
            IO_TIMEOUT,
            self.0.send(Message::Binary(frame.to_cbor().into())),
        )
        .await??;
        Ok(())
    }

    pub async fn next(&mut self) -> Result<(Instant, Frame)> {
        loop {
            match self.0.next().await.ok_or("the WebSocket ended")?? {
                Message::Binary(bytes) => return Ok((Instant::now(), Frame::from_cbor(&bytes)?)),
                Message::Close(frame) => {
                    return Err(format!(
                        "the WebSocket closed ({})",
                        frame.map_or(1005, |f| f.code.into())
                    )
                    .into());
                }
                Message::Text(_) => return Err("the WebSocket sent a text frame".into()),
                _ => {}
            }
        }
    }

    pub async fn ready(&mut self, auth: Frame) -> Result<()> {
        self.send(&auth).await?;
        self.send(&Frame::Sync { cursor: 0 }).await?;
        timeout(IO_TIMEOUT, async {
            loop {
                match self.next().await?.1 {
                    Frame::Batch(batch) if batch.is_empty() => return Ok(()),
                    Frame::Push(Push::Message { seq, .. }) => {
                        self.send(&Frame::Ack { seq }).await?
                    }
                    _ => {}
                }
            }
        })
        .await?
    }

    pub async fn within(&mut self, accept: impl Fn(&Frame) -> bool) -> Result<Frame> {
        timeout(IO_TIMEOUT, async {
            loop {
                let (_, frame) = self.next().await?;
                if accept(&frame) {
                    return Ok(frame);
                }
            }
        })
        .await?
    }
}
