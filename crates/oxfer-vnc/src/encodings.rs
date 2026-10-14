//! Rectangle decoders. The client asks the server for 32-bit true-colour
//! pixels laid out as R, G, B, unused, so a decoded rectangle is RGBA once
//! the unused byte is set to 255.

use flate2::{Decompress, FlushDecompress};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::error::{Error, Result};

pub const ENC_RAW: i32 = 0;
pub const ENC_COPY_RECT: i32 = 1;
pub const ENC_HEXTILE: i32 = 5;
pub const ENC_ZRLE: i32 = 16;
pub const ENC_DESKTOP_SIZE: i32 = -223;

type Px = [u8; 4];

fn fill(out: &mut [u8], stride: usize, x: usize, y: usize, w: usize, h: usize, px: Px) {
    for row in y..y + h {
        let start = (row * stride + x) * 4;
        for p in out[start..start + w * 4].as_chunks_mut::<4>().0 {
            *p = px;
        }
    }
}

async fn read_px<R: AsyncRead + Unpin>(r: &mut R) -> Result<Px> {
    let mut p = [0u8; 4];
    r.read_exact(&mut p).await?;
    p[3] = 255;
    Ok(p)
}

pub async fn read_raw<R: AsyncRead + Unpin>(r: &mut R, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; w * h * 4];
    r.read_exact(&mut buf).await?;
    for p in buf.as_chunks_mut::<4>().0 {
        p[3] = 255;
    }
    Ok(buf)
}

/// Hextile: 16x16 tiles, each raw or a background with coloured or
/// foreground-coloured sub-rectangles. Background and foreground carry over
/// from tile to tile.
pub async fn read_hextile<R: AsyncRead + Unpin>(r: &mut R, w: usize, h: usize) -> Result<Vec<u8>> {
    const RAW: u8 = 1;
    const BG: u8 = 2;
    const FG: u8 = 4;
    const SUBRECTS: u8 = 8;
    const COLOURED: u8 = 16;

    let mut out = vec![0u8; w * h * 4];
    let (mut bg, mut fg) = ([0, 0, 0, 255], [0, 0, 0, 255]);
    for ty in (0..h).step_by(16) {
        for tx in (0..w).step_by(16) {
            let (tw, th) = (16.min(w - tx), 16.min(h - ty));
            let sub = r.read_u8().await?;
            if sub & RAW != 0 {
                for row in 0..th {
                    let start = ((ty + row) * w + tx) * 4;
                    let line = &mut out[start..start + tw * 4];
                    r.read_exact(line).await?;
                    for p in line.as_chunks_mut::<4>().0 {
                        p[3] = 255;
                    }
                }
                continue;
            }
            if sub & BG != 0 {
                bg = read_px(r).await?;
            }
            if sub & FG != 0 {
                fg = read_px(r).await?;
            }
            fill(&mut out, w, tx, ty, tw, th, bg);
            if sub & SUBRECTS != 0 {
                let n = r.read_u8().await?;
                for _ in 0..n {
                    let colour = if sub & COLOURED != 0 {
                        read_px(r).await?
                    } else {
                        fg
                    };
                    let xy = r.read_u8().await?;
                    let wh = r.read_u8().await?;
                    let (sx, sy) = ((xy >> 4) as usize, (xy & 15) as usize);
                    let (sw, sh) = ((wh >> 4) as usize + 1, (wh & 15) as usize + 1);
                    if sx + sw > tw || sy + sh > th {
                        return Err(Error::protocol("hextile sub-rectangle outside its tile"));
                    }
                    fill(&mut out, w, tx + sx, ty + sy, sw, sh, colour);
                }
            }
        }
    }
    Ok(out)
}

/// A bounds-checked cursor over decompressed ZRLE data.
struct Cur<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cur<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|e| *e <= self.data.len())
            .ok_or_else(|| Error::protocol("ZRLE data ends early"))?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    /// A compressed pixel: 3 bytes (R, G, B) for our pixel format.
    fn cpixel(&mut self) -> Result<Px> {
        let b = self.take(3)?;
        Ok([b[0], b[1], b[2], 255])
    }

    fn palette(&mut self, n: usize) -> Result<Vec<Px>> {
        (0..n).map(|_| self.cpixel()).collect()
    }

    fn run_length(&mut self) -> Result<usize> {
        let mut len = 1usize;
        loop {
            let b = self.u8()?;
            len += b as usize;
            if b != 255 {
                return Ok(len);
            }
        }
    }
}

/// ZRLE decoder state: one zlib stream is shared by all rectangles of a
/// connection.
pub struct Zrle {
    inflate: Decompress,
}

impl Default for Zrle {
    fn default() -> Self {
        Self::new()
    }
}

impl Zrle {
    pub fn new() -> Self {
        Self {
            inflate: Decompress::new(true),
        }
    }

    /// Decode one rectangle from its zlib-compressed payload.
    pub fn decode(&mut self, compressed: &[u8], w: usize, h: usize) -> Result<Vec<u8>> {
        // Even the worst case (4 bytes per pixel plus tile headers) fits.
        let cap = w * h * 6 + 4096;
        let data = self.inflate(compressed, cap)?;
        decode_tiles(&data, w, h)
    }

    fn inflate(&mut self, compressed: &[u8], cap: usize) -> Result<Vec<u8>> {
        let start = self.inflate.total_in();
        let mut out = Vec::with_capacity(compressed.len().saturating_mul(4).min(cap));
        loop {
            let consumed = (self.inflate.total_in() - start) as usize;
            if out.capacity() - out.len() < 4096 {
                out.reserve(16 * 1024);
            }
            let (before_in, before_len) = (self.inflate.total_in(), out.len());
            self.inflate
                .decompress_vec(&compressed[consumed..], &mut out, FlushDecompress::Sync)
                .map_err(|e| Error::protocol(format!("bad ZRLE data: {e}")))?;
            if out.len() > cap {
                return Err(Error::protocol("ZRLE rectangle is too large"));
            }
            let all_read = (self.inflate.total_in() - start) as usize >= compressed.len();
            if all_read && out.len() < out.capacity() {
                return Ok(out);
            }
            if self.inflate.total_in() == before_in && out.len() == before_len {
                return Err(Error::protocol("ZRLE stream stalled"));
            }
        }
    }
}

fn put(out: &mut [u8], stride: usize, tx: usize, ty: usize, tw: usize, i: usize, px: Px) {
    let (x, y) = (i % tw, i / tw);
    let o = ((ty + y) * stride + tx + x) * 4;
    out[o..o + 4].copy_from_slice(&px);
}

fn decode_tiles(data: &[u8], w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = vec![0u8; w * h * 4];
    let mut c = Cur { data, pos: 0 };
    for ty in (0..h).step_by(64) {
        for tx in (0..w).step_by(64) {
            let (tw, th) = (64.min(w - tx), 64.min(h - ty));
            let total = tw * th;
            let sub = c.u8()?;
            let (rle, psize) = (sub & 0x80 != 0, (sub & 0x7f) as usize);
            match (rle, psize) {
                (false, 0) => {
                    for i in 0..total {
                        let px = c.cpixel()?;
                        put(&mut out, w, tx, ty, tw, i, px);
                    }
                }
                (false, 1) => {
                    let px = c.cpixel()?;
                    fill(&mut out, w, tx, ty, tw, th, px);
                }
                (false, 2..=16) => {
                    let palette = c.palette(psize)?;
                    let bits = match psize {
                        2 => 1,
                        3..=4 => 2,
                        _ => 4,
                    };
                    for y in 0..th {
                        let row = c.take((tw * bits).div_ceil(8))?;
                        for x in 0..tw {
                            let bit = x * bits;
                            let shift = 8 - bits - bit % 8;
                            let idx = ((row[bit / 8] >> shift) as usize) & ((1 << bits) - 1);
                            let px = *palette
                                .get(idx)
                                .ok_or_else(|| Error::protocol("ZRLE palette index"))?;
                            put(&mut out, w, tx, ty, tw, y * tw + x, px);
                        }
                    }
                }
                (true, 0) => {
                    let mut i = 0;
                    while i < total {
                        let px = c.cpixel()?;
                        let len = c.run_length()?;
                        if i + len > total {
                            return Err(Error::protocol("ZRLE run past the tile"));
                        }
                        for k in i..i + len {
                            put(&mut out, w, tx, ty, tw, k, px);
                        }
                        i += len;
                    }
                }
                (true, 2..=127) => {
                    let palette = c.palette(psize)?;
                    let mut i = 0;
                    while i < total {
                        let b = c.u8()?;
                        let px = *palette
                            .get((b & 0x7f) as usize)
                            .ok_or_else(|| Error::protocol("ZRLE palette index"))?;
                        let len = if b & 0x80 != 0 { c.run_length()? } else { 1 };
                        if i + len > total {
                            return Err(Error::protocol("ZRLE run past the tile"));
                        }
                        for k in i..i + len {
                            put(&mut out, w, tx, ty, tw, k, px);
                        }
                        i += len;
                    }
                }
                _ => return Err(Error::protocol(format!("bad ZRLE tile type {sub}"))),
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression, FlushCompress};

    fn px(out: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let o = (y * w + x) * 4;
        out[o..o + 4].try_into().unwrap()
    }

    #[tokio::test]
    async fn raw_sets_alpha() {
        let data = [1u8, 2, 3, 0, 4, 5, 6, 0];
        let out = read_raw(&mut &data[..], 2, 1).await.unwrap();
        assert_eq!(out, [1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[tokio::test]
    async fn raw_reports_short_data() {
        assert!(read_raw(&mut &[0u8; 7][..], 2, 1).await.is_err());
    }

    #[tokio::test]
    async fn hextile_background_foreground_and_subrects() {
        // One 20x3 rectangle = two tiles (16 and 4 wide).
        let mut d = vec![];
        // tile 1: background red, foreground blue, 2 subrects with fg colour
        d.push(2 | 4 | 8);
        d.extend([255, 0, 0, 0]);
        d.extend([0, 0, 255, 0]);
        d.push(2);
        d.extend([0x00, 0x10]); // (0,0) 2x1
        d.extend([0xF2, 0x00]); // (15,2) 1x1
                                // tile 2: reuses the background, coloured subrect green
        d.push(8 | 16);
        d.push(1);
        d.extend([0, 255, 0, 0]);
        d.extend([0x11, 0x11]); // (1,1) 2x2
        let out = read_hextile(&mut &d[..], 20, 3).await.unwrap();
        let (red, blue, green) = ([255, 0, 0, 255], [0, 0, 255, 255], [0, 255, 0, 255]);
        assert_eq!(px(&out, 20, 0, 0), blue);
        assert_eq!(px(&out, 20, 1, 0), blue);
        assert_eq!(px(&out, 20, 2, 0), red);
        assert_eq!(px(&out, 20, 15, 2), blue);
        assert_eq!(px(&out, 20, 16, 0), red, "background carries over");
        assert_eq!(px(&out, 20, 17, 1), green);
        assert_eq!(px(&out, 20, 18, 2), green);
        assert_eq!(px(&out, 20, 19, 2), red);
    }

    #[tokio::test]
    async fn hextile_raw_tile_and_bad_subrect() {
        let mut d = vec![1u8];
        d.extend([9, 8, 7, 0, 6, 5, 4, 0]);
        let out = read_hextile(&mut &d[..], 2, 1).await.unwrap();
        assert_eq!(out, [9, 8, 7, 255, 6, 5, 4, 255]);

        // a 2x1 tile cannot hold a 2x2 sub-rectangle
        let bad = [8u8, 1, 0x00, 0x11];
        assert!(read_hextile(&mut &bad[..], 2, 1).await.is_err());
    }

    /// Compress `data` as the server would: one sync-flushed block of a
    /// stream that persists across calls.
    fn deflate(z: &mut Compress, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() + 64);
        z.compress_vec(data, &mut out, FlushCompress::Sync).unwrap();
        out
    }

    #[test]
    fn zrle_tile_types() {
        let mut z = Compress::new(Compression::default(), true);
        let mut dec = Zrle::new();

        // 5x2: raw tile
        let mut d = vec![0u8];
        for i in 0..10u8 {
            d.extend([i, i + 1, i + 2]);
        }
        let out = dec.decode(&deflate(&mut z, &d), 5, 2).unwrap();
        assert_eq!(px(&out, 5, 3, 1), [8, 9, 10, 255]);

        // solid tile (second rectangle on the same stream)
        let out = dec
            .decode(&deflate(&mut z, &[1, 10, 20, 30]), 3, 3)
            .unwrap();
        assert!(out.chunks(4).all(|p| p == [10, 20, 30, 255]));

        // packed palette, 2 colours, 1 bit per pixel, 3x2
        let d = [2u8, 1, 2, 3, 4, 5, 6, 0b1010_0000, 0b0110_0000];
        let out = dec.decode(&deflate(&mut z, &d), 3, 2).unwrap();
        let (a, b) = ([1, 2, 3, 255], [4, 5, 6, 255]);
        assert_eq!(
            out.chunks(4).collect::<Vec<_>>(),
            vec![&b, &a, &b, &a, &b, &b]
        );

        // plain RLE: 4 pixels red, then 2 blue (run of 2 = length byte 1)
        let d = [128u8, 255, 0, 0, 3, 0, 0, 255, 1];
        let out = dec.decode(&deflate(&mut z, &d), 3, 2).unwrap();
        assert_eq!(px(&out, 3, 0, 0), [255, 0, 0, 255]);
        assert_eq!(px(&out, 3, 0, 1), [255, 0, 0, 255]);
        assert_eq!(px(&out, 3, 1, 1), [0, 0, 255, 255]);

        // palette RLE: palette {green, black}; idx0 run of 3, idx1 single, idx0 single, idx1 run of 1
        let d = [130u8, 0, 255, 0, 0, 0, 0, 0x80, 2, 1, 0, 0x81, 0];
        let out = dec.decode(&deflate(&mut z, &d), 3, 2).unwrap();
        assert_eq!(px(&out, 3, 2, 0), [0, 255, 0, 255]);
        assert_eq!(px(&out, 3, 0, 1), [0, 0, 0, 255]);
        assert_eq!(px(&out, 3, 1, 1), [0, 255, 0, 255]);
        assert_eq!(px(&out, 3, 2, 1), [0, 0, 0, 255]);
    }

    #[test]
    fn zrle_spans_several_tiles() {
        let mut z = Compress::new(Compression::default(), true);
        // 70x66: tiles 64x64, 6x64, 64x2, 6x2, all solid, distinct colours.
        let mut d = vec![];
        for c in 1..=4u8 {
            d.extend([1, c, c, c]);
        }
        let out = Zrle::new().decode(&deflate(&mut z, &d), 70, 66).unwrap();
        assert_eq!(px(&out, 70, 0, 0), [1, 1, 1, 255]);
        assert_eq!(px(&out, 70, 69, 0), [2, 2, 2, 255]);
        assert_eq!(px(&out, 70, 0, 65), [3, 3, 3, 255]);
        assert_eq!(px(&out, 70, 69, 65), [4, 4, 4, 255]);
    }

    #[test]
    fn zrle_rejects_garbage() {
        let mut dec = Zrle::new();
        assert!(dec.decode(&[1, 2, 3, 4, 5, 6], 4, 4).is_err());

        // truncated tile data
        let mut z = Compress::new(Compression::default(), true);
        let mut dec = Zrle::new();
        assert!(dec.decode(&deflate(&mut z, &[0, 1, 2]), 2, 2).is_err());

        // run past the end of the tile
        let mut z = Compress::new(Compression::default(), true);
        let mut dec = Zrle::new();
        assert!(dec
            .decode(&deflate(&mut z, &[128, 1, 2, 3, 200]), 2, 2)
            .is_err());
    }

    #[test]
    fn zrle_bomb_is_capped() {
        let mut z = Compress::new(Compression::best(), true);
        // 16 MiB of zeros compresses to a few KiB but a 2x2 rectangle may
        // only expand to a few KiB.
        let zeros = vec![0u8; 16 * 1024 * 1024];
        let mut out = Vec::new();
        while out.capacity() == out.len() || out.is_empty() {
            out.reserve(64 * 1024);
            let consumed = z.total_in() as usize;
            if consumed >= zeros.len() {
                break;
            }
            z.compress_vec(&zeros[consumed..], &mut out, FlushCompress::Sync)
                .unwrap();
        }
        assert!(Zrle::new().decode(&out, 2, 2).is_err());
    }
}
