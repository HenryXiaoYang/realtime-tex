use super::decode::*;
use super::encode::*;
use super::*;
use crate::{DisplayList, FontDesc, Item, Line};
use std::collections::BTreeMap;
#[test]
fn items_after_a_matrix_survive_encoding() {
    let m = |op: &str| Item::Matrix {
        op: op.into(),
        x: 10,
        y: 20,
        data: if op == "set" {
            "2 0 0 2".into()
        } else {
            String::new()
        },
    };
    let rule = Item::Rule {
        x: 1,
        y_top: 2,
        width: 3,
        height: 4,
    };
    let mut dl = sample();
    dl.other = vec![m("save"), m("set"), rule.clone(), m("restore"), rule];
    assert_eq!(decode(&dl.to_binary()).unwrap().other, dl.other);
}

#[test]
fn math_owners_roundtrip() {
    let mut dl = sample();
    let plain = dl.to_binary();
    let json = serde_json::to_string(&dl).unwrap();
    assert!(!json.contains("math_owners"));
    let owners = crate::MathOwners {
        glyphs: vec![crate::OwnedGlyph {
            owner: -42,
            font: 27,
            char: 65,
            bbox: [-100, 20, 300, 400],
        }],
        fields: vec![crate::MathField {
            owner: 0,
            role: crate::MathRole::Num,
            bbox: [5, 6, 5, 6],
            empty: true,
        }],
    };
    dl.math_owners = Some(owners.clone());
    let binary = dl.to_binary();
    let decoded = decode(&binary).unwrap();
    assert_eq!(decoded.math_owners, Some(owners.clone()));
    assert_eq!(decoded.to_binary(), binary);
    let json = serde_json::to_string(&decoded).unwrap();
    assert_eq!(
        DisplayList::from_json(&json).unwrap().math_owners,
        Some(owners)
    );
    dl.math_owners = None;
    assert_eq!(dl.to_binary(), plain);
}

#[test]
fn math_owner_fields_reject_bad_codes_and_trailing_bytes() {
    let field = |role: u8, empty: u8, extra: &[u8]| {
        let mut q = Writer(Vec::new());
        q.u32(0); // no glyphs
        q.u32(1);
        q.i32(7);
        q.u8(role);
        q.u8(empty);
        q.u16(0);
        [1, 2, 3, 4].iter().for_each(|&v| q.i32(v));
        q.0.extend_from_slice(extra);
        let mut body = Writer(Vec::new());
        body.rec(0x29, &q.0);
        body.rec(0xFF, &[]);
        let mut bytes = sample().to_binary()[..16].to_vec();
        bytes.extend(body.0);
        decode(&bytes)
    };
    let ok = field(crate::MathRole::Cell.code(), 1, &[]).unwrap();
    let f = &ok.math_owners.unwrap().fields[0];
    assert_eq!(
        (f.owner, f.role, f.empty, f.bbox),
        (7, crate::MathRole::Cell, true, [1, 2, 3, 4])
    );
    for bad in [
        field(0, 0, &[]),
        field(9, 0, &[]),
        field(1, 2, &[]),
        field(1, 0, &[0]),
    ] {
        assert!(matches!(bad, Err(BinError::Malformed { tag: 0x29, .. })));
    }
}

#[test]
fn malformed_math_owners_is_an_error() {
    let mut body = Writer(Vec::new());
    // One glyph announced, none present.
    body.rec(0x29, &1u32.to_le_bytes());
    body.rec(0xFF, &[]);
    let mut bytes = sample().to_binary()[..16].to_vec();
    bytes.extend(body.0);
    assert!(matches!(
        decode(&bytes),
        Err(BinError::Malformed { tag: 0x29, .. })
    ));
}

fn sample() -> DisplayList {
    let mut fonts = BTreeMap::new();
    fonts.insert(
        "27".to_string(),
        FontDesc {
            id: 27,
            name: Some("TeXGyrePagella".into()),
            fullname: None,
            psname: Some("TeXGyrePagella-Regular".into()),
            filename: Some("/f/texgyrepagella-regular.otf".into()),
            format: Some("opentype".into()),
            kind: Some("real".into()),
            size: Some(717619.0),
            designsize: Some(0.0),
            slant: Some(0.0),
            extend: Some(1000.0),
            squeeze: Some(0.0),
            subfont: Some(0),
            encodingbytes: None,
            embedding: None,
        },
    );
    DisplayList {
        math_owners: None,
        kind: "page".into(),
        unit: "sp".into(),
        fonts,
        inserts: 0,
        images: BTreeMap::new(),
        lines: vec![Line {
            par: 3,
            i: 1,
            unit: 7,
            row: 2,
            x: 100,
            y: 2000,
            w: 5000,
            h: 500,
            d: 200,
            gs: 0.25,
            gsign: 1,
            gorder: 0,
            items: vec![
                Item::Glyph {
                    font: 27,
                    char: 65,
                    index: Some(36),
                    x: 100,
                    y: 2000,
                    width: 400,
                    expansion: 20000,
                },
                Item::Glyph {
                    font: 27,
                    char: 66,
                    index: Some(37),
                    x: 500,
                    y: 2000,
                    width: 420,
                    expansion: 20000,
                },
                Item::Rule {
                    x: 950,
                    y_top: 1500,
                    width: 30,
                    height: 700,
                },
                Item::Color {
                    stack: 0,
                    cmd: Some(1),
                    data: "1 0 0 rg".into(),
                },
                Item::Glyph {
                    font: 27,
                    char: 67,
                    index: None,
                    x: 1000,
                    y: 2000,
                    width: 400,
                    expansion: 0,
                },
                Item::Color {
                    stack: 0,
                    cmd: Some(2),
                    data: String::new(),
                },
                Item::Math { on: true, x: 1400 },
                Item::Image {
                    index: 2,
                    x: 1500,
                    y_top: 1200,
                    width: 800,
                    height: 800,
                },
                Item::Literal {
                    mode: 0,
                    data: "q Q".into(),
                },
                Item::Unsupported {
                    kind: "leaders".into(),
                    detail: serde_json::Value::String("101".into()),
                },
            ],
        }],
        other: vec![Item::Color {
            stack: 0,
            cmd: Some(0),
            data: "0 g 0 G".into(),
        }],
        flags: serde_json::json!({"literal": 1}),
        glyphs: 3,
        width: 5000,
        height: 700,
        depth: 200,
        page: Some(4),
        page_width: Some(40000),
        page_height: Some(52000),
        origin: Some((4736286, 4736286)),
    }
}
#[test]
fn roundtrip() {
    let dl = sample();
    let bytes = encode(&dl);
    assert_eq!(&bytes[..4], b"RTDL");
    let back = decode(&bytes).unwrap();
    assert_eq!(
        serde_json::to_value(&back).unwrap(),
        serde_json::to_value(&dl).unwrap()
    );
    // unknown record is skipped
    let mut with_unknown = bytes.clone();
    let end = with_unknown.len() - 5; // before END record
    with_unknown.splice(end..end, [0x7E, 3, 0, 0, 0, 1, 2, 3]);
    assert!(decode(&with_unknown).is_ok());
    // truncation is detected
    assert!(decode(&bytes[..bytes.len() - 3]).is_err());
}
