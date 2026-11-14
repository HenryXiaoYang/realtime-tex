//! Encoder: a `DisplayList` to bytes, the inverse of `decode`.

use super::{MAGIC, VERSION};
use crate::{DisplayList, FontDesc, Item};

pub(super) fn kind_code(d: &FontDesc) -> u8 {
    if d.kind.as_deref() == Some("virtual") {
        return 5;
    }
    match d.format.as_deref() {
        Some("opentype") => 1,
        Some("truetype") => 2,
        Some("type1") => 3,
        Some("type3") => 4,
        _ => 0,
    }
}

pub(super) fn write_math_owners(m: &crate::MathOwners) -> Vec<u8> {
    let mut q = Writer(Vec::with_capacity(
        8 + 28 * m.glyphs.len() + 24 * m.fields.len(),
    ));
    q.u32(m.glyphs.len() as u32);
    for g in &m.glyphs {
        q.i32(g.owner);
        q.u32(g.font as u32);
        q.u32(g.char as u32);
        g.bbox.iter().for_each(|&v| q.i32(v));
    }
    q.u32(m.fields.len() as u32);
    for f in &m.fields {
        q.i32(f.owner);
        q.u8(f.role.code());
        q.u8(f.empty as u8);
        q.u16(0);
        f.bbox.iter().for_each(|&v| q.i32(v));
    }
    q.0
}

pub(super) struct Writer(pub(super) Vec<u8>);

impl Writer {
    pub(super) fn u8(&mut self, v: u8) {
        self.0.push(v)
    }
    pub(super) fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes())
    }
    pub(super) fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes())
    }
    pub(super) fn i32(&mut self, v: i64) {
        self.0.extend_from_slice(&(v as i32).to_le_bytes())
    }
    pub(super) fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_le_bytes())
    }
    pub(super) fn str(&mut self, s: &str) {
        let b = &s.as_bytes()[..s.len().min(65535)];
        self.u16(b.len() as u16);
        self.0.extend_from_slice(b);
    }
    pub(super) fn rec(&mut self, tag: u8, payload: &[u8]) {
        self.u8(tag);
        self.u32(payload.len() as u32);
        self.0.extend_from_slice(payload);
    }
}

pub(super) fn write_items(out: &mut Writer, items: &[Item]) {
    let mut i = 0;
    while i < items.len() {
        match &items[i] {
            Item::Glyph {
                font, y, expansion, ..
            } => {
                let (f0, y0, e0) = (*font, *y, *expansion);
                let mut j = i;
                let mut run = Writer(Vec::new());
                let mut n = 0u32;
                while j < items.len() {
                    if let Item::Glyph {
                        font,
                        char,
                        index,
                        x,
                        y,
                        width,
                        expansion,
                    } = &items[j]
                    {
                        if *font != f0 || *y != y0 || *expansion != e0 {
                            break;
                        }
                        run.u32(*char as u32);
                        run.u32(index.map(|v| v as u32).unwrap_or(u32::MAX));
                        run.i32(*x);
                        run.i32(*width);
                        n += 1;
                        j += 1;
                    } else {
                        break;
                    }
                }
                let mut p = Writer(Vec::new());
                p.u32(f0 as u32);
                p.i32(y0);
                p.i32(e0);
                p.u32(n);
                p.0.extend_from_slice(&run.0);
                out.rec(0x20, &p.0);
                i = j;
                continue;
            }
            Item::Rule {
                x,
                y_top,
                width,
                height,
            } => {
                let mut p = Writer(Vec::new());
                p.i32(*x);
                p.i32(*y_top);
                p.i32(*width);
                p.i32(*height);
                out.rec(0x21, &p.0);
            }
            Item::Color { stack, cmd, data } => {
                let mut p = Writer(Vec::new());
                p.u8(cmd.map(|c| c as u8).unwrap_or(255));
                p.u8(0);
                p.u16(*stack as u16);
                p.str(data);
                out.rec(0x22, &p.0);
            }
            Item::Literal { mode, data } => {
                let mut p = Writer(Vec::new());
                p.i32(*mode);
                p.str(data);
                out.rec(0x23, &p.0);
            }
            Item::Unsupported { kind, detail } => {
                let mut p = Writer(Vec::new());
                p.str(kind);
                p.str(&match detail {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Null => String::new(),
                    other => other.to_string(),
                });
                out.rec(0x24, &p.0);
            }
            Item::Math { on, x } => {
                let mut p = Writer(Vec::new());
                p.u8(*on as u8);
                p.i32(*x);
                out.rec(0x25, &p.0);
            }
            Item::Matrix { op, x, y, data } => {
                let mut p = Writer(Vec::new());
                p.u8(match op.as_str() {
                    "save" => 0,
                    "set" => 1,
                    _ => 2,
                });
                p.i32(*x);
                p.i32(*y);
                p.str(data);
                out.rec(0x28, &p.0);
            }
            Item::Image {
                index,
                x,
                y_top,
                width,
                height,
            } => {
                let mut p = Writer(Vec::new());
                p.i32(*index);
                p.i32(*x);
                p.i32(*y_top);
                p.i32(*width);
                p.i32(*height);
                out.rec(0x26, &p.0);
            }
        }
        i += 1;
    }
}

/// Encode a display list into the binary format.
pub fn encode(dl: &DisplayList) -> Vec<u8> {
    let mut body = Writer(Vec::new());
    {
        let mut p = Writer(Vec::new());
        p.i32(dl.width);
        p.i32(dl.height);
        p.i32(dl.depth);
        p.i32(dl.page.unwrap_or(0));
        p.i32(dl.page_width.unwrap_or(0));
        p.i32(dl.page_height.unwrap_or(0));
        let (ox, oy) = dl.origin.unwrap_or((0, 0));
        p.i32(ox);
        p.i32(if dl.kind == "page" { oy } else { dl.inserts });
        p.u32(dl.glyphs as u32);
        let images = dl
            .lines
            .iter()
            .flat_map(|l| l.items.iter())
            .chain(dl.other.iter())
            .filter(|i| matches!(i, Item::Image { .. }))
            .count();
        p.u32(images as u32);
        body.rec(0x01, &p.0);
    }
    for d in dl.fonts.values() {
        let mut p = Writer(Vec::new());
        p.u32(d.id as u32);
        p.i32(d.size.unwrap_or(0.0).round() as i64);
        p.u8(kind_code(d));
        p.u8(0);
        p.u16(d.subfont.unwrap_or(0) as u16);
        p.i32(d.slant.unwrap_or(0.0).round() as i64);
        p.i32(d.extend.unwrap_or(0.0).round() as i64);
        p.i32(d.squeeze.unwrap_or(0.0).round() as i64);
        p.i32(d.designsize.unwrap_or(0.0).round() as i64);
        p.str(d.filename.as_deref().unwrap_or(""));
        p.str(d.psname.as_deref().unwrap_or(""));
        p.str(d.name.as_deref().unwrap_or(""));
        p.str(d.fullname.as_deref().unwrap_or(""));
        p.str(d.format.as_deref().unwrap_or(""));
        body.rec(0x02, &p.0);
    }
    for (k, v) in dl.flags_map() {
        let mut p = Writer(Vec::new());
        p.str(&k);
        p.i32(v.as_i64().unwrap_or(1));
        body.rec(0x03, &p.0);
    }
    for im in dl.images.values() {
        let mut p = Writer(Vec::new());
        p.u32(im.index as u32);
        p.u32(im.page as u32);
        p.u32(im.pages as u32);
        p.str(&im.file);
        body.rec(0x27, &p.0);
    }
    write_items(&mut body, &dl.other);
    for l in &dl.lines {
        let mut p = Writer(Vec::new());
        p.i32(l.par);
        p.i32(l.i);
        p.i32(l.x);
        p.i32(l.y);
        p.i32(l.w);
        p.i32(l.h);
        p.i32(l.d);
        p.f64(l.gs);
        p.u8(l.gsign as u8);
        p.u8(l.gorder as u8);
        body.rec(0x10, &p.0);
        if l.unit != 0 {
            let mut q = Writer(Vec::new());
            q.i32(l.unit);
            q.i32(l.row);
            body.rec(0x12, &q.0);
        }
        write_items(&mut body, &l.items);
        body.rec(0x11, &[]);
    }
    if let Some(owners) = &dl.math_owners {
        body.rec(0x29, &write_math_owners(owners));
    }
    body.rec(0xFF, &[]);
    let mut out = Writer(Vec::new());
    out.0.extend_from_slice(MAGIC);
    out.u16(VERSION);
    out.u16(if dl.kind == "page" { 1 } else { 0 });
    out.u32(16 + body.0.len() as u32);
    out.u32(0);
    out.0.extend_from_slice(&body.0);
    out.0
}
