//! Small float geometry used by the document (image pixel coordinates).
use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct P {
    pub x: f32,
    pub y: f32,
}

pub const fn p(x: f32, y: f32) -> P {
    P { x, y }
}

impl P {
    pub fn len(self) -> f32 {
        self.x.hypot(self.y)
    }
    pub fn dist(self, o: P) -> f32 {
        (self - o).len()
    }
    pub fn arr(self) -> [f32; 2] {
        [self.x, self.y]
    }
    pub fn from(a: [f32; 2]) -> P {
        P { x: a[0], y: a[1] }
    }
    pub fn egui(self) -> egui::Pos2 {
        egui::pos2(self.x, self.y)
    }
}

impl Add for P {
    type Output = P;
    fn add(self, o: P) -> P {
        p(self.x + o.x, self.y + o.y)
    }
}
impl Sub for P {
    type Output = P;
    fn sub(self, o: P) -> P {
        p(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f32> for P {
    type Output = P;
    fn mul(self, k: f32) -> P {
        p(self.x * k, self.y * k)
    }
}
impl Div<f32> for P {
    type Output = P;
    fn div(self, k: f32) -> P {
        p(self.x / k, self.y / k)
    }
}
impl Neg for P {
    type Output = P;
    fn neg(self) -> P {
        p(-self.x, -self.y)
    }
}

/// Axis-aligned rectangle; may be built un-normalized, `norm()` fixes negative sizes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub const fn r(x: f32, y: f32, w: f32, h: f32) -> R {
    R { x, y, w, h }
}

impl R {
    pub fn from(a: [f32; 4]) -> R {
        r(a[0], a[1], a[2], a[3])
    }
    pub fn arr(self) -> [f32; 4] {
        [self.x, self.y, self.w, self.h]
    }
    pub fn pts(a: P, b: P) -> R {
        r(a.x, a.y, b.x - a.x, b.y - a.y).norm()
    }
    pub fn ltrb(l: f32, t: f32, rr: f32, b: f32) -> R {
        r(l, t, rr - l, b - t)
    }
    pub fn norm(self) -> R {
        let (x, w) = if self.w < 0.0 {
            (self.x + self.w, -self.w)
        } else {
            (self.x, self.w)
        };
        let (y, h) = if self.h < 0.0 {
            (self.y + self.h, -self.h)
        } else {
            (self.y, self.h)
        };
        r(x, y, w, h)
    }
    pub fn left(self) -> f32 {
        self.x
    }
    pub fn top(self) -> f32 {
        self.y
    }
    pub fn right(self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(self) -> f32 {
        self.y + self.h
    }
    pub fn center(self) -> P {
        p(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    pub fn tl(self) -> P {
        p(self.x, self.y)
    }
    pub fn br(self) -> P {
        p(self.right(), self.bottom())
    }
    pub fn contains(self, q: P) -> bool {
        q.x >= self.x && q.x < self.right() && q.y >= self.y && q.y < self.bottom()
    }
    pub fn adjusted(self, l: f32, t: f32, rr: f32, b: f32) -> R {
        R::ltrb(self.x + l, self.y + t, self.right() + rr, self.bottom() + b)
    }
    pub fn grow(self, d: f32) -> R {
        self.adjusted(-d, -d, d, d)
    }
    pub fn translate(self, d: P) -> R {
        r(self.x + d.x, self.y + d.y, self.w, self.h)
    }
    pub fn union(self, o: R) -> R {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return self;
        }
        R::ltrb(
            self.x.min(o.x),
            self.y.min(o.y),
            self.right().max(o.right()),
            self.bottom().max(o.bottom()),
        )
    }
    pub fn intersect(self, o: R) -> R {
        let l = self.x.max(o.x);
        let t = self.y.max(o.y);
        let rr = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        if rr <= l || b <= t {
            R::default()
        } else {
            R::ltrb(l, t, rr, b)
        }
    }
    pub fn is_empty(self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
    /// Smallest integer rectangle containing this one (QRectF::toAlignedRect).
    pub fn aligned(self) -> IR {
        let l = self.x.floor() as i32;
        let t = self.y.floor() as i32;
        let rr = self.right().ceil() as i32;
        let b = self.bottom().ceil() as i32;
        IR {
            x: l,
            y: t,
            w: rr - l,
            h: b - t,
        }
    }
    pub fn of(e: egui::Rect) -> R {
        r(e.min.x, e.min.y, e.width(), e.height())
    }
}

/// Integer rectangle (crop, regions), image pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct IR {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl IR {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> IR {
        IR { x, y, w, h }
    }
    pub fn f(self) -> R {
        r(self.x as f32, self.y as f32, self.w as f32, self.h as f32)
    }
    pub fn right(self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(self) -> i32 {
        self.y + self.h
    }
    pub fn is_empty(self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    pub fn intersect(self, o: IR) -> IR {
        let l = self.x.max(o.x);
        let t = self.y.max(o.y);
        let rr = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        if rr <= l || b <= t {
            IR::default()
        } else {
            IR::new(l, t, rr - l, b - t)
        }
    }
    pub fn contains_rect(self, o: IR) -> bool {
        o.x >= self.x && o.y >= self.y && o.right() <= self.right() && o.bottom() <= self.bottom()
    }
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
    pub fn translated(self, dx: i32, dy: i32) -> IR {
        IR::new(self.x + dx, self.y + dy, self.w, self.h)
    }
    pub fn arr(self) -> [i32; 4] {
        [self.x, self.y, self.w, self.h]
    }
}
