//! A device's mailbox over server-sent events: `docs/protocol/delivery.md`, "Frames" and
//! "Mailbox". A stream proves its device as each HTTPS request of the mailbox does
//! ([`requests`](crate::requests)), acks the cursor in `Last-Event-ID`, sends the entries after it,
//! then an empty `batch`, then each new entry and `space-notice`; one frame per event. It ends when
//! its session ends or its device is revoked, and sends a keep-alive comment while idle.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::time::Duration;

use axum::Extension;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::stream;
use skald_proto::{Cbor as _, Frame, Push};
use tokio::sync::oneshot::error::TryRecvError;

use crate::connections::Proved;
use crate::delivery::Reader;
use crate::notices::Listening;
use crate::protocol::Stated;
use crate::requests::{ack_listening, invalid, listening, prove, receiving};
use crate::{AppState, database_failed, error_reports, service_tokens};

/// The keep-alive interval of a stream without the server setting.
pub const SSE_KEEP_ALIVE: Duration = Duration::from_secs(15);
/// The request header that states the cursor.
const LAST_EVENT_ID: &str = "last-event-id";

/// `GET /mailbox/events`: acks the device's mailbox up to the cursor in `Last-Event-ID`, 0 without
/// it, and streams the entries after it in `seq` order, then an empty `batch`, then each new entry
/// and `space-notice`, until the session ends or the device is revoked.
pub(crate) async fn events(
    State(state): State<AppState>,
    Extension(Stated(version)): Extension<Stated>,
    headers: HeaderMap,
) -> Response {
    let proved = match prove(&state, &headers).await {
        Ok(proved) => proved,
        Err(refused) => return refused,
    };
    if let Err(refused) = receiving(&proved) {
        return refused;
    }
    let Some(cursor) = cursor(&headers) else {
        return invalid("Last-Event-ID is not a seq.");
    };
    // Until the instance listens to the others, a stream would miss what they tell, and get the
    // notices of revisions stored before it opened, which the instance sends once it listens.
    if let Err(failed) = listening(&state).await {
        return *failed;
    }
    // Listened to before reading, so that nothing added meanwhile is missed.
    let mut reader = Reader::new(
        &state.wakers,
        proved.device,
        version,
        state.mailbox_retention,
    );
    let notices = state.notices.listen(proved.account);
    if let Err(failed) = ack_listening(&state, proved.device, cursor, false).await {
        return *failed;
    }
    let held = match reader.start(&state.db).await {
        Ok(held) => held,
        Err(err) => return database_failed("read a mailbox", err),
    };
    let keep_alive = KeepAlive::new().interval(state.sse_keep_alive);
    let pushing = Pushing {
        state,
        proved,
        reader,
        notices,
        reading: held,
        catching_up: true,
        queued: VecDeque::new(),
    };
    Sse::new(stream::unfold(pushing, Pushing::next))
        .keep_alive(keep_alive)
        .into_response()
}

/// The cursor in the request's only `Last-Event-ID`, 0 without one; `None` if it is not digits.
fn cursor(headers: &HeaderMap) -> Option<u64> {
    let mut values = headers.get_all(LAST_EVENT_ID).iter();
    match (values.next(), values.next()) {
        (None, _) => Some(0),
        (Some(value), None) if value.as_bytes().iter().all(u8::is_ascii_digit) => {
            value.to_str().ok()?.parse().ok()
        }
        (Some(_), None) => None,
        (Some(_), Some(_)) => None,
    }
}

/// A stream's state: what it has to send, and what it waits for.
struct Pushing {
    state: AppState,
    proved: Proved,
    reader: Reader,
    notices: Listening,
    /// The `seq` up to which entries are left to read, one page at a time.
    reading: Option<i64>,
    /// Whether the entries left to read end catch-up.
    catching_up: bool,
    queued: VecDeque<Frame>,
}

impl Pushing {
    /// The next event and the state after it; `None` once the stream ends.
    async fn next(mut self) -> Option<(Result<Event, Infallible>, Self)> {
        loop {
            // Fired, or let go of: the session ended or the device is revoked.
            if !matches!(self.proved.ended.try_recv(), Err(TryRecvError::Empty)) {
                return None;
            }
            if let Some(frame) = self.queued.pop_front() {
                return Some((Ok(event(&frame)), self));
            }
            if let Some(upto) = self.reading {
                if !self.read(upto).await? {
                    self.reading = None;
                    if self.catching_up {
                        self.catching_up = false;
                        self.queued.push_back(Frame::Batch(Vec::new()));
                    }
                }
                continue;
            }
            tokio::select! {
                // Once the session ends or the device is revoked, nothing more is sent.
                biased;
                _ = &mut self.proved.ended => return None,
                () = self.reader.woken() => self.reading = Some(i64::MAX),
                spaces = self.notices.next() => {
                    let spaces = service_tokens::seen(self.proved.token.as_ref(), spaces);
                    let notices = spaces.into_iter().map(|space| Push::SpaceNotice { space });
                    self.queued.extend(notices.map(Frame::Push));
                }
            }
        }
    }

    /// Queues the next page of entries up to `seq` `upto`: whether there was one; `None`, which
    /// ends the stream, if the read failed.
    async fn read(&mut self, upto: i64) -> Option<bool> {
        match self.reader.next(&self.state.db, upto).await {
            Ok(Some(page)) => {
                self.queued.extend(page.into_iter().map(Frame::Push));
                Some(true)
            }
            Ok(None) => Some(false),
            Err(err) => {
                let cause = error_reports::logged("read a mailbox", &err);
                error_reports::report(
                    &format!("a stream ended with server-error: {cause}"),
                    Some("GET /mailbox/events"),
                );
                None
            }
        }
    }
}

/// The event of `frame`: its CBOR in base64url without padding, with the `seq` of a `message` or
/// `status` as its `id`.
fn event(frame: &Frame) -> Event {
    let event = Event::default().data(URL_SAFE_NO_PAD.encode(frame.to_cbor()));
    match frame {
        Frame::Push(Push::Message { seq, .. } | Push::Status { seq, .. }) => {
            event.id(seq.to_string())
        }
        _ => event,
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(LAST_EVENT_ID, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    #[test]
    fn the_cursor_is_the_only_last_event_id_or_0_without_one() {
        assert_eq!(cursor(&headers(&[])), Some(0));
        assert_eq!(cursor(&headers(&["7"])), Some(7));
        for values in [&["7", "8"][..], &["-1"], &["+7"], &["seven"], &[""]] {
            assert_eq!(cursor(&headers(values)), None, "{values:?}");
        }
    }
}
