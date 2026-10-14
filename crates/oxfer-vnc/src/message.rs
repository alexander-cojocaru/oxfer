//! Screen events and their binary wire form for the UI.
//!
//! A UI message starts with a tag byte. Tags 0-2 are screen updates: a
//! message is a sequence of such records, all integers little-endian.
//!
//! | tag | record                                                   |
//! |-----|----------------------------------------------------------|
//! | 0   | resize: width u16, height u16                            |
//! | 1   | rectangle: x, y, w, h (u16 each), then w*h*4 RGBA bytes  |
//! | 2   | copy: x, y, w, h, src x, src y (u16 each)                |
//! | 3   | bell (whole message)                                     |
//! | 4   | clipboard text from the server, UTF-8 (whole message)    |
//! | 5   | closed, optional UTF-8 reason (whole message)            |

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// The screen changed size; its content is undefined until redrawn.
    Resize { width: u16, height: u16 },
    Rect {
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        rgba: Vec<u8>,
    },
    /// Copy a region of the screen onto another one.
    Copy {
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        src_x: u16,
        src_y: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// All changes of one framebuffer update.
    Update(Vec<Op>),
    Bell,
    Clipboard(String),
    /// The connection ended (with the reason, if it was an error).
    Closed(Option<String>),
}

pub const TAG_RESIZE: u8 = 0;
pub const TAG_RECT: u8 = 1;
pub const TAG_COPY: u8 = 2;
pub const TAG_BELL: u8 = 3;
pub const TAG_CLIPBOARD: u8 = 4;
pub const TAG_CLOSED: u8 = 5;

impl Event {
    /// The message sent to the UI.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Event::Update(ops) => {
                let size = ops
                    .iter()
                    .map(|op| match op {
                        Op::Rect { rgba, .. } => 9 + rgba.len(),
                        Op::Resize { .. } => 5,
                        Op::Copy { .. } => 13,
                    })
                    .sum();
                let mut out = Vec::with_capacity(size);
                for op in ops {
                    match op {
                        Op::Resize { width, height } => {
                            out.push(TAG_RESIZE);
                            out.extend(width.to_le_bytes());
                            out.extend(height.to_le_bytes());
                        }
                        Op::Rect {
                            x,
                            y,
                            width,
                            height,
                            rgba,
                        } => {
                            out.push(TAG_RECT);
                            for v in [x, y, width, height] {
                                out.extend(v.to_le_bytes());
                            }
                            out.extend(rgba);
                        }
                        Op::Copy {
                            x,
                            y,
                            width,
                            height,
                            src_x,
                            src_y,
                        } => {
                            out.push(TAG_COPY);
                            for v in [x, y, width, height, src_x, src_y] {
                                out.extend(v.to_le_bytes());
                            }
                        }
                    }
                }
                out
            }
            Event::Bell => vec![TAG_BELL],
            Event::Clipboard(text) => [&[TAG_CLIPBOARD][..], text.as_bytes()].concat(),
            Event::Closed(reason) => [
                &[TAG_CLOSED][..],
                reason.as_deref().unwrap_or_default().as_bytes(),
            ]
            .concat(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_encoding() {
        let ev = Event::Update(vec![
            Op::Resize {
                width: 0x0102,
                height: 3,
            },
            Op::Rect {
                x: 1,
                y: 2,
                width: 1,
                height: 1,
                rgba: vec![9, 8, 7, 255],
            },
            Op::Copy {
                x: 1,
                y: 2,
                width: 3,
                height: 4,
                src_x: 5,
                src_y: 6,
            },
        ]);
        assert_eq!(
            ev.encode(),
            vec![
                0, 2, 1, 3, 0, //
                1, 1, 0, 2, 0, 1, 0, 1, 0, 9, 8, 7, 255, //
                2, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0,
            ]
        );
    }

    #[test]
    fn control_messages() {
        assert_eq!(Event::Bell.encode(), vec![3]);
        assert_eq!(Event::Clipboard("hé".into()).encode(), b"\x04h\xc3\xa9");
        assert_eq!(Event::Closed(None).encode(), vec![5]);
        assert_eq!(Event::Closed(Some("x".into())).encode(), vec![5, b'x']);
    }
}
