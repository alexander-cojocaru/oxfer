//! Data-connection wrappers returned by `open_read` / `open_write`.
//!
//! FTP allows a single transfer per control connection, so each wrapper owns
//! the connection lock (`OwnedMutexGuard`) for as long as the transfer is
//! open. The transfer is finalised (data socket closed, `226` reply read) when
//!
//! * the reader hits EOF or the writer is `shutdown()` -- the outcome of the
//!   transfer is reported through that call, so a truncated download or a
//!   rejected upload surfaces as an `io::Error`;
//! * the wrapper is dropped earlier -- a task is spawned on the current
//!   runtime that closes the data socket, reads the reply and only then
//!   releases the lock, so the next operation always finds a clean control
//!   connection. Without a runtime, suppaftp's own drop handling defers the
//!   reply to the next command.
//!
//! A writer dropped without `shutdown()` still closes the data connection,
//! which makes the server commit whatever was written so far; callers that
//! care about the result must call `shutdown()`.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use std::time::Duration;

use suppaftp::tokio::{AsyncRustlsStream, TransferStream};
use suppaftp::FtpError;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::OwnedMutexGuard;

use crate::fs::State;

type Stream = TransferStream<AsyncRustlsStream>;
type Finish = Pin<Box<dyn Future<Output = io::Result<()>> + Send>>;

const DROP_FINISH_TIMEOUT: Duration = Duration::from_secs(30);

fn to_io(e: FtpError) -> io::Error {
    match e {
        FtpError::ConnectionError(e) => e,
        e => io::Error::other(e.to_string()),
    }
}

struct Transfer {
    stream: Option<Stream>,
    finishing: Option<Finish>,
    done: bool,
    guard: Option<OwnedMutexGuard<State>>,
}

impl Transfer {
    fn new(stream: Stream, guard: OwnedMutexGuard<State>) -> Self {
        Self {
            stream: Some(stream),
            finishing: None,
            done: false,
            guard: Some(guard),
        }
    }

    fn stream(&mut self) -> io::Result<&mut Stream> {
        match (&mut self.stream, self.done || self.finishing.is_some()) {
            (Some(s), false) => Ok(s),
            _ => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "transfer already finished",
            )),
        }
    }

    fn poll_finish(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.done {
            return Poll::Ready(Ok(()));
        }
        if self.finishing.is_none() {
            let Some(stream) = self.stream.take() else {
                self.done = true;
                return Poll::Ready(Ok(()));
            };
            self.finishing = Some(Box::pin(
                async move { stream.finish().await.map_err(to_io) },
            ));
        }
        let res = ready!(self
            .finishing
            .as_mut()
            .expect("set above")
            .as_mut()
            .poll(cx));
        self.finishing = None;
        self.done = true;
        self.guard = None;
        Poll::Ready(res)
    }
}

impl Drop for Transfer {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let (stream, finishing, guard) =
            (self.stream.take(), self.finishing.take(), self.guard.take());
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        rt.spawn(async move {
            let _guard = guard;
            let finish = async {
                match (finishing, stream) {
                    (Some(f), _) => f.await,
                    (None, Some(s)) => s.finish().await.map_err(to_io),
                    (None, None) => Ok(()),
                }
            };
            match tokio::time::timeout(DROP_FINISH_TIMEOUT, finish).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::debug!("abandoned FTP transfer ended with: {e}"),
                Err(_) => tracing::warn!("timed out finalising an abandoned FTP transfer"),
            }
        });
    }
}

pub(crate) struct FtpReader(Transfer);

impl FtpReader {
    pub(crate) fn new(stream: Stream, guard: OwnedMutexGuard<State>) -> Self {
        Self(Transfer::new(stream, guard))
    }
}

impl AsyncRead for FtpReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let t = &mut self.0;
        if t.done {
            return Poll::Ready(Ok(()));
        }
        if t.finishing.is_none() {
            let before = buf.filled().len();
            let s = t.stream().expect("not finished");
            ready!(Pin::new(s).poll_read(cx, buf))?;
            if buf.filled().len() > before || buf.remaining() == 0 {
                return Poll::Ready(Ok(()));
            }
        }
        // EOF on the data connection: read the transfer reply.
        t.poll_finish(cx)
    }
}

pub(crate) struct FtpWriter(Transfer);

impl FtpWriter {
    pub(crate) fn new(stream: Stream, guard: OwnedMutexGuard<State>) -> Self {
        Self(Transfer::new(stream, guard))
    }
}

impl AsyncWrite for FtpWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let s = self.0.stream()?;
        Pin::new(s).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.0.stream() {
            Ok(s) => Pin::new(s).poll_flush(cx),
            Err(_) => Poll::Ready(Ok(())),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.0.poll_finish(cx)
    }
}
