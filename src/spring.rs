//! Tiny damped-spring animator. Constants were fitted to frame captures of a
//! reference notch (ω≈12.6 rad/s, ζ≈0.7 → ~5% overshoot, ~350 ms to peak).

#[derive(Clone, Copy)]
pub struct Spring {
    pub value: f32,
    pub target: f32,
    vel: f32,
    omega: f32,
    zeta: f32,
}

impl Spring {
    pub fn new(v: f32) -> Self {
        Self { value: v, target: v, vel: 0.0, omega: 12.6, zeta: 0.70 }
    }
    pub fn with(v: f32, omega: f32, zeta: f32) -> Self {
        Self { value: v, target: v, vel: 0.0, omega, zeta }
    }
    /// Change stiffness / damping on the fly (the island morphs with different springs in and out).
    pub fn retune(&mut self, omega: f32, zeta: f32) {
        self.omega = omega;
        self.zeta = zeta;
    }
    pub fn set(&mut self, t: f32) {
        self.target = t;
    }
    pub fn snap(&mut self, v: f32) {
        self.value = v;
        self.target = v;
        self.vel = 0.0;
    }
    pub fn settled(&self) -> bool {
        (self.value - self.target).abs() < 0.05 && self.vel.abs() < 0.2
    }
    /// Semi-implicit Euler, sub-stepped so large frame gaps stay stable.
    pub fn step(&mut self, dt: f32) {
        let mut left = dt.min(0.1);
        while left > 0.0 {
            let h = left.min(1.0 / 240.0);
            let acc = -self.omega * self.omega * (self.value - self.target) - 2.0 * self.zeta * self.omega * self.vel;
            self.vel += acc * h;
            self.value += self.vel * h;
            left -= h;
        }
        if self.settled() {
            self.value = self.target;
            self.vel = 0.0;
        }
    }
}

/// Exponential smoothing toward a target (for opacity/fades).
#[derive(Clone, Copy)]
pub struct Ease {
    pub value: f32,
    pub target: f32,
    pub rate: f32,
}
impl Ease {
    pub fn new(v: f32, rate: f32) -> Self {
        Self { value: v, target: v, rate }
    }
    pub fn step(&mut self, dt: f32) {
        let k = 1.0 - (-self.rate * dt).exp();
        self.value += (self.target - self.value) * k;
        if (self.value - self.target).abs() < 0.002 {
            self.value = self.target;
        }
    }
    pub fn settled(&self) -> bool {
        self.value == self.target
    }
}

pub fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
