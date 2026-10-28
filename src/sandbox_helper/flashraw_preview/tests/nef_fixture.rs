// SPDX-License-Identifier: MIT
// Derived from FlashRaw 0.1.1 preview_fallback.rs, copyright 2026 Yassine Bouanane and FlashRaw contributors.
// The unsupported sensor payload ensures thumbnails cannot pass by developing the sensor.

const W: u32 = 64;
const H: u32 = 48;

fn ifd(entries: &[(u16, u16, u32, Vec<u8>)], extra_base: usize, next: u32) -> (Vec<u8>, Vec<u8>) {
    let mut e = entries.to_vec();
    e.sort_by_key(|x| x.0);
    let mut out = (e.len() as u16).to_le_bytes().to_vec();
    let mut extra = vec![];
    let dir_len = 2 + e.len() * 12 + 4;
    for (tag, typ, count, v) in &e {
        out.extend(tag.to_le_bytes());
        out.extend(typ.to_le_bytes());
        out.extend(count.to_le_bytes());
        if v.len() <= 4 {
            let mut v = v.clone();
            v.resize(4, 0);
            out.extend(v);
        } else {
            out.extend(((extra_base + dir_len + extra.len()) as u32).to_le_bytes());
            extra.extend(v);
            if extra.len() % 2 == 1 {
                extra.push(0);
            }
        }
    }
    out.extend(next.to_le_bytes());
    (out, extra)
}

fn short(v: u16) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}
fn long(v: u32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

pub fn high_efficiency_nef(pw: u32, ph: u32, orientation: u16) -> Vec<u8> {
    let preview: Vec<u8> = (0..ph)
        .flat_map(|_| {
            (0..pw).flat_map(move |x| [(x * 255 / pw.max(2).saturating_sub(1)) as u8, 100, 200])
        })
        .collect();
    let payload: Vec<u8> = [0xff, 0x10, 0xff, 0x50]
        .into_iter()
        .chain((0..96u8).map(|i| i.wrapping_mul(37)))
        .collect();
    let ifd0_at = 8usize;
    let make = b"NIKON CORPORATION\0".to_vec();
    let model = b"NIKON Z 8\0".to_vec();
    let ifd0_entries = |raw_at: u32, preview_at: u32| {
        vec![
            (0x00FE, 4, 1, long(1)),
            (0x0100, 4, 1, long(pw)),
            (0x0101, 4, 1, long(ph)),
            (
                0x0102,
                3,
                3,
                [8u16, 8, 8].iter().flat_map(|v| v.to_le_bytes()).collect(),
            ),
            (0x0103, 3, 1, short(1)),
            (0x0106, 3, 1, short(2)),
            (0x010F, 2, make.len() as u32, make.clone()),
            (0x0110, 2, model.len() as u32, model.clone()),
            (0x0111, 4, 1, long(preview_at)),
            (0x0112, 3, 1, short(orientation)),
            (0x0115, 3, 1, short(3)),
            (0x0116, 4, 1, long(ph)),
            (0x0117, 4, 1, long(pw * ph * 3)),
            (0x014A, 4, 1, long(raw_at)),
        ]
    };
    let raw_entries = |payload_at: u32| {
        vec![
            (0x00FE, 4, 1, long(0)),
            (0x0100, 4, 1, long(W)),
            (0x0101, 4, 1, long(H)),
            (0x0102, 3, 1, short(14)),
            (0x0103, 3, 1, short(34713)),
            (0x0106, 3, 1, short(32803)),
            (0x0111, 4, 1, long(payload_at)),
            (0x0115, 3, 1, short(1)),
            (0x0116, 4, 1, long(H)),
            (0x0117, 4, 1, long(payload.len() as u32)),
        ]
    };
    let (d0, x0) = ifd(&ifd0_entries(0, 0), ifd0_at, 0);
    let raw_at = ifd0_at + d0.len() + x0.len();
    let (d1, x1) = ifd(&raw_entries(0), raw_at, 0);
    let preview_at = raw_at + d1.len() + x1.len();
    let payload_at = preview_at + preview.len();
    let (d0, x0) = ifd(&ifd0_entries(raw_at as u32, preview_at as u32), ifd0_at, 0);
    let (d1, x1) = ifd(&raw_entries(payload_at as u32), raw_at, 0);
    let mut out = b"II*\0".to_vec();
    out.extend(long(ifd0_at as u32));
    for part in [d0, x0, d1, x1, preview, payload] {
        out.extend(part);
    }
    out
}
