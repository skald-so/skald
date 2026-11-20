//! The answer to a request whose body the server did not read to its end, as one it refuses before
//! reading the body, says `Connection: close`. The server closes such a connection after the answer
//! unless the rest of the body has come by then; without the header, a client would send its next
//! request on the connection and get no answer.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use axum::body::{Body, Bytes, HttpBody};
use axum::extract::Request;
use axum::http::{HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;
use http_body::{Frame, SizeHint};

/// Middleware: passes on the request, its body marking when it was read to its end, and closes the
/// connection after the answer if it was not.
pub async fn close_unread(request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    if body.is_end_stream() {
        return next.run(Request::from_parts(parts, body)).await;
    }
    let ended = Arc::new(AtomicBool::new(false));
    let body = Body::new(Marked {
        body,
        ended: Arc::clone(&ended),
    });
    let mut response = next.run(Request::from_parts(parts, body)).await;
    if !ended.load(Ordering::SeqCst) {
        (response.headers_mut()).insert(header::CONNECTION, HeaderValue::from_static("close"));
    }
    response
}

/// A request's body that sets `ended` once it was read to its end.
struct Marked {
    body: Body,
    ended: Arc<AtomicBool>,
}

impl HttpBody for Marked {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let polled = Pin::new(&mut self.body).poll_frame(cx);
        if matches!(polled, Poll::Ready(None)) || self.body.is_end_stream() {
            self.ended.store(true, Ordering::SeqCst);
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt as _;

    use super::*;

    /// The body that marks `ended`, of `body`.
    fn marked(body: Body, ended: &Arc<AtomicBool>) -> Body {
        Body::new(Marked {
            body,
            ended: Arc::clone(ended),
        })
    }

    /// A body read to its end is marked so; one read only in part is not.
    #[tokio::test]
    async fn a_body_is_marked_ended_once_read_to_its_end() {
        let ended = Arc::new(AtomicBool::new(false));
        let whole = axum::body::to_bytes(marked(Body::from("whole"), &ended), usize::MAX).await;
        assert_eq!(whole.expect("the body"), "whole");
        assert!(ended.load(Ordering::SeqCst));

        let ended = Arc::new(AtomicBool::new(false));
        let chunks = ["first", "second"].map(|chunk| Ok::<_, axum::Error>(Bytes::from(chunk)));
        let body = marked(
            Body::from_stream(futures_util::stream::iter(chunks)),
            &ended,
        );
        let mut stream = body.into_data_stream();
        let first = stream.next().await.expect("a chunk").expect("the chunk");
        assert_eq!(first, "first");
        assert!(!ended.load(Ordering::SeqCst));
    }
}
