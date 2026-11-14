//! Decoder: bytes to a `DisplayList`, one record at a time. Unknown record types are skipped.
//! `decode` is long on purpose: it is one `match` with an arm per record type, in the order of
//! the table in docs/display-list.md.

use super::{BinError, MAGIC, VERSION};
use crate::{DisplayList, FontDesc, ImageInfo, Item, Line, Sp};

pub(super) struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn need(&self, n: usize) -> Result<(), BinError> {
        if self.pos + n > self.b.len() {
            Err(BinError::Truncated(self.pos))
        } else {
            Ok(())
        }
    }
    fn u8(&mut self) -> Result<u8, BinError> {
        self.need(1)?;
        let v = self.b[self.pos];
        self.pos += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16, BinError> {
        self.need(2)?;
        let v = u16::from_le_bytes([self.b[self.pos], self.b[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32, BinError> {
        self.need(4)?;
        let v = u32::from_le_bytes(self.b[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }
    fn i32(&mut self) -> Result<i32, BinError> {
        Ok(self.u32()? as i32)
    }
    fn f64(&mut self) -> Result<f64, BinError> {
        self.need(8)?;
        let v = f64::from_le_bytes(self.b[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        Ok(v)
    }
    fn str(&mut self) -> Result<String, BinError> {
        let n = self.u16()? as usize;
        self.need(n)?;
        let s = std::str::from_utf8(&self.b[self.pos..self.pos + n])
            .map_err(|_| BinError::Utf8)?
            .to_string();
        self.pos += n;
        Ok(s)
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], BinError> {
        self.need(n)?;
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
}

pub(super) fn kind_name(k: u8) -> Option<&'static str> {
    match k {
        1 => Some("opentype"),
        2 => Some("truetype"),
        3 => Some("type1"),
        4 => Some("type3"),
        _ => None,
    }
}

pub(super) fn opt_str(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Decode a binary display list.
pub fn decode(bytes: &[u8]) -> Result<DisplayList, BinError> {
    let mut r = Reader { b: bytes, pos: 0 };
    if r.bytes(4)? != MAGIC {
        return Err(BinError::BadMagic);
    }
    let version = r.u16()?;
    if version != VERSION {
        return Err(BinError::Version(version));
    }
    let flags = r.u16()?;
    let _total = r.u32()?;
    let _reserved = r.u32()?;
    let mut dl = DisplayList {
        kind: if flags & 1 == 1 {
            "page".into()
        } else {
            "paragraph".into()
        },
        unit: "sp".into(),
        ..Default::default()
    };
    let mut flag_map = serde_json::Map::new();
    let mut cur: Option<Line> = None;
    loop {
        let start = r.pos;
        let tag = r.u8()?;
        let len = r.u32()? as usize;
        let payload = r.bytes(len)?;
        let mut p = Reader { b: payload, pos: 0 };
        let malformed = |_e: BinError| BinError::Malformed { tag, offset: start };
        match tag {
            0x01 => {
                dl.width = p.i32().map_err(malformed)? as Sp;
                dl.height = p.i32().map_err(malformed)? as Sp;
                dl.depth = p.i32().map_err(malformed)? as Sp;
                let page = p.i32().map_err(malformed)?;
                dl.page = if page > 0 { Some(page as i64) } else { None };
                let pw = p.i32().map_err(malformed)?;
                let ph = p.i32().map_err(malformed)?;
                let ox = p.i32().map_err(malformed)?;
                let oy = p.i32().map_err(malformed)?;
                if dl.kind == "page" {
                    dl.page_width = Some(pw as Sp);
                    dl.page_height = Some(ph as Sp);
                    dl.origin = Some((ox as Sp, oy as Sp));
                } else {
                    dl.inserts = oy as i64; // paragraph lists carry the insert count in this slot
                }
                dl.glyphs = p.u32().map_err(malformed)? as i64;
                let _images = p.u32().map_err(malformed)?;
            }
            0x02 => {
                let id = p.u32().map_err(malformed)? as i64;
                let size = p.i32().map_err(malformed)?;
                let kind = p.u8().map_err(malformed)?;
                let _pad = p.u8().map_err(malformed)?;
                let subfont = p.u16().map_err(malformed)?;
                let slant = p.i32().map_err(malformed)?;
                let extend = p.i32().map_err(malformed)?;
                let squeeze = p.i32().map_err(malformed)?;
                let designsize = p.i32().map_err(malformed)?;
                let filename = p.str().map_err(malformed)?;
                let psname = p.str().map_err(malformed)?;
                let name = p.str().map_err(malformed)?;
                let fullname = p.str().map_err(malformed)?;
                let format = p.str().map_err(malformed)?;
                dl.fonts.insert(
                    id.to_string(),
                    FontDesc {
                        id,
                        name: opt_str(name),
                        fullname: opt_str(fullname),
                        psname: opt_str(psname),
                        filename: opt_str(filename),
                        format: opt_str(format).or_else(|| kind_name(kind).map(|s| s.to_string())),
                        kind: Some(if kind == 5 {
                            "virtual".into()
                        } else {
                            "real".into()
                        }),
                        size: Some(size as f64),
                        designsize: Some(designsize as f64),
                        slant: Some(slant as f64),
                        extend: Some(extend as f64),
                        squeeze: Some(squeeze as f64),
                        subfont: Some(subfont as i64),
                        encodingbytes: None,
                        embedding: None,
                    },
                );
            }
            0x03 => {
                let k = p.str().map_err(malformed)?;
                let v = p.i32().map_err(malformed)?;
                flag_map.insert(k, serde_json::Value::from(v));
            }
            0x10 => {
                if cur.is_some() {
                    return Err(BinError::Malformed { tag, offset: start });
                }
                let par = p.i32().map_err(malformed)? as i64;
                let i = p.i32().map_err(malformed)? as i64;
                let x = p.i32().map_err(malformed)? as Sp;
                let y = p.i32().map_err(malformed)? as Sp;
                let w = p.i32().map_err(malformed)? as Sp;
                let h = p.i32().map_err(malformed)? as Sp;
                let d = p.i32().map_err(malformed)? as Sp;
                let gs = p.f64().map_err(malformed)?;
                let gsign = p.u8().map_err(malformed)? as i64;
                let gorder = p.u8().map_err(malformed)? as i64;
                cur = Some(Line {
                    par,
                    i,
                    unit: 0,
                    row: 0,
                    x,
                    y,
                    w,
                    h,
                    d,
                    gs,
                    gsign,
                    gorder,
                    items: Vec::new(),
                });
            }
            0x12 => {
                let unit = p.i32().map_err(malformed)? as i64;
                let row = p.i32().map_err(malformed)? as i64;
                let l = cur
                    .as_mut()
                    .ok_or(BinError::Malformed { tag, offset: start })?;
                l.unit = unit;
                l.row = row;
            }
            0x27 => {
                let index = p.u32().map_err(malformed)? as i64;
                let page = p.u32().map_err(malformed)? as i64;
                let pages = p.u32().map_err(malformed)? as i64;
                let file = p.str().map_err(malformed)?;
                dl.images.insert(
                    index.to_string(),
                    ImageInfo {
                        index,
                        file,
                        page,
                        pages,
                    },
                );
            }
            0x11 => {
                let l = cur
                    .take()
                    .ok_or(BinError::Malformed { tag, offset: start })?;
                dl.lines.push(l);
            }
            0x28 => {
                let op = p.u8().map_err(malformed)?;
                let x = p.i32().map_err(malformed)? as Sp;
                let y = p.i32().map_err(malformed)? as Sp;
                let data = p.str().map_err(malformed)?;
                let item = Item::Matrix {
                    op: match op {
                        0 => "save",
                        1 => "set",
                        _ => "restore",
                    }
                    .to_string(),
                    x,
                    y,
                    data,
                };
                match cur.as_mut() {
                    Some(l) => l.items.push(item),
                    None => dl.other.push(item),
                }
            }
            0x20..=0x26 => {
                let item = match tag {
                    0x20 => {
                        let font = p.u32().map_err(malformed)? as i64;
                        let y = p.i32().map_err(malformed)? as Sp;
                        let ef = p.i32().map_err(malformed)? as i64;
                        let n = p.u32().map_err(malformed)? as usize;
                        let target = cur.as_mut().map(|l| &mut l.items).unwrap_or(&mut dl.other);
                        for _ in 0..n {
                            let ch = p.u32().map_err(malformed)? as i64;
                            let idx = p.u32().map_err(malformed)?;
                            let x = p.i32().map_err(malformed)? as Sp;
                            let w = p.i32().map_err(malformed)? as Sp;
                            target.push(Item::Glyph {
                                font,
                                char: ch,
                                index: if idx == u32::MAX {
                                    None
                                } else {
                                    Some(idx as i64)
                                },
                                x,
                                y,
                                width: w,
                                expansion: ef,
                            });
                        }
                        continue;
                    }
                    0x21 => Item::Rule {
                        x: p.i32().map_err(malformed)? as Sp,
                        y_top: p.i32().map_err(malformed)? as Sp,
                        width: p.i32().map_err(malformed)? as Sp,
                        height: p.i32().map_err(malformed)? as Sp,
                    },
                    0x22 => {
                        let cmd = p.u8().map_err(malformed)?;
                        let _pad = p.u8().map_err(malformed)?;
                        let stack = p.u16().map_err(malformed)? as i64;
                        let data = p.str().map_err(malformed)?;
                        Item::Color {
                            stack,
                            cmd: if cmd == 255 { None } else { Some(cmd as i64) },
                            data,
                        }
                    }
                    0x23 => Item::Literal {
                        mode: p.i32().map_err(malformed)? as i64,
                        data: p.str().map_err(malformed)?,
                    },
                    0x24 => {
                        let kind = p.str().map_err(malformed)?;
                        let detail = p.str().map_err(malformed)?;
                        Item::Unsupported {
                            kind,
                            detail: serde_json::Value::String(detail),
                        }
                    }
                    0x25 => Item::Math {
                        on: p.u8().map_err(malformed)? == 1,
                        x: p.i32().map_err(malformed)? as Sp,
                    },
                    0x26 => Item::Image {
                        index: p.i32().map_err(malformed)? as i64,
                        x: p.i32().map_err(malformed)? as Sp,
                        y_top: p.i32().map_err(malformed)? as Sp,
                        width: p.i32().map_err(malformed)? as Sp,
                        height: p.i32().map_err(malformed)? as Sp,
                    },
                    _ => unreachable!(),
                };
                match cur.as_mut() {
                    Some(l) => l.items.push(item),
                    None => dl.other.push(item),
                }
            }
            0x29 => {
                dl.math_owners = Some(
                    read_math_owners(&mut p)
                        .map_err(|_| BinError::Malformed { tag, offset: start })?,
                );
            }
            0xFF => break,
            _ => { /* unknown record: skipped (forward compatibility) */ }
        }
    }
    dl.flags = serde_json::Value::Object(flag_map);
    Ok(dl)
}

/// Record 0x29: u32 glyph count, glyphs (i32 owner, u32 font, u32 char, i32 bbox[4]); u32 field
/// count, fields (i32 owner, u8 role, u8 empty, u16 0, i32 bbox[4]). Nothing may follow.
pub(super) fn read_math_owners(p: &mut Reader) -> Result<crate::MathOwners, BinError> {
    const GLYPH: usize = 28;
    const FIELD: usize = 24;
    let bad = |p: &Reader| BinError::Malformed {
        tag: 0x29,
        offset: p.pos,
    };
    let ng = p.u32()? as usize;
    p.need(ng.checked_mul(GLYPH).ok_or_else(|| bad(p))?)?;
    let mut glyphs = Vec::with_capacity(ng);
    for _ in 0..ng {
        glyphs.push(crate::OwnedGlyph {
            owner: p.i32()? as i64,
            font: p.u32()? as i64,
            char: p.u32()? as i64,
            bbox: read_bbox(p)?,
        });
    }
    let nf = p.u32()? as usize;
    p.need(nf.checked_mul(FIELD).ok_or_else(|| bad(p))?)?;
    let mut fields = Vec::with_capacity(nf);
    for _ in 0..nf {
        let owner = p.i32()? as i64;
        let role = crate::MathRole::from_code(p.u8()?).ok_or_else(|| bad(p))?;
        let empty = match p.u8()? {
            0 => false,
            1 => true,
            _ => return Err(bad(p)),
        };
        p.u16()?; // reserved
        fields.push(crate::MathField {
            owner,
            role,
            bbox: read_bbox(p)?,
            empty,
        });
    }
    if p.pos != p.b.len() {
        return Err(bad(p));
    }
    Ok(crate::MathOwners { glyphs, fields })
}

pub(super) fn read_bbox(p: &mut Reader) -> Result<[Sp; 4], BinError> {
    Ok([
        p.i32()? as Sp,
        p.i32()? as Sp,
        p.i32()? as Sp,
        p.i32()? as Sp,
    ])
}
