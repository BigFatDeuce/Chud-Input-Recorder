# Feuille de route et état réel

> Règle : une fonctionnalité n'est marquée **Fait** que si elle est implémentée **et** vérifiée
> (tests automatiques, diagnostic intégré ou essai réel sur l'application). Ce qui manque est
> écrit tel quel.

Légende : **Fait** · **Partiel** (utilisable, avec limites décrites) · **À faire**

Plateformes de vérification : Windows 11 (3 écrans, 100–200 % de mise à l'échelle) et Linux
(Fedora 45 sous WSL2 ; X11 testé avec Xvfb, Wayland avec WSLg).

## Enregistrement

| Fonctionnalité | État | Vérifié par / limites |
|---|---|---|
| Clavier, boutons et molettes de souris, positions absolues | Fait | bouclage `chud diag loopback` (Windows, Linux) |
| Mouvements bruts du capteur (jeux à curseur capturé) | Fait | Raw Input (Windows), evdev (Linux) |
| Manettes | Fait | XInput sous Windows (Xbox et ce qui s'y présente : Steam Input, DS4Windows…) ; evdev sous Linux (Xbox, PlayStation, Nintendo, génériques). Les manettes HID non-XInput ne sont pas vues sous Windows. |
| Raccourcis globaux (enregistrer, lire, pause, arrêt d'urgence) | Fait | jamais enregistrés ; sous Windows ils sont aussi avalés (l'application ciblée ne les voit pas), sous Linux elle les voit aussi |
| Position absolue du pointeur sous Wayland | À faire | Wayland ne l'expose pas : seuls les mouvements relatifs sont enregistrés (expliqué dans Diagnostics) |
| Compte à rebours avant l'enregistrement (avec son optionnel) | Fait | |
| Démarrer à la première entrée, durée maximale, filtre de touches, application unique | Fait | |
| Indicateur d'enregistrement (overlay, zone de notification, minuteur) | Fait | |
| Applications exclues (gestionnaires de mots de passe, invites d'identifiants par défaut) | Fait | Windows et Linux X11 ; impossible sous Wayland (fenêtre active illisible), signalé dans Diagnostics |
| macOS | À faire | aucun code macOS pour l'instant |

## Lecture

| Fonctionnalité | État | Vérifié par / limites |
|---|---|---|
| Vitesse, répétitions, délai entre répétitions | Fait | tests unitaires du plan de lecture |
| Modes de timing : exact, relatif, adaptatif, rapide, personnalisé | Fait | tests unitaires + `chud diag timer` |
| Souris absolue / relative / désactivée ; clavier physique / logique | Fait | |
| Adapter les coordonnées à une autre configuration d'écrans | Fait | |
| Lecture manette (manettes Xbox 360 virtuelles) | Fait | ViGEmBus sous Windows (détecté au démarrage), uinput sous Linux |
| Relâchement de tout ce qui est maintenu à l'arrêt, en pause, en cas d'erreur | Fait | tests du lecteur contre un backend simulé |
| Arrêt d'urgence physique, interruption si l'utilisateur bouge | Fait | |
| Lecture d'une sélection ou d'une plage de la timeline | Fait | |

## Édition (Studio)

| Fonctionnalité | État | Vérifié par / limites |
|---|---|---|
| Timeline non destructive (prise originale conservée), annuler/rétablir | Fait | tests unitaires de l'historique |
| Inspecteur, liste d'événements, carte des trajectoires, statistiques | Fait | |
| Outils : temps morts, simplification de trajectoires, fusion molette / mouvements bruts, nettoyage, timing uniforme, détection de répétitions, extraction en nouvelle macro | Fait | tests unitaires des transformations |
| Transfert binaire en colonnes vers l'interface | À faire | les événements passent aujourd'hui en JSON ; suffisant pour des milliers d'événements, à revoir pour des centaines de milliers |

## Automatisation (Flow)

| Fonctionnalité | État | Vérifié par / limites |
|---|---|---|
| Éditeur visuel par blocs (glisser-déposer, raccourcis clavier) | Fait | essais sur l'application |
| Boucles, conditions, variables, expressions sûres, macros imbriquées | Fait | tests unitaires de l'interpréteur |
| Conditions : expression, fenêtre active/existante, couleur de pixel, touche maintenue, hasard, image/texte/couleur à l'écran | Fait | |
| **Trouver à l'écran** : image, couleur ou texte → cliquer, double-cliquer, clic droit/milieu, déplacer, ou seulement retenir la position (variables `found_x`…) ; attente, zone, décalage, comportement si introuvable | Fait | tests unitaires ; essais réels Windows (icône, bouton Démarrer trouvé à 100 % en ~150 ms capture comprise, lecture réelle du flux) ; capture X11 vérifiée sous Xvfb |
| Image « toute taille » (autre mise à l'échelle) | Fait | essai réel : image agrandie à 125 % retrouvée |
| Texte à l'écran (OCR) | Partiel | Windows : OCR intégré, hors ligne (vérifié). Linux : pas d'OCR pour l'instant. |
| Recherche à l'écran sous Wayland | À faire | Wayland ne laisse que le compositeur lire l'écran ; une session X11 est nécessaire (message explicite) |
| Sélecteur d'écran (capture figée, zoom, loupe, multi-écrans, image/zone/point) | Fait | essais réels sur 3 écrans |
| Déclencheurs | Partiel | raccourcis globaux par macro uniquement ; à faire : au démarrage, à l'activation d'une fenêtre, horaire |

## Bibliothèque et données

| Fonctionnalité | État | Vérifié par / limites |
|---|---|---|
| Dossiers, favoris, recherche, tri, corbeille | Fait | |
| Format ouvert `.chud` (ZIP) et export/import JSON intégral | Fait | tests aller-retour (événements, flux, images) ; voir `FORMAT.md` |
| Sauvegarde automatique, historique des versions | Fait | |
| Mode portable (`portable.flag` à côté de l'exécutable) | Fait | |
| Profils nommés de réglages | À faire | aujourd'hui : réglages par défaut globaux + réglages propres à chaque macro |

## Interface

| Fonctionnalité | État | Vérifié par / limites |
|---|---|---|
| Interface spatiale, français et anglais | Fait | |
| Visualisation en direct (clavier, souris, manettes), statistiques | Fait | y compris Chud au premier plan : essai réel Windows (F13, Alt, Maj+C+H affichés) |
| Fenêtre toujours atteignable (barre de titre jamais hors écran), adaptée aux petits écrans | Fait | essais sur 3 écrans de tailles et échelles différentes |
| Taille de l'interface (80–150 %, Ctrl+= / Ctrl+- / Ctrl+0) | Fait | tests Vitest des paliers |

## Sécurité et vie privée

| Engagement | État |
|---|---|
| Aucune connexion réseau, aucune télémétrie | Fait |
| Indication claire quand un enregistrement est actif | Fait |
| Exclusion d'applications, protection des données sensibles | Fait (sauf Wayland, voir plus haut) |
| Arrêt d'urgence à tout moment, relâchement des touches et boutons | Fait |
| Aucun contournement des protections du système (UIPI, Wayland, contenu protégé) | Fait — ces limites sont expliquées, jamais contournées |
| Captures d'écran de la recherche gardées en mémoire, jamais écrites ni envoyées | Fait — seules les images choisies par l'utilisateur sont enregistrées dans la macro |
| Indicateur exclu des captures d'écran (pour ne pas fausser les recherches) | Partiel — demandé à Windows (`content_protected`), pas encore vérifié visuellement |

## Prochaines étapes proposées

1. Déclencheurs automatiques (au démarrage, fenêtre activée, horaire).
2. Profils nommés de réglages.
3. OCR sous Linux (moteur local, sans réseau).
4. Transfert binaire des événements vers l'interface pour les très longues prises.
5. Portage macOS.
