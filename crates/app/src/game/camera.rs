//! Beta's rear camera uses a four-block boom and corner rays to avoid clipping.
//! Sixteen corners extend the original eight-corner clearance test through W.
use super::Game;
use mc4d_core::{
    block::{self, BlockState},
    glam::{IVec4, Vec4},
    raycast,
};
fn rear_eye(eye: Vec4, forward: Vec4, get: &impl Fn(IVec4) -> Option<BlockState>) -> Vec4 {
    let direction = -forward.normalize_or_zero();
    let mut distance = 4.0_f32;
    for mask in 0..16 {
        let offset = Vec4::new(
            if mask & 1 == 0 { -0.1 } else { 0.1 },
            if mask & 2 == 0 { -0.1 } else { 0.1 },
            if mask & 4 == 0 { -0.1 } else { 0.1 },
            if mask & 8 == 0 { -0.1 } else { 0.1 },
        );
        if let Some(hit) = raycast::raycast_states(
            eye + offset,
            direction,
            4.0,
            &|p| Some(get(p).unwrap_or_else(|| block::BEDROCK.into())),
            false,
        ) {
            distance = distance.min((hit.dist - 0.05).max(0.0));
        }
    }
    eye + direction * distance
}
impl Game {
    /// Promote the actual displayed side frame. This is a signed quarter-turn:
    /// forward/pitch stay fixed and four successive presses restore the frame.
    pub fn switch_to_side_view(&mut self) {
        let side=mc4d_core::views::slice_bases(self.basis(),2)[1];
        self.orient.set_heading_basis(side);
        self.update_target();
    }

    pub fn camera_eye(&self) -> Vec4 {
        if self.third_person {
            rear_eye(self.eye(), self.basis().forward, &|p| {
                self.world.block_state(p)
            })
        } else {
            self.eye()
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rear_camera_clears_walls_and_unknown_space_in_xyz_and_w() {
        let eye = Vec4::splat(0.5);
        for forward in [Vec4::X, Vec4::Z, Vec4::W, (Vec4::X + Vec4::W).normalize()] {
            let clear = rear_eye(eye, forward, &|_| Some(block::AIR.into()));
            assert!((clear.distance(eye) - 4.0).abs() < 1e-5);
            let blocked = rear_eye(eye, forward, &|p| {
                Some(
                    if p.as_vec4().dot(-forward) > 1.0 {
                        block::STONE
                    } else {
                        block::AIR
                    }
                    .into(),
                )
            });
            assert!(blocked.distance(eye) < 3.0);
            assert_eq!(rear_eye(eye, forward, &|_| None), eye);
        }
    }
}

#[cfg(test)]mod side_view_tests {
    use super::*;
    use crate::{game::{GameMode,Target},input::{Action,Binding,Bindings,InputState},settings::Settings};
    use mc4d_core::{camera::Basis,chunk::{Chunk,chunk_of},glam::Vec4};
    fn same(a:Basis,b:Basis){for(x,y)in[(a.right,b.right),(a.up,b.up),(a.forward,b.forward),(a.ana,b.ana),(a.forward_h,b.forward_h)]{assert!((x-y).length()<1e-5);}}
    #[test]fn side_switch_uses_displayed_frame_and_four_presses_restore_orientation(){
        let mut g=Game::menu_preview(7);g.orient.yaw(0.71);g.orient.rotate_forward_ana(0.43);g.orient.rotate_right_ana(-0.26);g.orient.add_pitch(1.2);
        let before=g.basis();let side=mc4d_core::views::slice_bases(before,2)[1];let pos=g.body.pos;let pitch=g.orient.pitch;
        g.orient.flip_ana();g.switch_to_side_view();same(g.basis(),side);assert_eq!(g.body.pos,pos);assert_eq!(g.orient.pitch,pitch);assert!(!g.orient.is_animating());
        g.orient.update(0.5,false);same(g.basis(),side);
        g.switch_to_side_view();assert!((g.basis().right+before.right).length()<1e-5);assert!((g.basis().ana+before.ana).length()<1e-5);assert!((g.basis().forward-before.forward).length()<1e-5);
        g.switch_to_side_view();g.switch_to_side_view();same(g.basis(),before);
    }
    #[test]fn configured_shortcut_changes_strafe_and_subsequent_aiming_basis(){
        let mut g=Game::menu_preview(7);g.mode=GameMode::Creative;g.flying=true;g.body.pos=Vec4::new(8.5,65.0,8.5,8.5);g.body.vel=Vec4::ZERO;
        g.world.insert_chunk(chunk_of(g.body.pos.floor().as_ivec4()),Chunk::uniform(block::AIR));
        let bindings=Bindings::default();let mut input=InputState::default();input.press(bindings.get(Action::SwitchToSideView)[0]);g.handle_actions(&input,&bindings,&Settings::default());
        assert!((g.basis().right-Vec4::W).length()<1e-6);
        let before=g.body.pos;let mut right=InputState::default();right.press(Binding::Key(winit::keyboard::KeyCode::KeyD));g.move_player(0.05,&right,&bindings);
        let delta=g.body.pos-before;assert!(delta.w>0.1);assert!(delta.x.abs()<1e-5&&delta.z.abs()<1e-5);
        g.body.pos=before;g.body.vel=Vec4::ZERO;g.orient.yaw(std::f32::consts::FRAC_PI_2);
        let target=g.eye().floor().as_ivec4()+IVec4::W*3;g.world.set_block(target,block::STONE);g.update_target();
        assert!(matches!(g.target,Some(Target::Block(hit))if hit.block==target));
    }
}
