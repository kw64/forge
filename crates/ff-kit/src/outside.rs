//! Fermeture d'une fenêtre de réglages par un clic en dehors d'elle. v0.1.0
use egui;

/// À appeler à chaque image pour une fenêtre : renvoie `true` quand l'utilisateur clique sur le fond
/// (hors de toute fenêtre ou menu déroulant). Le clic qui a ouvert la fenêtre est ignoré.
#[derive(Default)]
pub struct OutsideClose {
    frames: u32,
    last: u64,
}

impl OutsideClose {
    pub fn check(&mut self, ctx: &egui::Context, open: bool) -> bool {
        if !open {
            self.frames = 0;
            return false;
        }
        // Image non consécutive (fenêtre restée fermée entre-temps) : on repart de zéro.
        let nr = ctx.cumulative_frame_nr();
        if nr > self.last + 1 {
            self.frames = 0;
        }
        self.last = nr;
        self.frames = self.frames.saturating_add(1);
        if self.frames <= 2 || !ctx.input(|i| i.pointer.primary_clicked()) {
            return false;
        }
        match ctx.input(|i| i.pointer.interact_pos()) {
            Some(p) => ctx.layer_id_at(p).map(|l| l.order == egui::Order::Background).unwrap_or(true),
            None => false,
        }
    }
}
