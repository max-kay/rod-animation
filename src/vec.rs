use std::ops::{Add, Div, Mul, Neg};

use bincode::{Decode, Encode};
use geo_types::Coord;

macro_rules! impl_op_for_refs {
    ($t:ty, $trait:ident, $method:ident) => {
        impl_op_for_refs!($t, $t, $trait, $method);
    };

    ($tl:ty, $tr:ty, $trait:ident, $method:ident) => {
        impl std::ops::$trait<$tr> for &$tl {
            type Output = <$tl as std::ops::$trait<$tr>>::Output;

            fn $method(self, rhs: $tr) -> Self::Output {
                (*self).$method(rhs)
            }
        }

        impl std::ops::$trait<&$tr> for $tl {
            type Output = <$tl as std::ops::$trait<$tr>>::Output;

            fn $method(self, rhs: &$tr) -> Self::Output {
                self.$method(*rhs)
            }
        }

        impl std::ops::$trait<&$tr> for &$tl {
            type Output = <$tl as std::ops::$trait<$tr>>::Output;

            fn $method(self, rhs: &$tr) -> Self::Output {
                (*self).$method(*rhs)
            }
        }
    };
}

macro_rules! complete_group {
    ($t:ty) => {
        impl std::ops::Neg for &$t {
            type Output = $t;
            fn neg(self) -> Self::Output {
                -*self
            }
        }
        impl std::ops::Sub for $t {
            type Output = $t;

            fn sub(self, rhs: $t) -> Self::Output {
                self + (-rhs)
            }
        }

        impl_op_for_refs!($t, Add, add);
        impl_op_for_refs!($t, Sub, sub);
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Decode, Encode)]
pub struct Vector {
    pub x: f32,
    pub y: f32,
}

impl From<Coord<f32>> for Vector {
    fn from(value: Coord<f32>) -> Self {
        Vector {
            x: value.x,
            y: value.y,
        }
    }
}

impl From<&Coord<f32>> for Vector {
    fn from(value: &Coord<f32>) -> Self {
        Vector {
            x: value.x,
            y: value.y,
        }
    }
}
impl Vector {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn norm(&self) -> f32 {
        (self.x.powi(2) + self.y.powi(2)).sqrt()
    }
    pub fn zeros() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}
impl Add for Vector {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Vector {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}
impl Neg for Vector {
    type Output = Self;

    fn neg(self) -> Self::Output {
        let Self { x, y } = self;
        Self { x: -x, y: -y }
    }
}
complete_group!(Vector);

impl Mul<f32> for Vector {
    type Output = Vector;

    fn mul(mut self, rhs: f32) -> Self::Output {
        self.x *= rhs;
        self.y *= rhs;
        self
    }
}
impl_op_for_refs!(Vector, f32, Mul, mul);
impl_op_for_refs!(f32, Vector, Mul, mul);
impl Mul<Vector> for f32 {
    type Output = Vector;

    fn mul(self, mut vec: Vector) -> Self::Output {
        vec.x *= self;
        vec.y *= self;
        vec
    }
}

impl Div<f32> for Vector {
    type Output = Vector;

    fn div(mut self, rhs: f32) -> Self::Output {
        self.x /= rhs;
        self.y /= rhs;
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Transform {
    scale: f32,
    translation: Vector,
}

impl Transform {
    pub fn new(scale: f32, translation: Vector) -> Self {
        Self { scale, translation }
    }

    pub fn identity() -> Self {
        Self {
            scale: 1.0,
            translation: Vector::zeros(),
        }
    }

    pub fn invert(&self) -> Transform {
        Self {
            scale: 1.0 / self.scale,
            translation: -self.translation / self.scale,
        }
    }
}

impl Mul<Vector> for Transform {
    type Output = Vector;
    fn mul(self, rhs: Vector) -> Self::Output {
        self.scale * rhs + self.translation
    }
}

impl_op_for_refs!(Transform, Vector, Mul, mul);

impl Mul for Transform {
    type Output = Transform;

    fn mul(self, rhs: Self) -> Self::Output {
        Transform {
            scale: self.scale * rhs.scale,
            translation: self.scale * rhs.translation + self.translation,
        }
    }
}

impl_op_for_refs!(Transform, Mul, mul);
