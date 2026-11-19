//! What no face has, on the web, where the system's faces aren't the renderer's to read: drawn by the browser, as
//! Flutter web's CanvasKit draws an emoji by the browser's fonts. Each grapheme cluster none had is a glyph of a font
//! made here ("Browser n", made again as clusters come), as wide as the browser's canvas measures it, its characters
//! in its `cmap` and, where it's more than one (a flag, a family, a skin tone), a ligature of theirs (`ccmp`), so
//! Parley shapes, measures and wraps it as any glyph; its pixels are the canvas's (`platform::web::glyph`), in colour
//! where the browser draws it so (an emoji) and the text's colour where it doesn't (a symbol).
//!
//! The font is the few tables OpenType says a TrueType font has, with no outlines: its glyphs are drawn by the browser.
//! It's written here, not by a font-writing library (fontations' write-fonts), which would be a third copy of
//! read-fonts in the WebAssembly every browser downloads.

use std::cell::RefCell;

/// Its units to an em.
const UNITS: f32 = 1000.0;
const ASCENT: i16 = 900;
const DESCENT: i16 = -250;

#[derive(Default)]
struct Browser {
    /// Each glyph's text, after `.notdef`, its advance in units, and whether the browser draws it in colour.
    glyphs: Vec<(String, u16, bool)>,
    /// How many fonts it's been made as, and the blobs of each, by fontique's id.
    made: u32,
    blobs: Vec<u64>,
}

thread_local! {
    static BROWSER: RefCell<Browser> = RefCell::new(Browser::default());
}

/// The font that has the grapheme cluster of `text` at byte `at` too, if it's new, by its family name, and its bytes,
/// which the caller registers and says the blob of (`registered`).
pub fn cover(text: &str, at: usize) -> Option<(String, Vec<u8>)> {
    use unicode_segmentation::UnicodeSegmentation as _;
    let cluster = text.grapheme_indices(true).find(|(start, cluster)| (*start..start + cluster.len()).contains(&at))?.1;
    BROWSER.with_borrow_mut(|browser| {
        let mut added = false;
        // The cluster, and each of its characters alone, as each may come alone too.
        for part in std::iter::once(cluster.to_owned()).chain(cluster.chars().map(String::from)) {
            if !browser.glyphs.iter().any(|(text, ..)| *text == part) {
                let (advance, colored) = crate::platform::web::measure(&part);
                browser.glyphs.push((part, (advance * UNITS).round().clamp(0.0, f32::from(u16::MAX)) as u16, colored));
                added = true;
            }
        }
        if !added {
            return None;
        }
        browser.made += 1;
        let family = format!("Browser {}", browser.made);
        Some((family.clone(), font(&family, &browser.glyphs)))
    })
}

/// The blob a font `cover` made was registered as.
pub fn registered(blob: u64) {
    BROWSER.with_borrow_mut(|browser| browser.blobs.push(blob));
}

/// Whether `family` is one of its fonts.
pub fn is_family(family: &str) -> bool {
    family.starts_with("Browser ")
}

/// Glyph `glyph` of the font in blob `blob`, if it's one of its fonts: its text, and whether it's drawn in colour.
pub fn glyph(blob: u64, glyph: u32) -> Option<(String, bool)> {
    BROWSER.with_borrow(|browser| {
        if !browser.blobs.contains(&blob) || glyph == 0 {
            return None;
        }
        browser.glyphs.get(glyph as usize - 1).map(|(text, _, colored)| (text.clone(), *colored))
    })
}

/// The font: `.notdef`, then each glyph, mapped from its character where it's one, and as a ligature of its
/// characters' glyphs where it's more.
fn font(family: &str, glyphs: &[(String, u16, bool)]) -> Vec<u8> {
    let count = glyphs.len() as u16 + 1;
    let id_of = |text: &str| glyphs.iter().position(|(own, ..)| own == text).map(|at| at as u16 + 1);
    let mut cmap: Vec<(u32, u16)> = glyphs.iter().enumerate().filter(|(_, (text, ..))| text.chars().count() == 1).map(|(at, (text, ..))| (text.chars().next().map_or(0, u32::from), at as u16 + 1)).collect();
    cmap.sort();
    let ligatures: Vec<(Vec<u16>, u16)> = glyphs
        .iter()
        .enumerate()
        .filter(|(_, (text, ..))| text.chars().count() > 1)
        .filter_map(|(at, (text, ..))| Some((text.chars().map(|character| id_of(&character.to_string())).collect::<Option<Vec<u16>>>()?, at as u16 + 1)))
        .collect();
    let widest = glyphs.iter().map(|(_, advance, _)| *advance).max().unwrap_or(0);

    let mut head = Vec::new();
    for value in [0x0001_0000u32, 0x0001_0000, 0, 0x5F0F_3CF5] {
        head.extend(value.to_be_bytes());
    }
    for value in [0x000Bu16, UNITS as u16] {
        head.extend(value.to_be_bytes());
    }
    head.extend([0u8; 16]);
    for value in [0i16, DESCENT, widest as i16, ASCENT, 0, 8, 2, 1, 0] {
        head.extend(value.to_be_bytes());
    }

    let mut hhea = Vec::new();
    hhea.extend(0x0001_0000u32.to_be_bytes());
    for value in [ASCENT, DESCENT, 0, widest as i16, 0, 0, widest as i16, 1, 0, 0, 0, 0, 0, 0, 0, count as i16] {
        hhea.extend(value.to_be_bytes());
    }

    let mut maxp = Vec::new();
    maxp.extend(0x0001_0000u32.to_be_bytes());
    for value in [count, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
        maxp.extend(value.to_be_bytes());
    }

    let mut os2 = Vec::new();
    for value in [4u16, widest / 2, 400, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 50, 250, 0] {
        os2.extend(value.to_be_bytes());
    }
    os2.extend([0u8; 10 + 16]);
    os2.extend(*b"ONE ");
    let (first, last) = (cmap.first().map_or(0, |(code, _)| (*code).min(0xFFFF) as u16), cmap.last().map_or(0, |(code, _)| (*code).min(0xFFFF) as u16));
    for value in [0x00C0u16, first, last] {
        os2.extend(value.to_be_bytes());
    }
    for value in [ASCENT, DESCENT, 0] {
        os2.extend(value.to_be_bytes());
    }
    for value in [ASCENT as u16, (-DESCENT) as u16] {
        os2.extend(value.to_be_bytes());
    }
    os2.extend([0u8; 8]);
    for value in [500u16, 700, 0, 32, 4] {
        os2.extend(value.to_be_bytes());
    }

    let mut hmtx = Vec::new();
    for advance in std::iter::once(500).chain(glyphs.iter().map(|(_, advance, _)| *advance)) {
        hmtx.extend(advance.to_be_bytes());
        hmtx.extend(0i16.to_be_bytes());
    }

    let mut subtable = Vec::new();
    subtable.extend(12u16.to_be_bytes());
    subtable.extend(0u16.to_be_bytes());
    subtable.extend((16 + 12 * cmap.len() as u32).to_be_bytes());
    subtable.extend(0u32.to_be_bytes());
    subtable.extend((cmap.len() as u32).to_be_bytes());
    for (code, glyph) in &cmap {
        subtable.extend(code.to_be_bytes());
        subtable.extend(code.to_be_bytes());
        subtable.extend(u32::from(*glyph).to_be_bytes());
    }
    let mut cmap_table = Vec::new();
    for value in [0u16, 1, 3, 10] {
        cmap_table.extend(value.to_be_bytes());
    }
    cmap_table.extend(12u32.to_be_bytes());
    cmap_table.extend(subtable);

    let loca = vec![0u8; (usize::from(count) + 1) * 4];

    let mut post = Vec::new();
    post.extend(0x0003_0000u32.to_be_bytes());
    post.extend(0u32.to_be_bytes());
    post.extend((-100i16).to_be_bytes());
    post.extend(50i16.to_be_bytes());
    post.extend([0u8; 20]);

    let names: [(u16, String); 4] = [(1, family.to_owned()), (2, "Regular".into()), (4, family.to_owned()), (6, family.replace(' ', ""))];
    let mut strings = Vec::new();
    let mut name = Vec::new();
    for value in [0u16, names.len() as u16, 6 + 12 * names.len() as u16] {
        name.extend(value.to_be_bytes());
    }
    for (id, text) in &names {
        let encoded: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for value in [3u16, 1, 0x409, *id, encoded.len() as u16, strings.len() as u16] {
            name.extend(value.to_be_bytes());
        }
        strings.extend(encoded);
    }
    name.extend(strings);

    let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"OS/2", os2),
        (*b"cmap", cmap_table),
        (*b"glyf", Vec::new()),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"maxp", maxp),
        (*b"name", name),
        (*b"post", post),
    ];
    if !ligatures.is_empty() {
        tables.push((*b"GSUB", gsub(&ligatures)));
    }
    tables.sort_by_key(|(tag, _)| *tag);
    sfnt(&tables)
}

/// A GSUB with one lookup, under `ccmp` for every script, of each ligature: its components' glyphs to its own.
fn gsub(ligatures: &[(Vec<u16>, u16)]) -> Vec<u8> {
    let u16s = |values: &[u16]| values.iter().flat_map(|value| value.to_be_bytes()).collect::<Vec<u8>>();
    // By their first glyph, the longest first, as a ligature set's are tried in order.
    let mut firsts: Vec<u16> = ligatures.iter().map(|(components, _)| components[0]).collect();
    firsts.sort();
    firsts.dedup();
    let sets: Vec<Vec<u8>> = firsts
        .iter()
        .map(|first| {
            let mut those: Vec<&(Vec<u16>, u16)> = ligatures.iter().filter(|(components, _)| components[0] == *first).collect();
            those.sort_by_key(|(components, _)| std::cmp::Reverse(components.len()));
            let bodies: Vec<Vec<u8>> = those.iter().map(|(components, glyph)| [u16s(&[*glyph, components.len() as u16]), u16s(&components[1..])].concat()).collect();
            let mut set = u16s(&[bodies.len() as u16]);
            let mut at = 2 + 2 * bodies.len();
            for body in &bodies {
                set.extend(u16s(&[at as u16]));
                at += body.len();
            }
            set.extend(bodies.concat());
            set
        })
        .collect();
    let coverage = [u16s(&[1, firsts.len() as u16]), u16s(&firsts)].concat();
    let mut subtable = u16s(&[1, 0, sets.len() as u16]);
    let mut at = 6 + 2 * sets.len();
    let coverage_at = at;
    at += coverage.len();
    for set in &sets {
        subtable.extend(u16s(&[at as u16]));
        at += set.len();
    }
    subtable[2..4].copy_from_slice(&(coverage_at as u16).to_be_bytes());
    subtable.extend(coverage);
    subtable.extend(sets.concat());
    let lookup = [u16s(&[4, 0, 1, 8]), subtable].concat();
    let lookups = [u16s(&[1, 4]), lookup].concat();
    // A script list of DFLT with a default language system, and a feature list of ccmp, each of its one lookup.
    let scripts = [u16s(&[1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0])].concat();
    let features = [u16s(&[1]), b"ccmp".to_vec(), u16s(&[8, 0, 1, 0])].concat();
    let header_len = 10;
    [u16s(&[1, 0, header_len, header_len + scripts.len() as u16, header_len + (scripts.len() + features.len()) as u16]), scripts, features, lookups].concat()
}

/// The tables, sorted by tag, as one font file: its table directory, then each table, four-byte aligned.
fn sfnt(tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let count = tables.len() as u16;
    let power = 1u16 << (15 - count.leading_zeros() as u16);
    let mut out = Vec::new();
    out.extend(0x0001_0000u32.to_be_bytes());
    for value in [count, power * 16, power.trailing_zeros() as u16, count * 16 - power * 16] {
        out.extend(value.to_be_bytes());
    }
    let mut at = 12 + 16 * tables.len();
    let mut bodies = Vec::new();
    for (tag, table) in tables {
        let checksum = table.chunks(4).fold(0u32, |sum, word| {
            let mut padded = [0u8; 4];
            padded[..word.len()].copy_from_slice(word);
            sum.wrapping_add(u32::from_be_bytes(padded))
        });
        out.extend(tag);
        out.extend(checksum.to_be_bytes());
        out.extend((at as u32).to_be_bytes());
        out.extend((table.len() as u32).to_be_bytes());
        let mut body = table.clone();
        body.resize(body.len().div_ceil(4) * 4, 0);
        at += body.len();
        bodies.push(body);
    }
    out.extend(bodies.concat());
    out
}
