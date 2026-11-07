//! Traductions du lecteur (français = clé). v0.4.0
use ff_core::Lang;
use std::collections::HashMap;
use std::sync::OnceLock;

const EN: &[(&str, &str)] = &[
    ("Thème (partagé avec MediaForge)", "Theme (shared with MediaForge)"),
    ("Page précédente", "Previous page"),
    ("Page suivante", "Next page"),
    ("Glisser : tourner · Molette : zoom · W : fil de fer", "Drag: rotate · Wheel: zoom · W: wireframe"),
    ("triangles", "triangles"),
    ("Clair", "Light"),
    ("Sombre", "Dark"),
    ("Ouvrir…", "Open…"),
    ("Ouvrir des fichiers", "Open files"),
    ("Ajouter à la liste…", "Add to playlist…"),
    ("Médias", "Media"),
    ("Liste de lecture", "Playlist"),
    ("Aléatoire", "Shuffle"),
    ("Répéter : non", "Repeat: off"),
    ("Répéter : tout", "Repeat: all"),
    ("Répéter : un", "Repeat: one"),
    ("Vider la liste", "Clear list"),
    ("La liste est vide. Glissez des fichiers ici.", "The list is empty. Drop files here."),
    ("Glissez un fichier audio, vidéo ou image ici\nou faites un clic droit pour ouvrir.", "Drop an audio, video or image file here\nor right-click to open."),
    ("Plein écran", "Full screen"),
    ("Quitter le plein écran", "Exit full screen"),
    ("Piste audio", "Audio track"),
    ("Sous-titres", "Subtitles"),
    ("Désactivés", "Off"),
    ("Charger un sous-titre…", "Load subtitle file…"),
    ("Piste", "Track"),
    ("Vitesse", "Speed"),
    ("Volume", "Volume"),
    ("Muet", "Mute"),
    ("Capture d'image", "Screenshot"),
    ("Enregistrer le média", "Save media"),
    ("Finalisation du fichier…", "Finalizing file…"),
    ("Enregistrement du direct… recliquez pour arrêter et finaliser.", "Recording live… click again to stop and finalize."),
    ("Téléchargement en cours…", "Downloading…"),
    ("Média enregistré : ", "Media saved: "),
    ("Enregistrement impossible : ", "Cannot save: "),
    ("Capture enregistrée : ", "Screenshot saved: "),
    ("Capture impossible : ", "Screenshot failed: "),
    ("Image copiée dans le presse-papiers.", "Image copied to clipboard."),
    ("Copie impossible : ", "Copy failed: "),
    ("Copier l'image", "Copy image"),
    ("Pivoter", "Rotate"),
    ("Ajuster", "Fit"),
    ("Taille réelle (1:1)", "Actual size (1:1)"),
    ("Zoom +", "Zoom in"),
    ("Zoom −", "Zoom out"),
    ("Diaporama", "Slideshow"),
    ("s", "s"),
    ("Convertir avec MediaForge", "Convert with MediaForge"),
    ("Afficher dans le dossier", "Show in folder"),
    ("Copier le chemin", "Copy path"),
    ("Précédent", "Previous"),
    ("Suivant", "Next"),
    ("Lecture", "Play"),
    ("Pause", "Pause"),
    ("Erreur de lecture : ", "Playback error: "),
    ("Reprise à ", "Resumed at "),
    ("Aide (F1)", "Help (F1)"),
    ("Réglages", "Settings"),
    ("Flou des bords (0 = net)", "Edge blur (0 = sharp)"),
    ("Notes disponibles pour les fichiers locaux uniquement", "Notes are only available for local files"),
    ("Note de derushage", "Rough-cut note"),
    ("Timecode", "Timecode"),
    ("Durée d'affichage (s)", "Display duration (s)"),
    ("Note", "Note"),
    ("100 caractères maximum", "100 characters max"),
    ("Enregistrer", "Save"),
    ("Annuler", "Cancel"),
    ("Timecode ou texte invalide", "Invalid timecode or text"),
    ("Notes de derushage", "Rough-cut notes"),
    ("Afficher sur l'image", "Show on picture"),
    ("Durée par défaut (s)", "Default duration (s)"),
    ("+ Note ici (A)", "+ Note here (A)"),
    ("Aucune note. Touche A : annoter l'instant courant.", "No notes. Press A to annotate the current time."),
    ("Aller à cet instant", "Go to this time"),
    ("Notes de derushage (J)", "Rough-cut notes (J)"),
    ("Annoter l'instant courant (A)", "Annotate current time (A)"),
    ("Timecode en surimpression (T)", "Timecode overlay (T)"),
    ("Derushage", "Rough-cut tools"),
    ("N° d'image", "Frame no."),
    ("Temps restant", "Time remaining"),
    ("Position", "Position"),
    ("Taille", "Size"),
    ("Afficher les notes sur l'image", "Show notes on picture"),
    ("Mode cinéma : bords flous en miroir si le format n'est pas celui de l'écran (touche C)", "Cinema mode: blurred mirrored edges when the aspect differs from the screen (key C)"),
    ("Mode cinéma activé", "Cinema mode on"),
    ("Mode cinéma désactivé", "Cinema mode off"),
    ("Langue :", "Language:"),
    ("Charger les fichiers du dossier à l'ouverture", "Load folder files on open"),
    ("Accélération matérielle du décodage", "Hardware video decoding"),
    ("Mise à l'échelle haute qualité (GPU requis, au prochain démarrage)", "High-quality scaling (needs a GPU, applies on next start)"),
    ("Reprendre la lecture où elle s'était arrêtée", "Resume playback where it stopped"),
    ("Durée du diaporama :", "Slideshow delay:"),
    ("Associer les formats… (Windows)", "Associate file types… (Windows)"),
    ("Rester en mémoire à la fermeture (réduit ; Ctrl+Q pour quitter)", "Stay in memory when closed (minimized; Ctrl+Q to quit)"),
    ("Couleur copiée :", "Color copied:"),
    ("Pipette (touche I) : format copié au clic", "Eyedropper (I key): format copied on click"),
    ("Pipette de couleur (I)", "Color picker (I)"),
    ("Démarrer avec Windows (réduit)", "Start with Windows (minimized)"),
    ("Ouvre ensuite les « Applications par défaut » de Windows pour choisir MediaForge Player.", "Then opens Windows “Default apps” so you can pick MediaForge Player."),
    ("libmpv est introuvable.", "libmpv was not found."),
    ("Placez libmpv-2.dll (Windows) ou libmpv.so.2 (Linux) à côté de l'exécutable, ou choisissez son dossier.", "Put libmpv-2.dll (Windows) or libmpv.so.2 (Linux) next to the executable, or choose its folder."),
    ("Choisir le dossier de libmpv…", "Choose libmpv folder…"),
    ("Redémarrez le lecteur.", "Please restart the player."),
    ("Raccourcis", "Shortcuts"),
    ("Espace : lecture/pause · ←/→ : ±5 s (Ctrl : ±30 s) · ↑/↓ : volume · M : muet · F ou F11 : plein écran · PgSuiv/PgPréc ou N/P : suivant/précédent · [ ] : vitesse · S : capture · R : pivoter · + − 0 1 : zoom · L : liste · O : ouvrir · Échap : quitter le plein écran", "Space: play/pause · ←/→: ±5 s (Ctrl: ±30 s) · ↑/↓: volume · M: mute · F or F11: full screen · PgDn/PgUp or N/P: next/previous · [ ]: speed · S: screenshot · R: rotate · + − 0 1: zoom · L: playlist · O: open · Esc: leave full screen"),
];

fn table() -> &'static HashMap<&'static str, &'static str> {
    static T: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    T.get_or_init(|| EN.iter().copied().collect())
}

pub fn tr<'a>(l: Lang, fr: &'a str) -> &'a str {
    match l {
        Lang::Fr => fr,
        Lang::En => table().get(fr).copied().unwrap_or(fr),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_duplicates() {
        let mut s = std::collections::HashSet::new();
        for (k, _) in EN {
            assert!(s.insert(*k), "doublon : {k}");
        }
    }
    /// Toute chaîne passée littéralement à `tr(l, "…")` doit être traduite.
    #[test]
    fn literal_calls_translated() {
        let mut missing = Vec::new();
        for src in [include_str!("app.rs")] {
            let src = src.split("#[cfg(test)]").next().unwrap();
            let pat = "tr(l, \"";
            let mut rest = src;
            while let Some(i) = rest.find(pat) {
                rest = &rest[i + pat.len()..];
                let b = rest.as_bytes();
                let mut e = 0;
                while e < b.len() && !(b[e] == b'"' && b[e - 1] != b'\\') {
                    e += 1;
                }
                let key = rest[..e].replace("\\n", "\n");
                if !table().contains_key(key.as_str()) {
                    missing.push(key);
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "traductions manquantes : {missing:#?}");
    }
}
