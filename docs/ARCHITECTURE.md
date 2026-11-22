# Chud Input Recorder — Architecture

> Document vivant. Il décrit les choix techniques, leur justification et la façon dont les
> modules s'articulent. Le statut réel de chaque fonctionnalité est suivi dans `ROADMAP.md`,
> le format de fichier dans `FORMAT.md`.

## 1. Principes directeurs

1. **Le moteur d'abord.** La capture, l'ordonnancement et l'injection sont écrits en Rust,
   sans dépendance à l'interface. L'interface est un client du moteur, jamais l'inverse.
2. **Fidélité mesurable.** Toute affirmation sur la précision (timing, positions) doit être
   vérifiable par un outil de diagnostic intégré (`chud diag`, panneau Diagnostics).
3. **Non destructif.** La prise originale d'un enregistrement est conservée intacte dans le
   fichier. Toutes les éditions sont réversibles (annuler/rétablir, historique de versions).
4. **Sûreté de l'utilisateur.** Arrêt d'urgence physique, libération systématique des
   touches/boutons maintenus, aucune donnée ne quitte la machine, exclusions d'applications.
5. **Honnêteté envers le système.** Les protections de l'OS (UIPI, Wayland, contenu protégé)
   sont expliquées à l'utilisateur, jamais contournées.
6. **Ouvert.** Format de fichier documenté (`FORMAT.md`), export JSON complet.

## 2. Choix technologiques

| Couche | Choix | Pourquoi |
|---|---|---|
| Moteur (capture, lecture, édition, formats, vision) | **Rust** (workspace de crates) | Performance, absence de GC (pas de pauses pendant la lecture), sûreté mémoire, accès direct aux API système (`windows`, `evdev`, `x11rb`). |
| Application desktop | **Tauri 2** (WebView2 sous Windows, WebKitGTK sous Linux) | Binaire léger, moteur Rust dans le même processus, et le web permet l'interface « spatiale » visée (dégradés, flou, animations, canvas) bien mieux qu'egui ou Slint aujourd'hui. |
| Interface | **React 19 + TypeScript + Vite**, état **Zustand**, CSS natif à tokens | Écosystème mature et maintenable. Timeline, carte des trajectoires et vue en direct sont dessinées sur **canvas** (pas de DOM par événement). |
| Manettes (capture) | **XInput** (Windows), **evdev** (Linux) | XInput : sans permission, fonctionne fenêtre en arrière-plan, couvre Xbox et tout ce qui s'y présente (Steam Input, DS4Windows…). evdev : toutes les manettes connues du noyau (xpad, hid-playstation, hid-nintendo, génériques). |
| Manettes (lecture) | **ViGEmBus** via `vigem-client` (Windows), **uinput** (Linux) | Manettes Xbox 360 virtuelles vues par les jeux comme de vraies manettes. ViGEmBus est détecté à l'exécution ; s'il manque, la lecture manette est désactivée avec une explication. |
| Reconnaissance de texte | OCR intégré à Windows 10/11 (`Windows.Media.Ocr`) | Hors ligne, dans les langues installées, sans dépendance à embarquer. Pas d'équivalent sous Linux pour l'instant. |
| Polices | Sora (UI), JetBrains Mono (valeurs, timecodes) — embarquées | Aucune requête réseau : l'application fonctionne entièrement hors ligne. |

Alternatives écartées :
- **egui** : excellent pour des outils internes, mais le rendu « immediate mode » rend les
  animations, la typographie fine et les effets de verre coûteux à obtenir proprement.
- **Slint / Qt** : très bons pour du natif, mais l'itération visuelle est plus lente et Qt
  ajoute une chaîne de build C++ et des contraintes de licence.
- **Electron** : trop lourd en mémoire pour un outil qui reste ouvert en tâche de fond.
- **SDL3 pour les manettes** : envisagé, écarté pour éviter une dépendance C de plus ; XInput
  et evdev couvrent les cas courants (limite : manettes HID non-XInput sous Windows).

## 3. Vue d'ensemble

```
┌──────────────────────────── Application Tauri ─────────────────────────────┐
│  WebView (React)                                                           │
│   Library · Studio (timeline) · Flow · Observatory (live) · Settings ·     │
│   Diagnostics                                                              │
│        ▲ commandes (invoke, JSON)     ▲ événements (emit, JSON) ;          │
│        │                              │ pixels de capture en binaire brut   │
├────────┼──────────────────────────────┼─────────────────────────────────────┤
│  src-tauri : commandes, documents ouverts, bibliothèque, réglages,          │
│              fenêtres (principale + indicateur), zone de notification       │
├─────────────────────────────────────────────────────────────────────────────┤
│  chud-engine : machine d'états, Recorder, Player, raccourcis, runtime Flow, │
│                recherche à l'écran                                          │
├─────────────────────────────────────────────────────────────────────────────┤
│  chud-core : événements, Timeline, historique, transformations, format      │
│              .chud, plan de lecture, Flow (AST + expressions), vision        │
│              [aucune API système]                                           │
├─────────────────────────────────────────────────────────────────────────────┤
│  chud-platform : trait Platform / Injector                                  │
│   Windows : hooks LL · Raw Input · SetWinEventHook · SendInput · XInput ·   │
│             ViGEm · GDI · OCR                                               │
│   Linux   : evdev · uinput · XTest · X11 (fenêtres, écrans, capture)        │
└─────────────────────────────────────────────────────────────────────────────┘
        chud (CLI) : enregistrement/lecture/export, diagnostics, recherche à l'écran
```

### Crates

| Crate | Rôle | Dépend de |
|---|---|---|
| `chud-core` | Types, timeline, édition, format, plan de lecture, Flow, vision. Pur calcul → testable partout. | — |
| `chud-platform` | Abstraction OS. Windows et Linux implémentés derrière le même trait ; macOS non commencé. | core |
| `chud-engine` | Orchestration temps réel : threads de capture et de lecture, sécurité, raccourcis, Flow. | core, platform |
| `chud-cli` (binaire `chud`) | Diagnostics et automatisation sans interface. | engine |
| `app/src-tauri` (binaire `chud-input-recorder`) | Pont entre le moteur et l'interface. | engine |

## 4. Modèle d'événements

Le temps est exprimé en **microsecondes** (`i64`) depuis le début de l'enregistrement,
mesuré sur l'horloge monotone (`QueryPerformanceCounter` sous Windows ; horodatages du noyau
`CLOCK_MONOTONIC` des événements evdev sous Linux). La microseconde couvre les souris
8 000 Hz (125 µs entre rapports) avec marge.

```rust
struct Event { id: u32, t: i64, kind: EventKind }   // Copy, aucune allocation

enum EventKind {
    MouseMove  { x, y }                 // position absolue, pixels physiques, bureau virtuel
    MouseRaw   { dx, dy }               // déplacement brut du capteur (jeux, curseur capturé)
    MouseDown/MouseUp { button, x, y }  // gauche, droit, milieu, X1, X2
    Wheel      { delta, horizontal }    // unités WHEEL_DELTA (120 = un cran)
    KeyDown/KeyUp { key: Key{vk, scan}, repeat }
    PadButton  { pad, button, down }
    PadAxis    { pad, axis, value }     // i16 normalisé
}
```

- Les durées (maintien d'une touche, d'un bouton) ne sont **pas stockées** : elles sont la
  distance entre un `Down` et le `Up` apparié. Éditer une durée revient à déplacer le `Up`.
  Aucun état incohérent possible (« durée » contredisant les événements).
- Les événements simultanés sont ordonnés par `(t, id)` : l'ordre de capture est préservé.
- **Contexte** (piste séparée) : changements de fenêtre active (processus, titre), capturés
  sans sondage sous Windows (`SetWinEventHook`), via `_NET_ACTIVE_WINDOW` sous X11. Sert à
  l'affichage, aux exclusions de confidentialité et à l'option « une seule application ».
- **Environnement** : système, écrans (bornes, échelle, principal, fréquence), disposition
  clavier, vitesse et accélération de la souris. Permet de détecter qu'un fichier est lu sur
  une configuration différente et d'adapter les coordonnées (`adapt_to_screens`).

## 5. Capture

### Windows

Un **thread de capture dédié** possède une boucle de messages et héberge :

| Source | Données | Pourquoi |
|---|---|---|
| `WH_KEYBOARD_LL` | vk, scan, étendu, injecté | Ordre exact, indicateur d'injection, possibilité d'**avaler** les raccourcis globaux. |
| `WH_MOUSE_LL` | position absolue, boutons, molettes, injecté | Coordonnées « per-monitor aware » (le processus est DPI-aware v2). |
| Raw Input (`RIDEV_INPUTSINK`) | deltas bruts | Jeux à curseur capturé, souris haute fréquence. |
| `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` | fenêtre active | Contexte, exclusions, filtrage par application. |
| XInput (sondage) | manettes 1 à 4 | Voir §2. |

Règles du callback de hook : horodatage immédiat, filtrage, poussée dans une file sans verrou,
retour. Rien d'autre : Windows retire silencieusement un hook qui dépasse
`LowLevelHooksTimeout`.

Les raccourcis globaux sont reconnus **dans le hook**, sur les seules frappes physiques
(non injectées), puis avalés : ils n'atteignent ni l'application ciblée ni l'enregistrement.
Les modificateurs du raccourci d'arrêt pressés juste avant l'arrêt sont retirés de la fin de
la prise, et les relâchements orphelins du raccourci de départ sont ignorés.

**Touches tapées dans la fenêtre de Chud.** Windows n'appelle pas le hook clavier bas niveau
de Chud tant que sa propre fenêtre (WebView2) a le focus ; le hook souris, lui, continue.
L'interface relaie donc ses propres `keydown`/`keyup` (commande `local_key`, sans les
répétitions automatiques) au thread de capture, qui les traite comme le hook : raccourcis
globaux et vue en direct fonctionnent Chud au premier plan. Un tableau des touches
enfoncées écarte une touche déjà vue par le hook. Ces frappes ne sont jamais enregistrées
(la fenêtre de Chud est toujours exclue de l'enregistrement), et les sélecteurs de raccourci
les interceptent avant le relais (choisir F9 ne lance pas d'enregistrement). L'interface
empêche aussi Alt ou F10 seuls d'ouvrir le mode menu de la fenêtre sans cadre, qui la
figeait jusqu'à Échap.

### Linux

- Entrées lues directement dans le noyau (**evdev**, `/dev/input/event*`) : fonctionne sous
  X11, Wayland et en console. Nécessite d'appartenir au groupe `input` ; Diagnostics
  l'explique, Chud ne modifie jamais les permissions lui-même.
- Claviers toujours ouverts (raccourcis) ; souris ouvertes seulement quand c'est utile
  (enregistrement, vue en direct, interruption par l'utilisateur). Branchement à chaud vu
  immédiatement (inotify sur `/dev/input`).
- Positions absolues et fenêtre active lues via **X11**. Sous **Wayland**, aucune API standard
  ne les expose : seuls les mouvements relatifs sont enregistrés, les exclusions
  d'applications ne peuvent pas s'appliquer (signalé dans Diagnostics).
- Les raccourcis ne peuvent pas être avalés (l'application active les voit aussi), mais ils
  ne sont jamais enregistrés.
- Les périphériques virtuels de Chud (préfixe de nom réservé) ne sont jamais enregistrés.

### Compte à rebours et démarrage

Un enregistrement peut démarrer après un **compte à rebours** (affiché dans l'indicateur, son
optionnel), ou à la **première entrée** de l'utilisateur. Une durée maximale l'arrête seule.

## 6. Lecture

1. `chud-core::playback::compile` transforme timeline + réglages en **plan** trié
   (vitesse, plage ou sélection, mode de timing, mode souris, mode clavier, coordonnées).
   Fonction pure, testée.
2. Le **Player** exécute le plan sur un thread dédié (sous Windows : priorité
   `TIME_CRITICAL` et `timeBeginPeriod(1)` pendant la lecture).
3. Attente hybride (`chud-platform::timer`) : sommeil système qui se réveille juste avant
   l'échéance, puis courte attente active sur l'horloge monotone. La marge s'adapte au retard
   mesuré du système. Windows : minuterie haute résolution
   (`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`) ; Linux : `clock_nanosleep`. La précision réelle
   de la machine se mesure avec `chud diag timer`.
4. Les actions dont les échéances coïncident sont envoyées ensemble (un seul `SendInput`
   sous Windows : insertion atomique, pas d'entrelacement avec la saisie réelle).

| Mode de timing | Comportement |
|---|---|
| Exact | Échéances ancrées au départ : aucune dérive cumulée. |
| Relatif | Chaque intervalle est respecté à partir de l'envoi précédent réel. |
| Adaptatif | Comme Exact, mais un retard important ré-ancre la timeline au lieu de rattraper en rafale. |
| Rapide | Intervalles supprimés, groupes simultanés conservés. |
| Personnalisé | Intervalles bornés ou remplacés par une valeur fixe. |

| Mode souris | Usage |
|---|---|
| Absolu | Bureautique : la souris va exactement aux coordonnées enregistrées. |
| Relatif (brut) | Jeux à curseur capturé : rejoue les deltas du capteur. |
| Désactivé | Seuls les clics, la molette et le clavier sont rejoués. |

Clavier : mode **physique** (scancodes, indépendant de la disposition logique, compatible
DirectInput) ou **logique** (codes virtuels : rejoue les mêmes caractères sur une autre
disposition).

Injection Linux : **XTest** en session X11 (un seul flux ordonné, positions absolues
exactes, aucune permission), sinon **uinput** (clavier + souris virtuels, créés une fois pour
toute la vie du processus ; positions absolues indisponibles sous Wayland).

### Sécurité de lecture

- Le Player tient l'ensemble des touches, boutons et entrées de manette qu'il a enfoncés.
  Arrêt, erreur, panique (`Drop`) : tout est relâché. En pause, ils sont relâchés puis
  ré-enfoncés à la reprise.
- Arrêt d'urgence : raccourci physique global, traité côté capture, donc actif même si
  l'interface est figée.
- Option « interrompre si je bouge la souris / tape au clavier » (entrées non injectées).
- Option « remettre le curseur où il était » à la fin.

## 7. Édition

- `Timeline` : `Vec<Event>` trié + index `id → position`.
- Toute modification est un `Change { removed, added }` dont l'inverse est trivial ; une
  transaction regroupe des changements sous un libellé (« Déplacer 12 événements »).
  L'historique coûte la taille de ce qui change, pas celle du document.
- Outils (tous en Rust, tous réversibles) : suppression des temps morts (maintiens
  respectés), simplification des trajectoires (Ramer–Douglas–Peucker, bornes préservées),
  fusion des mouvements bruts, fusion des crans de molette, nettoyage (relâchements orphelins,
  touches restées enfoncées, répétitions automatiques), timing uniforme, détection de
  répétitions, extraction d'une plage en nouvelle macro.
- L'interface reçoit les événements en **JSON**. C'est suffisant pour des prises de quelques
  dizaines de milliers d'événements ; un transfert binaire en colonnes reste à faire pour les
  très longues prises (voir `ROADMAP.md`).

## 8. Automatisation (Flow)

Une macro peut avoir un **Flow** : un arbre de blocs exécuté par le runtime Rust
(`chud-engine`). Une macro sans Flow se comporte comme un simple enregistrement : l'usage
basique reste un clic sur « Lire ».

- Blocs : lire l'enregistrement (plage, vitesse), lire une autre macro (imbrication limitée
  à 8 niveaux, ce qui arrête aussi une macro qui s'appellerait elle-même), attendre, combinaison de touches, touche enfoncée/relâchée, taper un
  texte, déplacer la souris (instantané, linéaire, doux, humain), cliquer, bouton
  enfoncé/relâché, molette, répéter, tant que, si/sinon, attendre que, **trouver à l'écran**,
  variable, notification, sortir de la boucle, continuer, arrêter.
- Conditions : expression, fenêtre active/existante, couleur d'un pixel, touche maintenue,
  **image/texte/couleur à l'écran**, hasard, et leurs combinaisons (non, toutes, au moins une).
- Expressions : petit langage sûr (arithmétique, comparaisons, `random`, variables), sans
  accès au système.
- Déclencheurs : aujourd'hui, des **raccourcis globaux** associés à des macros (option : avec
  ou sans leur Flow). Pas encore de déclencheur au démarrage, à l'activation d'une fenêtre ou
  horaire.
- La liste et les champs exacts sont dans `FORMAT.md` §5–6.

## 9. Recherche à l'écran

- **Capture** (`chud-platform`) : GDI sous Windows (tous les écrans, pixels physiques du
  bureau virtuel) ; `GetImage` X11 sous Linux. Sous Wayland, la capture est refusée avec une
  explication (seul le compositeur peut lire l'écran ; aucun contournement).
- **Vision** (`chud-core::vision`, pur calcul) :
  - image : corrélation croisée normalisée (la mesure `TM_CCOEFF_NORMED` d'OpenCV), donc
    insensible aux changements de luminosité et de contraste ; recherche grossière sur une
    pyramide puis affinage, score final en couleur à pleine résolution ; pixels transparents
    ignorés ; option « toute taille » qui essaie plusieurs échelles ;
  - couleur : premier pixel dans la tolérance ;
  - texte : mots reconnus par l'OCR, recherche d'une phrase, sensible ou non à la casse.
- **Sélecteur** (interface) : Chud se masque, capture tous les écrans, puis affiche une image
  figée où l'on choisit une image, une zone ou un point (zoom, loupe au pixel, flèches pour
  ajuster). Les pixels arrivent en binaire brut, pas en JSON.
- **Données** : les captures restent en mémoire et sont effacées à la fermeture du sélecteur.
  Seules les images retenues par l'utilisateur sont stockées dans la macro (`images/*.png`).
- L'indicateur d'enregistrement demande à Windows d'être exclu des captures
  (`content_protected`) pour ne pas fausser les recherches (non encore vérifié visuellement).

## 10. Fenêtre et interface

- Fenêtre sans cadre système : sa barre de titre est dessinée par l'interface. Au démarrage
  la fenêtre est dimensionnée à la zone de travail de l'écran, et si un redimensionnement, un
  changement d'échelle ou un déplacement laisse la barre de titre hors d'atteinte, elle est
  ramenée sur l'écran le plus proche (`placement.rs`).
- Mise en page adaptative (petits écrans, fortes mises à l'échelle) et **taille de
  l'interface** réglable (80–150 %, Ctrl+= / Ctrl+- / Ctrl+0) par-dessus l'échelle du système.
- Langues : français et anglais (choix automatique selon le système, ou forcé).
- Options d'accessibilité : réduire les animations, désactiver le fond étoilé et les halos.

## 11. Stockage

- Bibliothèque = un dossier visible (`Documents/Chud Input Recorder/Library`) ; les dossiers
  de la bibliothèque sont de vrais dossiers. Réglages et journaux dans le dossier de
  configuration de l'utilisateur.
- **Mode portable** : si `portable.flag` est présent à côté de l'exécutable, tout est rangé
  dans `./data` (réglages, bibliothèque, journaux).
- Fichier `.chud` = archive ZIP (`FORMAT.md`), écrite de façon atomique.
- Sauvegarde automatique des documents ouverts et historique des versions par macro (nombre
  de versions conservées réglable). Corbeille avant suppression définitive.

## 12. Confidentialité et permissions

- Aucune connexion réseau n'est ouverte par l'application. Pas de télémétrie.
- Indicateurs d'enregistrement : overlay discret toujours visible, icône de zone de
  notification, minuteur dans l'interface, titre de la fenêtre.
- Applications exclues : l'enregistrement ignore les entrées quand l'une d'elles est au
  premier plan (liste par défaut : gestionnaires de mots de passe et invites d'identifiants
  Windows) ; la coupure est marquée dans la timeline. Chud s'exclut aussi lui-même : s'il a
  le focus quand un enregistrement démarre (bouton Enregistrer), il se réduit pour laisser
  la main à l'application visée, puis revient à l'arrêt (réglage « Réduire pendant
  l'enregistrement »).
- Windows : aucun droit administrateur requis. Limite **UIPI** : un processus non élevé ne
  peut ni observer ni piloter une fenêtre élevée ; Diagnostics l'indique, sans jamais
  contourner cette protection.
- Linux : lecture d'evdev via le groupe `input` et écriture via `/dev/uinput`, expliquées
  dans Diagnostics ; Chud ne change aucune permission.
- Les entrées injectées sont marquées comme telles par Windows et détectables par les
  anti-cheats. Chud ne masque pas cet indicateur.

## 13. Qualité

- `chud-core`, `chud-engine`, `chud-platform`, CLI, application : plus de 100 tests unitaires
  Rust (format aller-retour, historique réversible, transformations, plan de lecture,
  expressions et runtime Flow, vision, lecteur contre un backend simulé). Pas de tests de
  propriétés pour l'instant.
- Tests de bouclage (`chud diag loopback`, Windows et Linux) : injecte des séquences connues
  (touches F13–F24 sans effet, déplacements souris restaurés, manette virtuelle si
  disponible), les recapture via nos propres hooks et compare positions et timings.
- Interface : tests Vitest (arbre Flow, paliers de zoom…) et vérification de bout en bout sur
  l'application réelle via le protocole de débogage de WebView2.
- `cargo clippy --workspace --all-targets` sans avertissement sur les deux systèmes.
