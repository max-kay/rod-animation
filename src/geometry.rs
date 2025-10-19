use bincode::{Decode, Encode};

use crate::vec::Vector;

#[derive(Encode, Decode, Debug, Clone)]
pub struct Path(pub Vec<Vector>);

impl Path {
    fn get_signed_area_sum(&self) -> f32 {
        let n = self.0.len();
        if n < 3 {
            return 0.0;
        }

        let mut signed_area_sum = 0.0;

        for i in 0..n {
            let p_i = &self.0[i];
            let p_next = &self.0[(i + 1) % n];
            signed_area_sum += (p_i.x * p_next.y) - (p_next.x * p_i.y);
        }
        signed_area_sum
    }

    fn reverse(&mut self) {
        self.0.reverse();
    }
}

#[derive(Encode, Decode, Default, Debug, Clone)]
pub struct Area {
    pub outer: Vec<Path>,
    pub inner: Vec<Path>,
}

impl Area {
    pub fn enforce_winding(&mut self) {
        for path in &mut self.outer {
            if path.get_signed_area_sum() < 0.0 {
                path.reverse();
            }
        }
        for path in &mut self.inner {
            if path.get_signed_area_sum() > 0.0 {
                path.reverse();
            }
        }
    }
}
