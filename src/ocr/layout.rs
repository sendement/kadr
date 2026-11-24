//! Reading order, text selection and how copied text is laid out: prose is joined into
//! paragraphs (line breaks and hyphenation removed), code keeps its lines and indentation,
//! reconstructed from where the characters sit horizontally.
use super::{OChar, OLine};
use crate::geom::{P, R};

/// Position between characters: `ch` = index of the character after the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Pos {
    pub line: usize,
    pub ch: usize,
}

fn vcenter(r: &R) -> f32 {
    r.y + r.h / 2.0
}

fn same_row(a: &R, b: &R) -> bool {
    let h = a.h.min(b.h);
    (vcenter(a) - vcenter(b)).abs() < h * 0.5
}

/// Group segments into blocks: words of one line (small gap) and lines stacked closely
/// with horizontal overlap belong together; separate columns and panels do not.
fn blocks(rects: &[R]) -> Vec<Vec<usize>> {
    let n = rects.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for i in 0..n {
        for j in i + 1..n {
            let (a, b) = (&rects[i], &rects[j]);
            let h = a.h.min(b.h);
            let linked = if same_row(a, b) {
                let gap = if a.x < b.x { b.x - a.right() } else { a.x - b.right() };
                gap < 2.0 * h
            } else {
                let vgap = if a.y < b.y { b.y - a.bottom() } else { a.y - b.bottom() };
                let overlap = a.right().min(b.right()) - a.x.max(b.x);
                vgap < 0.9 * h && overlap > -3.0 * h && (a.h - b.h).abs() < 0.6 * a.h.max(b.h)
            };
            if linked {
                let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                parent[ri] = rj;
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for i in 0..n {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    groups.into_values().collect()
}

fn bounds(rects: &[R], idx: &[usize]) -> R {
    idx.iter().fold(R::default(), |acc, &i| acc.union(rects[i]))
}

/// Sort segments into reading order: blocks top to bottom (left to right when side by side),
/// inside a block rows top to bottom and segments left to right.
pub fn order(lines: &mut Vec<OLine>) {
    let rects: Vec<R> = lines.iter().map(|l| l.rect).collect();
    let mut groups = blocks(&rects);
    let cmp = |a: &f32, b: &f32| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal);
    groups.sort_by(|a, b| {
        let (ra, rb) = (bounds(&rects, a), bounds(&rects, b));
        // side by side (vertical overlap): left first; otherwise top first
        let vo = ra.bottom().min(rb.bottom()) - ra.y.max(rb.y);
        if vo > 0.0 && (ra.x - rb.x).abs() > 1.0 {
            cmp(&ra.x, &rb.x)
        } else {
            cmp(&ra.y, &rb.y)
        }
    });
    let mut taken: Vec<Option<OLine>> = lines.drain(..).map(Some).collect();
    for g in groups {
        let mut rows: Vec<Vec<OLine>> = Vec::new();
        let mut items: Vec<OLine> = g.iter().map(|&i| taken[i].take().unwrap()).collect();
        items.sort_by(|a, b| cmp(&a.rect.y, &b.rect.y));
        for l in items {
            match rows
                .iter_mut()
                .rev()
                .take(3)
                .find(|row| same_row(&row[0].rect, &l.rect))
            {
                Some(row) => row.push(l),
                None => rows.push(vec![l]),
            }
        }
        rows.sort_by(|a, b| cmp(&vcenter(&a[0].rect), &vcenter(&b[0].rect)));
        for mut row in rows {
            row.sort_by(|a, b| cmp(&a.rect.x, &b.rect.x));
            lines.extend(row);
        }
    }
}

/// Caret position nearest to `q` (image pixels).
pub fn hit(lines: &[OLine], q: P) -> Option<Pos> {
    let mut best: Option<(f32, usize)> = None;
    for (i, l) in lines.iter().enumerate() {
        let r = l.rect;
        let dy = if q.y < r.y {
            r.y - q.y
        } else if q.y > r.bottom() {
            q.y - r.bottom()
        } else {
            0.0
        };
        let dx = if q.x < r.x {
            r.x - q.x
        } else if q.x > r.right() {
            q.x - r.right()
        } else {
            0.0
        };
        // rows matter more than columns: a point beside a line belongs to it
        let d = dy * 4.0 + dx;
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, i));
        }
    }
    let (_, i) = best?;
    let l = &lines[i];
    let ch = l
        .chars
        .iter()
        .position(|c| q.x < (c.x0 + c.x1) / 2.0)
        .unwrap_or(l.chars.len());
    Some(Pos { line: i, ch })
}

pub fn word_at(lines: &[OLine], p: Pos) -> (Pos, Pos) {
    let Some(l) = lines.get(p.line) else { return (p, p) };
    let ch = p.ch.min(l.chars.len().saturating_sub(1));
    let Some(at) = l.chars.get(ch) else { return (p, p) };
    if at.c.is_whitespace() {
        return (p, p);
    }
    // like text editors: letters, digits and _ make a word; a punctuation run is its own word
    let wordy = |c: char| c.is_alphanumeric() || c == '_';
    let kind = wordy(at.c);
    let word = |c: &OChar| !c.c.is_whitespace() && wordy(c.c) == kind;
    let mut s = ch;
    while s > 0 && word(&l.chars[s - 1]) {
        s -= 1;
    }
    let mut e = ch;
    while e < l.chars.len() && word(&l.chars[e]) {
        e += 1;
    }
    (Pos { line: p.line, ch: s }, Pos { line: p.line, ch: e })
}

pub fn all(lines: &[OLine]) -> (Pos, Pos) {
    let last = lines.len().saturating_sub(1);
    (
        Pos::default(),
        Pos {
            line: last,
            ch: lines.last().map_or(0, |l| l.chars.len()),
        },
    )
}

/// Highlight rectangles of a selection.
pub fn selection_rects(lines: &[OLine], a: Pos, b: Pos) -> Vec<R> {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    let mut out = Vec::new();
    for i in a.line..=b.line.min(lines.len().saturating_sub(1)) {
        let l = &lines[i];
        let s = if i == a.line { a.ch } else { 0 };
        let e = if i == b.line { b.ch } else { l.chars.len() };
        if s >= e || l.chars.is_empty() {
            continue;
        }
        let (x0, x1) = (
            l.chars[s.min(l.chars.len() - 1)].x0,
            l.chars[(e - 1).min(l.chars.len() - 1)].x1,
        );
        out.push(R::ltrb(x0, l.rect.y, x1, l.rect.bottom()));
    }
    out
}

// --- formatting -------------------------------------------------------------------

struct Seg {
    chars: Vec<OChar>,
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
}

struct Row {
    segs: Vec<Seg>,
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
}

impl Row {
    fn h(&self) -> f32 {
        self.y1 - self.y0
    }
}

fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[v.len() / 2]
}

/// The selected part of each segment, in reading order.
fn segs_of(lines: &[OLine], a: Pos, b: Pos) -> Vec<Seg> {
    let mut out = Vec::new();
    for i in a.line..=b.line.min(lines.len().saturating_sub(1)) {
        let l = &lines[i];
        let s = if i == a.line { a.ch } else { 0 };
        let e = if i == b.line {
            b.ch.min(l.chars.len())
        } else {
            l.chars.len()
        };
        if s >= e {
            continue;
        }
        let mut chars: Vec<OChar> = l.chars[s..e].to_vec();
        while chars.first().is_some_and(|c| c.c.is_whitespace()) {
            chars.remove(0);
        }
        while chars.last().is_some_and(|c| c.c.is_whitespace()) {
            chars.pop();
        }
        if chars.is_empty() {
            continue;
        }
        // whole segments keep their detected box: indentation is measured from it
        let (x0, x1) = if s == 0 && e == l.chars.len() {
            (l.rect.x.max(chars[0].x0 - (chars[0].x1 - chars[0].x0)), l.rect.right())
        } else {
            (chars[0].x0, chars.last().unwrap().x1)
        };
        out.push(Seg {
            x0,
            x1,
            y0: l.rect.y,
            y1: l.rect.bottom(),
            chars,
        });
    }
    out
}

/// Rows of one block (segments already in reading order).
fn rows_of(segs: Vec<Seg>) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for seg in segs {
        let r = R::ltrb(seg.x0, seg.y0, seg.x1, seg.y1);
        match rows.last_mut() {
            Some(row) if same_row(&R::ltrb(row.x0, row.y0, row.x1, row.y1), &r) => {
                row.x0 = row.x0.min(seg.x0);
                row.x1 = row.x1.max(seg.x1);
                row.y0 = row.y0.min(seg.y0);
                row.y1 = row.y1.max(seg.y1);
                row.segs.push(seg);
            }
            _ => rows.push(Row {
                x0: seg.x0,
                x1: seg.x1,
                y0: seg.y0,
                y1: seg.y1,
                segs: vec![seg],
            }),
        }
    }
    rows
}

fn seg_text(s: &Seg) -> String {
    s.chars.iter().map(|c| c.c).collect()
}

/// Average character pitch of a run (monospace assumption).
fn pitch(s: &Seg) -> f32 {
    let n = s.chars.len().max(1) as f32;
    (s.chars.last().map_or(s.x1, |c| c.x1) - s.chars.first().map_or(s.x0, |c| c.x0)) / n
}

const CODE_SYMBOLS: &str = "{}()[];=<>_/\\|&*#$:\"'`!~^%+@";

fn is_code(rows: &[Row]) -> bool {
    let text: String = rows
        .iter()
        .flat_map(|r| r.segs.iter().map(seg_text))
        .collect::<Vec<_>>()
        .join(" ");
    let visible = text.chars().filter(|c| !c.is_whitespace()).count().max(1) as f32;
    let symbols = text.chars().filter(|c| CODE_SYMBOLS.contains(*c)).count() as f32 / visible;
    let ends = rows
        .iter()
        .filter(|r| {
            let t = r.segs.last().map(seg_text).unwrap_or_default();
            t.ends_with(['{', '}', ';', '(', ')', '[', ']', ',', ':', '\\']) || t.trim().len() <= 2
        })
        .count() as f32
        / rows.len().max(1) as f32;
    let p = median(rows.iter().flat_map(|r| r.segs.iter().map(pitch)).collect()).max(1.0);
    let base = rows.iter().map(|r| r.x0).fold(f32::MAX, f32::min);
    let mut levels: Vec<i32> = rows.iter().map(|r| ((r.x0 - base) / p).round() as i32).collect();
    levels.sort();
    levels.dedup();
    let indented = levels.len() >= 2;
    if rows.len() == 1 {
        return symbols >= 0.12;
    }
    (symbols >= 0.05 && (indented || ends >= 0.3)) || ends >= 0.6 || (indented && symbols >= 0.03)
}

fn format_code(rows: &[Row]) -> String {
    let p = median(rows.iter().flat_map(|r| r.segs.iter().map(pitch)).collect()).max(1.0);
    let base = rows.iter().map(|r| r.x0).fold(f32::MAX, f32::min);
    let mut out = Vec::new();
    for row in rows {
        let mut line = " ".repeat(((row.x0 - base) / p).round().max(0.0) as usize);
        let mut prev_end: Option<f32> = None;
        for s in &row.segs {
            if let Some(pe) = prev_end {
                line.push_str(&" ".repeat(((s.x0 - pe) / p).round().max(1.0) as usize));
            }
            line.push_str(&seg_text(s));
            prev_end = Some(s.x1);
        }
        out.push(line.trim_end().to_string());
    }
    out.join("\n")
}

fn starts_item(t: &str) -> bool {
    let t = t.trim_start();
    if t.starts_with(['•', '◦', '▪', '·', '–', '—', '*', '-', '>']) {
        return true;
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    digits > 0 && digits < 4 && t[digits..].starts_with(['.', ')'])
}

fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        // OCR puts spaces before punctuation; and collapse runs of spaces
        if matches!(c, ',' | '.' | ';' | ':' | '!' | '?' | ')' | '»' | '%') && out.ends_with(' ') {
            out.pop();
        }
        if c == ' ' && (out.ends_with(' ') || out.ends_with('(') || out.ends_with('«')) {
            continue;
        }
        out.push(c);
    }
    out
}

fn format_prose(rows: &[Row]) -> String {
    let right = rows.iter().map(|r| r.x1).fold(f32::MIN, f32::max);
    let left = rows.iter().map(|r| r.x0).fold(f32::MAX, f32::min);
    let mut paras: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<&Row> = None;
    for row in rows {
        let text = row.segs.iter().map(seg_text).collect::<Vec<_>>().join(" ");
        let h = row.h();
        let new_para = match prev {
            None => true,
            Some(p) => {
                let short = p.x1 < right - 2.5 * p.h();
                let indent = (row.x0 - left) > 1.5 * h && (p.x0 - left) < 0.5 * h;
                // narrow blocks are lists/menus unless a line visibly continues the previous one
                let narrow = right - left < 15.0 * h;
                let continues =
                    text.chars().next().is_some_and(char::is_lowercase) || cur.ends_with([',', '-', '‐', '¬']);
                short || indent || starts_item(&text) || (narrow && !continues)
            }
        };
        if new_para {
            if !cur.is_empty() {
                paras.push(tidy(&cur));
            }
            cur = text;
        } else {
            // a word split by a line-end hyphen is glued back together
            let mut tail = cur.chars().rev();
            let hyphen = matches!(tail.next(), Some('-' | '‐' | '¬')) && tail.next().is_some_and(char::is_alphabetic);
            if hyphen && text.chars().next().is_some_and(char::is_lowercase) {
                cur.pop();
                cur.push_str(&text);
            } else {
                cur.push(' ');
                cur.push_str(&text);
            }
        }
        prev = Some(row);
    }
    if !cur.is_empty() {
        paras.push(tidy(&cur));
    }
    paras.join("\n")
}

/// Text of the range [a, b) as it should be pasted.
pub fn format(lines: &[OLine], a: Pos, b: Pos) -> String {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    let segs = segs_of(lines, a, b);
    if segs.is_empty() {
        return String::new();
    }
    // blocks (columns, panels, paragraphs) first, rows only inside a block
    let rects: Vec<R> = segs.iter().map(|g| R::ltrb(g.x0, g.y0, g.x1, g.y1)).collect();
    let mut groups = blocks(&rects);
    groups.sort_by_key(|g| g.iter().copied().min().unwrap_or(0)); // segments are in reading order
    let mut slots: Vec<Option<Seg>> = segs.into_iter().map(Some).collect();
    let mut blocks_rows: Vec<Vec<Row>> = Vec::new();
    for mut g in groups {
        g.sort();
        let rows = rows_of(g.into_iter().map(|i| slots[i].take().unwrap()).collect());
        // a blank line inside one cluster still separates paragraphs
        let h = median(rows.iter().map(Row::h).collect()).max(1.0);
        let mut cur: Vec<Row> = Vec::new();
        for row in rows {
            if cur.last().is_some_and(|p| row.y0 - p.y1 > h * 0.9) {
                blocks_rows.push(std::mem::take(&mut cur));
            }
            cur.push(row);
        }
        if !cur.is_empty() {
            blocks_rows.push(cur);
        }
    }
    let blocks = blocks_rows;
    let mut out = String::new();
    let mut prev: Option<(R, usize)> = None; // last block's box and row count
    for b in &blocks {
        let text = if is_code(b) { format_code(b) } else { format_prose(b) };
        let rect = b
            .iter()
            .fold(R::default(), |acc, r| acc.union(R::ltrb(r.x0, r.y0, r.x1, r.y1)));
        if let Some((pr, pn)) = prev {
            out.push_str(if pn == 1 && b.len() == 1 && same_row(&pr, &rect) {
                "\t" // side by side on one line: a table row, a label and its value
            } else if pn > 1 || b.len() > 1 {
                "\n\n"
            } else {
                "\n"
            });
        }
        out.push_str(&text);
        prev = Some((rect, b.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::r;

    /// A segment of fixed-pitch characters at (x, y).
    fn seg(text: &str, x: f32, y: f32, pitch: f32) -> OLine {
        let chars = text
            .chars()
            .enumerate()
            .map(|(i, c)| OChar {
                c,
                x0: x + i as f32 * pitch,
                x1: x + (i + 1) as f32 * pitch,
            })
            .collect();
        OLine {
            rect: r(x, y, text.chars().count() as f32 * pitch, 16.0),
            chars,
        }
    }

    fn fmt(lines: &mut Vec<OLine>) -> String {
        order(lines);
        let (a, b) = all(lines);
        format(lines, a, b)
    }

    #[test]
    fn prose_joins_lines_and_hyphens() {
        let mut l = vec![
            seg("Это первая строка длинного абзаца, кото-", 10.0, 10.0, 8.0),
            seg("рая продолжается здесь и тоже длинная.", 10.0, 32.0, 8.0),
            seg("Короткий конец.", 10.0, 54.0, 8.0),
            seg("Второй абзац после пустой строки.", 10.0, 100.0, 8.0),
        ];
        assert_eq!(
            fmt(&mut l),
            "Это первая строка длинного абзаца, которая продолжается здесь и тоже длинная. Короткий конец.\n\nВторой абзац после пустой строки."
        );
    }

    #[test]
    fn short_lines_stay_separate() {
        let mut l = vec![
            seg("General", 10.0, 10.0, 8.0),
            seg("Network settings and more", 10.0, 32.0, 8.0),
            seg("Display", 10.0, 54.0, 8.0),
        ];
        assert_eq!(fmt(&mut l), "General\nNetwork settings and more\nDisplay");
    }

    #[test]
    fn code_keeps_indentation() {
        let p = 8.0;
        let mut l = vec![
            seg("fn main() {", 20.0, 10.0, p),
            seg("let x = vec![1, 2];", 20.0 + 4.0 * p, 30.0, p),
            seg("for v in &x {", 20.0 + 4.0 * p, 50.0, p),
            seg("println!(\"{v}\");", 20.0 + 8.0 * p, 70.0, p),
            seg("}", 20.0 + 4.0 * p, 90.0, p),
            seg("}", 20.0, 110.0, p),
        ];
        assert_eq!(
            fmt(&mut l),
            "fn main() {\n    let x = vec![1, 2];\n    for v in &x {\n        println!(\"{v}\");\n    }\n}"
        );
    }

    #[test]
    fn partial_selection_and_words() {
        let mut l = vec![seg("hello brave world", 0.0, 0.0, 10.0)];
        order(&mut l);
        let p = hit(&l, P { x: 72.0, y: 8.0 }).unwrap();
        assert_eq!(p, Pos { line: 0, ch: 7 });
        let (a, b) = word_at(&l, p);
        assert_eq!(format(&l, a, b), "brave");
        assert_eq!(
            format(&l, Pos { line: 0, ch: 6 }, Pos { line: 0, ch: 17 }),
            "brave world"
        );
    }
}
