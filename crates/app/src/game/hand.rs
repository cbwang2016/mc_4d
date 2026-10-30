//! Beta first-person equip/swing presentation, independent of mining progress.
use super::{Game, Slot};
#[derive(Default)]
pub(super) struct HandState {
    pub displayed: Option<Slot>,
    pub equip: f32,
    swing: Option<f32>,
}
impl HandState {
    pub fn update(&mut self, dt: f32, selected: Option<Slot>) {
        if let Some(time) = &mut self.swing {
            *time += dt;
            if *time >= 0.3 {
                self.swing = None;
            }
        }
        let same = self.displayed.map(|s| s.block) == selected.map(|s| s.block);
        if same {
            self.displayed = selected;
        }
        let target = if same { 1.0 } else { 0.0 };
        self.equip += (target - self.equip).clamp(-8.0 * dt, 8.0 * dt);
        if self.equip < 0.1 {
            self.displayed = selected;
        }
    }
    pub fn progress(&self) -> f32 {
        self.swing.map_or(0.0, |t| (t / 0.3).clamp(0.0, 1.0))
    }
}
impl Game {
    pub fn start_hand_swing(&mut self) {
        if self.hand.swing.is_none_or(|t| t >= 0.15) {
            self.hand.swing = Some(0.0);
        }
    }
    pub fn start_hand_use(&mut self) {
        self.hand.equip = 0.0;
    }
    pub fn first_person_hand(&self) -> (Option<Slot>, f32, f32) {
        (self.hand.displayed, self.hand.equip, self.hand.progress())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use mc4d_core::{block, item};
    #[test]
    fn switching_items_lowers_old_geometry_before_replacing_it() {
        let mut h = HandState::default();
        let dirt = Some(Slot::new(block::DIRT, 1));
        let sword = Some(Slot::new(item::WOOD_SWORD, 1));
        for _ in 0..20 {
            h.update(0.01, dirt);
        }
        assert_eq!(h.displayed, dirt);
        assert!(h.equip > 0.99);
        h.update(0.01, sword);
        assert_eq!(h.displayed, dirt);
        assert!(h.equip > 0.8);
        for _ in 0..30 {
            h.update(0.01, sword);
        }
        assert_eq!(h.displayed, sword);
        assert!(h.equip > 0.99);
        h.update(0.01, Some(Slot::new(item::WOOD_SWORD, 2)));
        assert!(
            h.equip > 0.99,
            "count changes must not repeatedly lower held items"
        );
    }
}
