# Forge — la suite multimédia en Rust

**MediaForge · Player · FileForge · AutoForge** : quatre applications rapides, écrites en Rust (egui), pour Windows et Linux, qui s'ouvrent les unes les autres. Convertir, lire, ranger et retrouver vos fichiers — avec une IA locale optionnelle qui ne sort jamais de votre machine.

![Rust](https://img.shields.io/badge/Rust-2021-orange) ![Windows](https://img.shields.io/badge/Windows-oui-blue) ![Linux](https://img.shields.io/badge/Linux-oui-blue) ![Licence](https://img.shields.io/badge/licence-MIT-green)

## Les applications

| Application | Rôle |
|---|---|
| **MediaForge** | Convertisseur de médias par lots (vidéo, audio, image) basé sur FFmpeg, sans ligne de commande. |
| **Player** | Lecteur audio, vidéo et images (libmpv) avec outils de derushage. |
| **FileForge** | Gestionnaire de fichiers à double panneau, avec file d'attente d'opérations. |
| **AutoForge** | Tri automatique par règles, classement par IA locale (LM Studio) et identification des fichiers. |

## Fonctionnalités

### MediaForge
- Conversion par lots avec profils (MP4, MKV, WebM, HDLight x264/x265, audio, images…).
- **Éditeur visuel de profils** : codec, qualité (CRF ou débit), preset, taille, images/s, désentrelacement, débruitage, audio — sans toucher aux arguments FFmpeg (mode avancé disponible).
- **Incrustations** : timecode à la cadence réelle du fichier, textes, logo PNG avec opacité, et **aperçu** de l'image encodée.
- Retouches par fichier : découpe et recadrage.
- FFmpeg « lite » (détecté) ou « full » (téléchargé à la demande) pour l'**accélération GPU** (NVENC, QSV, AMF, VAAPI).
- Outils : fusion, extraction/remplacement de piste audio, mixage, renommage et traitement d'images par lots.
- Téléchargement en ligne (yt-dlp), listes de lecture comprises.
- **Musiques rétro** : modules Amiga/PC (`.mod .xm .s3m .it` et autres, via libopenmpt), Atari ST (`.ym`, `.sndh`) et Commodore 64 (`.sid` PSID) convertibles en MP3, WAV, FLAC…

### Player
- Lecture de pratiquement tous les formats, décodage matériel, reprise de lecture, liste de lecture, boucle et lecture aléatoire.
- **Derushage** : timecode en surimpression, notes horodatées enregistrées à côté de la vidéo.
- **Mode cinéma** (bords flous), **pipette de couleur** (code `#RRGGBB` / `rgb()` copié au clic).
- Images, PDF, AI/PSD/EPS et maillages 3D (STL, OBJ).
- Résidence en mémoire, démarrage avec Windows, association des formats.

### FileForge
- Deux panneaux, saisie rapide au clavier, tri par nom/date/type/poids (clic droit).
- **File d'attente** FIFO des copies, déplacements, suppressions et archivages (annulation, priorité, historique).
- **Glisser-déposer** entre panneaux, depuis l'Explorateur et vers d'autres applications (Windows).
- Renommage avancé, archives ZIP/7z/RAR, **recherche multicritères** (nom, type, taille, date ou période, contenu, tous les lecteurs).
- Menu contextuel « Gérer avec FileForge » dans l'Explorateur.

### AutoForge
- Tri par règles ordonnées avec jetons de dossier (`{année}`, `{mois}`, `{type}`, `{artiste}`…), détection des doublons, plan validable et **annulation**.
- Classement par **IA locale** dans vos propres catégories (photos et textes).
- Commandes en langage naturel, identification des films, musiques et ROMs.

## Aide intégrée

Le dossier `www/` contient un **minisite d'aide illustré** (recherche instantanée, cas pratiques). Il s'ouvre depuis les boutons **?** et la touche **F1** de chaque application, sans connexion Internet. Il doit être placé à côté des exécutables.

## Prérequis

| Composant | Usage | Installation (Windows) |
|---|---|---|
| FFmpeg | conversions | `winget install Gyan.FFmpeg` |
| libmpv (`libmpv-2.dll`) | Player | à placer à côté de l'exécutable |
| yt-dlp | téléchargement en ligne (facultatif) | `winget install yt-dlp.yt-dlp` |
| 7-Zip / WinRAR | archives (facultatif) | `winget install 7zip.7zip` |
| LM Studio | IA locale (facultatif) | `winget install ElementLabs.LMStudio` |

## Compilation

```bash
cargo build --release
```

Les exécutables se trouvent dans `target/release/` : `mediaforge`, `mediaforge-player`, `fileforge`, `autoforge`. Tests : `cargo test --workspace` (les tests qui exigent FFmpeg sont ignorés s'il est absent).

## Organisation du dépôt

```
crates/ff-core       types communs, profils, formats
crates/ff-engine     moteur de conversion FFmpeg et file de tâches
crates/ff-chip       rendu audio des musiques rétro (YM, SNDH, SID)
crates/ff-ui         interface MediaForge
crates/ff-player     lecteur
crates/ff-forge      gestionnaire de fichiers
crates/autoforge     tri et IA locale
crates/ff-kit        composants d'interface partagés (icônes, thèmes, aide)
vendor/              dépendance tierce corrigée (SNDH)
www/                 aide illustrée et page de téléchargement
```

## Confidentialité

Aucune donnée n'est envoyée à un service externe par défaut. L'IA fonctionne uniquement avec un serveur local (LM Studio). Les téléchargements, mises à jour de FFmpeg et recherches d'identification (TMDB, iTunes, Wikipédia, MusicBrainz) ne se font qu'à la demande.

## Licence

MIT. Le dossier `vendor/ym2149-sndh-replayer` est une copie de la bibliothèque du même nom (licence MIT) avec une correction de compilation.
