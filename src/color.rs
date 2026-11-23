//! "#aarrggbb" / "#rrggbb" colors as stored in configs and documents (Qt's HexArgb names).

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    pub fn parse(s: &str) -> Option<Color> {
        let h = s.trim().strip_prefix('#')?;
        let n = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
        match h.len() {
            8 => Some(Color {
                a: n(0)?,
                r: n(2)?,
                g: n(4)?,
                b: n(6)?,
            }),
            6 => Some(Color {
                a: 255,
                r: n(0)?,
                g: n(2)?,
                b: n(4)?,
            }),
            3 => {
                let d = |i: usize| u8::from_str_radix(h.get(i..i + 1)?, 16).ok().map(|v| v * 17);
                Some(Color {
                    a: 255,
                    r: d(0)?,
                    g: d(1)?,
                    b: d(2)?,
                })
            }
            _ => None,
        }
    }

    /// "#aarrggbb" (what documents store)
    pub fn argb(&self) -> String {
        format!("#{:02x}{:02x}{:02x}{:02x}", self.a, self.r, self.g, self.b)
    }

    /// "#rrggbb"
    pub fn rgb_name(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    pub fn with_alpha(self, a: u8) -> Color {
        Color { a, ..self }
    }

    pub fn alpha_f(self, f: f32) -> Color {
        Color {
            a: (f.clamp(0.0, 1.0) * 255.0).round() as u8,
            ..self
        }
    }

    pub fn skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.r, self.g, self.b, self.a)
    }

    pub fn egui(self) -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied(self.r, self.g, self.b, self.a)
    }

    pub fn to_hsv(self) -> (f32, f32, f32, f32) {
        let (r, g, b) = (self.r as f32 / 255.0, self.g as f32 / 255.0, self.b as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        let h = if d == 0.0 {
            -1.0
        } else if max == r {
            ((g - b) / d).rem_euclid(6.0) / 6.0
        } else if max == g {
            ((b - r) / d + 2.0) / 6.0
        } else {
            ((r - g) / d + 4.0) / 6.0
        };
        let s = if max == 0.0 { 0.0 } else { d / max };
        (h, s, max, self.a as f32 / 255.0)
    }

    pub fn from_hsv(h: f32, s: f32, v: f32, a: f32) -> Color {
        let h = (h.rem_euclid(1.0)) * 6.0;
        let c = v * s;
        let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
        let (r, g, b) = match h as i32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        let q = |f: f32| ((f + m).clamp(0.0, 1.0) * 255.0).round() as u8;
        Color {
            r: q(r),
            g: q(g),
            b: q(b),
            a: (a.clamp(0.0, 1.0) * 255.0).round() as u8,
        }
    }
}
