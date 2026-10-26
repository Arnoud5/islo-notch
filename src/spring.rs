//! Damped spring, integrated per frame.
//!
//! Unlike a fixed-duration tween, a spring keeps its velocity when the target
//! changes mid-flight, so hovering in and out quickly feels continuous and
//! "physical" instead of restarting. `ratio` < 1 overshoots a little (playful),
//! `ratio` = 1 settles as fast as possible without overshoot.

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
    stiffness: f32,
    damping: f32,
}

impl Spring {
    pub fn new(value: f32) -> Self {
        let mut s = Spring { value, velocity: 0.0, target: value, stiffness: 0.0, damping: 0.0 };
        s.configure(380.0, 0.75);
        s
    }

    /// `stiffness`: how hard it pulls (higher = faster). `ratio`: damping ratio.
    pub fn configure(&mut self, stiffness: f32, ratio: f32) {
        self.stiffness = stiffness;
        // critical damping for unit mass is 2*sqrt(k)
        self.damping = 2.0 * ratio * stiffness.sqrt();
    }

    pub fn snap(&mut self, v: f32) {
        self.value = v;
        self.target = v;
        self.velocity = 0.0;
    }

    pub fn at_rest(&self) -> bool {
        (self.value - self.target).abs() < 0.05 && self.velocity.abs() < 0.5
    }

    /// Advance by `dt` seconds. Returns true while still moving.
    pub fn step(&mut self, dt: f32) -> bool {
        if self.at_rest() {
            self.value = self.target;
            self.velocity = 0.0;
            return false;
        }
        // Fixed sub-steps keep it stable on 30 Hz and 240 Hz displays alike.
        let mut left = dt.min(0.064);
        const H: f32 = 1.0 / 480.0;
        while left > 0.0 {
            let h = left.min(H);
            let force = -self.stiffness * (self.value - self.target) - self.damping * self.velocity;
            self.velocity += force * h;
            self.value += self.velocity * h;
            left -= h;
        }
        true
    }
}
