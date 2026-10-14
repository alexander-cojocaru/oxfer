//! A running connection: a reader task turning server messages into
//! [`Event`]s and a writer task sending input.

use std::sync::Arc;
#[cfg(test)]
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Notify};
use tokio::time::timeout;

use crate::encodings::{
    self, Zrle, ENC_COPY_RECT, ENC_DESKTOP_SIZE, ENC_HEXTILE, ENC_RAW, ENC_ZRLE,
};
use crate::error::{Error, Result};
use crate::handshake::{check_size, handshake, MAX_PIXELS};
use crate::message::{Event, Op};
use crate::ConnectOptions;

/// Cap for clipboard text in either direction.
const MAX_CLIPBOARD: usize = 1024 * 1024;

#[derive(Debug)]
enum Input {
    Key {
        keysym: u32,
        down: bool,
    },
    Pointer {
        x: u16,
        y: u16,
        buttons: u8,
    },
    Clipboard(String),
    Update {
        incremental: bool,
        width: u16,
        height: u16,
    },
    Close,
}

impl Input {
    fn encode(&self) -> Vec<u8> {
        match self {
            Input::Key { keysym, down } => {
                let mut m = vec![4, *down as u8, 0, 0];
                m.extend(keysym.to_be_bytes());
                m
            }
            Input::Pointer { x, y, buttons } => {
                let mut m = vec![5, *buttons];
                m.extend(x.to_be_bytes());
                m.extend(y.to_be_bytes());
                m
            }
            Input::Clipboard(text) => {
                // ClientCutText is Latin-1.
                let bytes: Vec<u8> = text
                    .chars()
                    .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
                    .take(MAX_CLIPBOARD)
                    .collect();
                let mut m = vec![6, 0, 0, 0];
                m.extend((bytes.len() as u32).to_be_bytes());
                m.extend(bytes);
                m
            }
            Input::Update {
                incremental,
                width,
                height,
            } => {
                let mut m = vec![3, *incremental as u8, 0, 0, 0, 0];
                m.extend(width.to_be_bytes());
                m.extend(height.to_be_bytes());
                m
            }
            Input::Close => vec![],
        }
    }
}

/// Sends input to the server. Cheap to clone.
#[derive(Clone)]
pub struct Handle {
    tx: mpsc::UnboundedSender<Input>,
    closed: Arc<Notify>,
}

impl Handle {
    /// A key press or release, as an X11 keysym.
    pub fn key(&self, keysym: u32, down: bool) {
        let _ = self.tx.send(Input::Key { keysym, down });
    }

    /// The pointer position with the pressed buttons (bit 0 = left,
    /// 1 = middle, 2 = right, 3-6 = wheel up/down/left/right).
    pub fn pointer(&self, x: u16, y: u16, buttons: u8) {
        let _ = self.tx.send(Input::Pointer { x, y, buttons });
    }

    pub fn clipboard(&self, text: String) {
        let _ = self.tx.send(Input::Clipboard(text));
    }

    /// End the connection. No further event follows.
    pub fn close(&self) {
        let _ = self.tx.send(Input::Close);
        self.closed.notify_one();
    }
}

pub struct Connection {
    pub width: u16,
    pub height: u16,
    /// The desktop's name as reported by the server.
    pub name: String,
    pub handle: Handle,
    /// Ends with [`Event::Closed`] unless [`Handle::close`] was called.
    pub events: mpsc::Receiver<Event>,
}

pub async fn connect(opts: ConnectOptions) -> Result<Connection> {
    let stream = timeout(
        opts.timeout,
        TcpStream::connect((opts.host.as_str(), opts.port)),
    )
    .await
    .map_err(|_| Error::Timeout)??;
    stream.set_nodelay(true)?;
    establish(stream, &opts).await
}

/// SetPixelFormat (32 bpp, R/G/B at shifts 0/8/16, little-endian) and
/// SetEncodings, most preferred first.
fn setup_messages() -> Vec<u8> {
    let mut m = vec![0u8, 0, 0, 0];
    m.extend([32, 24, 0, 1]); // bits per pixel, depth, big-endian, true colour
    for max in [255u16, 255, 255] {
        m.extend(max.to_be_bytes());
    }
    m.extend([0, 8, 16, 0, 0, 0]); // shifts, padding
    let encodings = [
        ENC_ZRLE,
        ENC_HEXTILE,
        ENC_COPY_RECT,
        ENC_RAW,
        ENC_DESKTOP_SIZE,
    ];
    m.extend([2, 0]);
    m.extend((encodings.len() as u16).to_be_bytes());
    for e in encodings {
        m.extend(e.to_be_bytes());
    }
    m
}

/// Handshake on an established stream and start the connection's tasks.
pub async fn establish<S>(mut stream: S, opts: &ConnectOptions) -> Result<Connection>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let init = timeout(opts.timeout, async {
        let init = handshake(&mut stream, opts).await?;
        stream.write_all(&setup_messages()).await?;
        stream.flush().await?;
        Ok::<_, Error>(init)
    })
    .await
    .map_err(|_| Error::Timeout)??;

    let (rd, mut wr) = tokio::io::split(stream);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Input>();
    let (event_tx, events) = mpsc::channel::<Event>(4);
    let closed = Arc::new(Notify::new());

    let _ = input_tx.send(Input::Update {
        incremental: false,
        width: init.width,
        height: init.height,
    });

    tokio::spawn(async move {
        while let Some(input) = input_rx.recv().await {
            if matches!(input, Input::Close) {
                break;
            }
            if wr.write_all(&input.encode()).await.is_err() {
                break;
            }
        }
        let _ = wr.shutdown().await;
    });

    let reader_input = input_tx.clone();
    let reader_closed = closed.clone();
    tokio::spawn(async move {
        let reader = Reader {
            r: BufReader::with_capacity(64 * 1024, rd),
            events: event_tx.clone(),
            input: reader_input,
            size: (init.width, init.height),
            zrle: Zrle::new(),
        };
        let reason = tokio::select! {
            _ = reader_closed.notified() => return,
            r = reader.run() => match r {
                Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    "the server closed the connection".to_string()
                }
                Err(e) => e.to_string(),
                // `run` only returns on error or when the UI stopped listening.
                Ok(()) => return,
            },
        };
        tracing::debug!("VNC connection ended: {reason}");
        let _ = event_tx.send(Event::Closed(Some(reason))).await;
    });

    Ok(Connection {
        width: init.width,
        height: init.height,
        name: init.name,
        handle: Handle {
            tx: input_tx,
            closed,
        },
        events,
    })
}

struct Reader<R> {
    r: BufReader<R>,
    events: mpsc::Sender<Event>,
    input: mpsc::UnboundedSender<Input>,
    size: (u16, u16),
    zrle: Zrle,
}

impl<R: AsyncRead + Unpin> Reader<R> {
    /// Handle server messages until the connection fails; `Ok` means the
    /// consumer of the events went away.
    async fn run(mut self) -> Result<()> {
        loop {
            match self.r.read_u8().await? {
                0 => {
                    let (ops, resized) = self.framebuffer_update().await?;
                    if !ops.is_empty() && self.events.send(Event::Update(ops)).await.is_err() {
                        return Ok(());
                    }
                    // Ask for the next change only after this one was taken:
                    // a slow UI slows the server down instead of piling up.
                    let (width, height) = self.size;
                    let request = Input::Update {
                        incremental: !resized,
                        width,
                        height,
                    };
                    if self.input.send(request).is_err() {
                        return Ok(());
                    }
                }
                1 => {
                    // SetColourMapEntries: unused with true colour.
                    let mut head = [0u8; 5];
                    self.r.read_exact(&mut head).await?;
                    let n = u16::from_be_bytes([head[3], head[4]]) as u64;
                    self.skip(n * 6).await?;
                }
                2 => {
                    if self.events.send(Event::Bell).await.is_err() {
                        return Ok(());
                    }
                }
                3 => {
                    let mut pad = [0u8; 3];
                    self.r.read_exact(&mut pad).await?;
                    let len = self.r.read_i32().await?;
                    // A negative length announces the extended clipboard
                    // format, which we did not ask for: skip it.
                    let n = len.unsigned_abs() as usize;
                    if n > 16 * MAX_CLIPBOARD {
                        return Err(Error::protocol("clipboard from the server is too large"));
                    }
                    if len < 0 {
                        self.skip(n as u64).await?;
                        continue;
                    }
                    let mut buf = vec![0u8; n];
                    self.r.read_exact(&mut buf).await?;
                    if n <= MAX_CLIPBOARD {
                        let text: String = buf.iter().map(|b| *b as char).collect();
                        if self.events.send(Event::Clipboard(text)).await.is_err() {
                            return Ok(());
                        }
                    }
                }
                t => return Err(Error::protocol(format!("unknown server message {t}"))),
            }
        }
    }

    async fn skip(&mut self, n: u64) -> Result<()> {
        let copied = tokio::io::copy(&mut (&mut self.r).take(n), &mut tokio::io::sink()).await?;
        if copied < n {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
        }
        Ok(())
    }

    /// Returns the changes and whether the screen was resized (then the
    /// whole screen must be requested again).
    async fn framebuffer_update(&mut self) -> Result<(Vec<Op>, bool)> {
        let mut head = [0u8; 3];
        self.r.read_exact(&mut head).await?;
        let count = u16::from_be_bytes([head[1], head[2]]);
        let mut ops = Vec::new();
        let mut resized = false;
        for _ in 0..count {
            let x = self.r.read_u16().await?;
            let y = self.r.read_u16().await?;
            let w = self.r.read_u16().await?;
            let h = self.r.read_u16().await?;
            let encoding = self.r.read_i32().await?;
            if encoding == ENC_DESKTOP_SIZE {
                check_size(w, h)?;
                self.size = (w, h);
                resized = true;
                ops.push(Op::Resize {
                    width: w,
                    height: h,
                });
                continue;
            }
            let (fb_w, fb_h) = self.size;
            if x as usize + w as usize > fb_w as usize || y as usize + h as usize > fb_h as usize {
                return Err(Error::protocol("rectangle outside the screen"));
            }
            let (wu, hu) = (w as usize, h as usize);
            debug_assert!(wu * hu <= MAX_PIXELS);
            let rgba = match encoding {
                ENC_RAW => encodings::read_raw(&mut self.r, wu, hu).await?,
                ENC_HEXTILE => encodings::read_hextile(&mut self.r, wu, hu).await?,
                ENC_ZRLE => {
                    let len = self.r.read_u32().await? as usize;
                    if len > wu * hu * 8 + 64 * 1024 {
                        return Err(Error::protocol("ZRLE rectangle is too large"));
                    }
                    let mut compressed = vec![0u8; len];
                    self.r.read_exact(&mut compressed).await?;
                    self.zrle.decode(&compressed, wu, hu)?
                }
                ENC_COPY_RECT => {
                    let src_x = self.r.read_u16().await?;
                    let src_y = self.r.read_u16().await?;
                    if src_x as usize + wu > fb_w as usize || src_y as usize + hu > fb_h as usize {
                        return Err(Error::protocol("copy source outside the screen"));
                    }
                    ops.push(Op::Copy {
                        x,
                        y,
                        width: w,
                        height: h,
                        src_x,
                        src_y,
                    });
                    continue;
                }
                other => {
                    return Err(Error::Unsupported(format!("rectangle encoding {other}")));
                }
            };
            if w > 0 && h > 0 {
                ops.push(Op::Rect {
                    x,
                    y,
                    width: w,
                    height: h,
                    rgba,
                });
            }
        }
        Ok((ops, resized))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::DuplexStream;

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    async fn server_hello(s: &mut DuplexStream, w: u16, h: u16) {
        s.write_all(b"RFB 003.008\n").await.unwrap();
        let mut reply = [0u8; 12];
        s.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"RFB 003.008\n");
        s.write_all(&[1, 1]).await.unwrap(); // one type: None
        assert_eq!(s.read_u8().await.unwrap(), 1);
        s.write_u32(0).await.unwrap(); // SecurityResult OK
        assert_eq!(s.read_u8().await.unwrap(), 1); // ClientInit shared
        s.write_u16(w).await.unwrap();
        s.write_u16(h).await.unwrap();
        s.write_all(&[0u8; 16]).await.unwrap();
        s.write_u32(4).await.unwrap();
        s.write_all(b"test").await.unwrap();
        // SetPixelFormat (20 bytes) + SetEncodings (4 + 5*4) + first request (10)
        let mut setup = vec![0u8; 20 + 24 + 10];
        s.read_exact(&mut setup).await.unwrap();
        assert_eq!(setup[0], 0);
        assert_eq!(&setup[4..8], &[32, 24, 0, 1]);
        assert_eq!(setup[20], 2);
        assert_eq!(
            &setup[44..],
            &[
                3,
                0,
                0,
                0,
                0,
                0,
                (w >> 8) as u8,
                w as u8,
                (h >> 8) as u8,
                h as u8
            ]
        );
    }

    fn rect_header(x: u16, y: u16, w: u16, h: u16, enc: i32) -> Vec<u8> {
        let mut m = vec![];
        for v in [x, y, w, h] {
            m.extend(v.to_be_bytes());
        }
        m.extend(enc.to_be_bytes());
        m
    }

    async fn next(c: &mut Connection) -> Event {
        timeout(TEST_TIMEOUT, c.events.recv())
            .await
            .expect("event in time")
            .expect("event")
    }

    #[tokio::test]
    async fn updates_input_and_close() {
        let (client, mut server) = tokio::io::duplex(1 << 16);
        let server_task = tokio::spawn(async move {
            server_hello(&mut server, 4, 2).await;

            // Update: a raw 2x1 rectangle and a copy rectangle.
            let mut m = vec![0u8, 0, 0, 2];
            m.extend(rect_header(1, 0, 2, 1, ENC_RAW));
            m.extend([10, 20, 30, 0, 40, 50, 60, 0]);
            m.extend(rect_header(0, 1, 2, 1, ENC_COPY_RECT));
            m.extend([0, 0, 0, 0]);
            server.write_all(&m).await.unwrap();

            // The client asks for the next incremental update.
            let mut req = [0u8; 10];
            server.read_exact(&mut req).await.unwrap();
            assert_eq!(req, [3, 1, 0, 0, 0, 0, 0, 4, 0, 2]);

            // Bell, clipboard (Latin-1 "é"), then a resize.
            server.write_all(&[2]).await.unwrap();
            server
                .write_all(&[3, 0, 0, 0, 0, 0, 0, 1, 0xE9])
                .await
                .unwrap();
            let mut m = vec![0u8, 0, 0, 1];
            m.extend(rect_header(0, 0, 8, 6, ENC_DESKTOP_SIZE));
            server.write_all(&m).await.unwrap();

            // After a resize the whole screen is requested again.
            server.read_exact(&mut req).await.unwrap();
            assert_eq!(req, [3, 0, 0, 0, 0, 0, 0, 8, 0, 6]);

            // Input from the client.
            let mut key = [0u8; 8];
            server.read_exact(&mut key).await.unwrap();
            assert_eq!(key, [4, 1, 0, 0, 0, 0, 0, 0x61]);
            let mut ptr = [0u8; 6];
            server.read_exact(&mut ptr).await.unwrap();
            assert_eq!(ptr, [5, 1, 0, 7, 0, 9]);
            let mut cut = [0u8; 10];
            server.read_exact(&mut cut).await.unwrap();
            assert_eq!(cut, [6, 0, 0, 0, 0, 0, 0, 2, b'h', b'?']);

            // Close: the server sees the connection end.
            let mut rest = vec![];
            server.read_to_end(&mut rest).await.unwrap();
            assert!(rest.is_empty());
        });

        let mut opts = ConnectOptions::new("unused", 0);
        opts.timeout = TEST_TIMEOUT;
        let mut c = establish(client, &opts).await.unwrap();
        assert_eq!((c.width, c.height, c.name.as_str()), (4, 2, "test"));

        let Event::Update(ops) = next(&mut c).await else {
            panic!("expected an update")
        };
        assert_eq!(
            ops,
            vec![
                Op::Rect {
                    x: 1,
                    y: 0,
                    width: 2,
                    height: 1,
                    rgba: vec![10, 20, 30, 255, 40, 50, 60, 255]
                },
                Op::Copy {
                    x: 0,
                    y: 1,
                    width: 2,
                    height: 1,
                    src_x: 0,
                    src_y: 0
                },
            ]
        );
        assert_eq!(next(&mut c).await, Event::Bell);
        assert_eq!(next(&mut c).await, Event::Clipboard("é".into()));
        assert_eq!(
            next(&mut c).await,
            Event::Update(vec![Op::Resize {
                width: 8,
                height: 6
            }])
        );

        c.handle.key(0x61, true);
        c.handle.pointer(7, 9, 1);
        c.handle.clipboard("h\u{4e2d}".into());
        // Give the writer a moment to flush before closing.
        tokio::time::sleep(Duration::from_millis(100)).await;
        c.handle.close();
        timeout(TEST_TIMEOUT, server_task)
            .await
            .expect("server finished")
            .unwrap();
    }

    #[tokio::test]
    async fn protocol_errors_end_the_connection() {
        let (client, mut server) = tokio::io::duplex(1 << 16);
        tokio::spawn(async move {
            server_hello(&mut server, 4, 2).await;
            // A rectangle outside the 4x2 screen.
            let mut m = vec![0u8, 0, 0, 1];
            m.extend(rect_header(3, 0, 2, 1, ENC_RAW));
            server.write_all(&m).await.unwrap();
            let mut sink = vec![];
            let _ = server.read_to_end(&mut sink).await;
        });
        let mut c = establish(client, &ConnectOptions::new("unused", 0))
            .await
            .unwrap();
        match next(&mut c).await {
            Event::Closed(Some(reason)) => {
                assert!(reason.contains("outside the screen"), "{reason}")
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn server_hanging_up_is_reported() {
        let (client, mut server) = tokio::io::duplex(1 << 16);
        tokio::spawn(async move {
            server_hello(&mut server, 4, 2).await;
            drop(server);
        });
        let mut c = establish(client, &ConnectOptions::new("unused", 0))
            .await
            .unwrap();
        match next(&mut c).await {
            Event::Closed(Some(reason)) => assert!(reason.contains("closed"), "{reason}"),
            other => panic!("unexpected {other:?}"),
        }
    }
}
